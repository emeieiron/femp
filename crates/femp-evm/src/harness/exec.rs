//! Runs harness messages in an in-process EVM. The harness is deployed
//! once; properties run as calls whose state changes are discarded, and each
//! machine run starts from a copy of the freshly deployed state.

use femp_core::property::{
    Executor, Halt, Outcome, Record, Sequence, StepStatus, Suite, Support, Trace, Trap, Ty, Value,
};
use femp_core::{Error, Result};
use revm::bytecode::Bytecode;
use revm::context::result::{ExecutionResult, HaltReason, Output};
use revm::context::{CfgEnv, Context, TxEnv};
use revm::database::InMemoryDB;
use revm::handler::{ExecuteCommitEvm, ExecuteEvm, MainBuilder, MainContext, MainnetContext};

use revm::primitives::hardfork::SpecId;
use revm::primitives::{Address, Bytes, TxKind, U256, keccak256};
use revm::state::AccountInfo;

use super::render::{self, Message};

pub(crate) const GAS_LIMIT: u64 = 1_000_000_000;
const CODE_SIZE_LIMIT: usize = 1024 * 1024;

pub struct EvmExecutor {
    /// The state right after the harness was deployed.
    deployed: InMemoryDB,
    harness: Address,
    suite: Suite,
}

pub(crate) type Evm = revm::handler::MainnetEvm<MainnetContext<InMemoryDB>>;

pub(crate) fn configure(cfg: &mut CfgEnv) {
    // Fe emits code for the Osaka revision.
    cfg.spec = SpecId::OSAKA;
    cfg.tx_gas_limit_cap = Some(u64::MAX);
    cfg.limit_contract_code_size = Some(CODE_SIZE_LIMIT);
    cfg.limit_contract_initcode_size = Some(2 * CODE_SIZE_LIMIT);
    cfg.disable_nonce_check = true;
}

pub(crate) fn evm(db: InMemoryDB) -> Evm {
    Context::mainnet()
        .with_db(db)
        .modify_cfg_chained(configure)
        .modify_block_chained(|block| {
            block.timestamp = U256::ZERO;
            block.number = U256::from(1u8);
            block.basefee = 0;
            block.gas_limit = u64::MAX;
        })
        .build_mainnet()
}

fn caller() -> Address {
    Address::with_last_byte(0xfe)
}

fn transaction(kind: TxKind, data: Vec<u8>) -> Result<TxEnv> {
    TxEnv::builder()
        .caller(caller())
        .gas_limit(GAS_LIMIT)
        .gas_price(0)
        .kind(kind)
        .data(Bytes::from(data))
        .build()
        .map_err(|error| Error::Toolchain(format!("invalid EVM transaction: {error:?}")))
}

impl EvmExecutor {
    /// Deploys the harness from its creation bytecode (hex).
    pub fn deploy(init_code_hex: &str, suite: Suite) -> Result<Self> {
        let init_code = decode_hex(init_code_hex)?;
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            caller(),
            AccountInfo::new(
                U256::from(u64::MAX),
                0,
                Default::default(),
                Bytecode::default(),
            ),
        );
        let mut evm = evm(db);
        let result = evm
            .transact_commit(transaction(TxKind::Create, init_code)?)
            .map_err(|error| {
                Error::Toolchain(format!("could not deploy the EVM harness: {error}"))
            })?;
        let harness = match result {
            ExecutionResult::Success {
                output: Output::Create(_, Some(address)),
                ..
            } => address,
            other => {
                return Err(Error::Toolchain(format!(
                    "deploying the EVM harness failed: {other:?}"
                )));
            }
        };
        Ok(Self {
            deployed: evm.ctx.journaled_state.database,
            harness,
            suite,
        })
    }
}

/// The calldata of a message: its selector, then one word per argument.
pub fn calldata(message: &Message, args: &[Value]) -> Vec<u8> {
    let mut data = keccak256(message.signature().as_bytes())[..4].to_vec();
    for arg in args {
        data.extend_from_slice(&arg.to_word().to_be_bytes::<32>());
    }
    data
}

/// Normalizes the result of a message call.
pub fn outcome(result: ExecutionResult, ty: Ty) -> Outcome {
    match result {
        ExecutionResult::Success { output, .. } => {
            let data = output.data();
            let word = data
                .get(..32)
                .map(|bytes| femp_core::property::U256::from_be_slice(bytes));
            match word.and_then(|word| Value::from_word(ty, word)) {
                Some(value) if data.len() == 32 => Outcome::Returned(value),
                _ => Outcome::Trapped(Trap::Crashed(format!(
                    "the harness returned 0x{} for a `{ty}`",
                    femp_core::property::outcome::hex(data)
                ))),
            }
        }
        ExecutionResult::Revert { output, .. } => Outcome::from_revert(&output),
        ExecutionResult::Halt { reason, .. } => Outcome::Trapped(halt(reason)),
    }
}

pub(crate) fn halt(reason: HaltReason) -> Trap {
    match reason {
        HaltReason::OutOfGas(_) => Trap::OutOfGas,
        other => Trap::Halted(format!("{other:?}")),
    }
}

struct Run {
    evm: Evm,
    harness: Address,
}

impl Run {
    fn call(&mut self, message: &Message, args: &[Value]) -> Result<Outcome> {
        let tx = transaction(TxKind::Call(self.harness), calldata(message, args))?;
        let result = self
            .evm
            .transact(tx)
            .map_err(|error| Error::Toolchain(format!("the EVM rejected a call: {error}")))?;
        Ok(outcome(result.result, message.result))
    }

    fn commit(&mut self, message: &Message, args: &[Value]) -> Result<Outcome> {
        let tx = transaction(TxKind::Call(self.harness), calldata(message, args))?;
        let result = self.evm.transact_commit(tx).map_err(|error| {
            Error::Toolchain(format!("the EVM rejected a transaction: {error}"))
        })?;
        Ok(outcome(result, message.result))
    }

    fn advance(&mut self, seconds: u64) {
        let block = &mut self.evm.ctx.block;
        block.timestamp = block.timestamp.saturating_add(U256::from(seconds));
        block.number = block.number.saturating_add(U256::from(1u8));
    }
}

impl EvmExecutor {
    fn fresh(&self) -> Run {
        Run {
            evm: evm(self.deployed.clone()),
            harness: self.harness,
        }
    }

    // The invariants and observations after a step, or the outcome that
    // stopped one of them.
    fn record(
        &self,
        run: &mut Run,
        machine: usize,
        status: StepStatus,
    ) -> Result<std::result::Result<Record, Outcome>> {
        let definition = &self.suite.machines[machine];
        let mut invariants = Vec::new();
        for index in 0..definition.invariants.len() {
            match run.call(&render::invariant_message(&self.suite, machine, index), &[])? {
                Outcome::Returned(value) => invariants.push(value.as_bool().unwrap_or(false)),
                other => return Ok(Err(other)),
            }
        }
        let mut observations = Vec::new();
        for index in 0..definition.observations.len() {
            match run.call(
                &render::observation_message(&self.suite, machine, index),
                &[],
            )? {
                Outcome::Returned(value) => observations.push(value),
                other => return Ok(Err(other)),
            }
        }
        Ok(Ok(Record {
            status,
            invariants,
            observations,
        }))
    }
}

impl Executor for EvmExecutor {
    fn supports_property(&self, _index: usize) -> Support {
        Support::Yes
    }

    fn supports_machine(&self, index: usize) -> Support {
        match render::unsupported_effect(&self.suite.machines[index]) {
            Some(effect) => Support::No(format!(
                "its methods use the `{effect}` effect, which the EVM harness does not provide"
            )),
            None => Support::Yes,
        }
    }

    fn call(&mut self, property: usize, cases: &[Vec<Value>]) -> Result<Vec<Outcome>> {
        let message = render::property_message(&self.suite, property);
        let mut run = self.fresh();
        cases.iter().map(|args| run.call(&message, args)).collect()
    }

    fn run(&mut self, machine: usize, sequence: &Sequence) -> Result<Trace> {
        let mut run = self.fresh();
        let mut trace = Trace::default();
        let halted = |trace: &mut Trace, outcome| {
            trace.halt = Some(Halt {
                step: trace.records.len(),
                outcome,
            });
        };

        match run.commit(&render::init_message(&self.suite, machine), &sequence.init)? {
            Outcome::Returned(_) => {}
            other => {
                halted(&mut trace, other);
                return Ok(trace);
            }
        }
        match self.record(&mut run, machine, StepStatus::Done)? {
            Ok(record) => trace.records.push(record),
            Err(outcome) => {
                halted(&mut trace, outcome);
                return Ok(trace);
            }
        }
        for step in &sequence.steps {
            run.advance(step.advance);
            let message = render::command_message(&self.suite, machine, step.command);
            let status = match run.commit(&message, &step.args)? {
                Outcome::Returned(_) => StepStatus::Done,
                // A custom error rejects the command; its effects are undone.
                Outcome::Reverted(data) => StepStatus::Rejected(data),
                other => {
                    halted(&mut trace, other);
                    return Ok(trace);
                }
            };
            match self.record(&mut run, machine, status)? {
                Ok(record) => trace.records.push(record),
                Err(outcome) => {
                    halted(&mut trace, outcome);
                    return Ok(trace);
                }
            }
        }
        Ok(trace)
    }
}

pub(crate) fn decode_hex(text: &str) -> Result<Vec<u8>> {
    let text = text.trim().trim_start_matches("0x");
    if !text.len().is_multiple_of(2) {
        return Err(Error::Artifact(
            "the harness bytecode has odd length".to_string(),
        ));
    }
    (0..text.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&text[index..index + 2], 16)
                .map_err(|_| Error::Artifact("the harness bytecode is not hex".to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use femp_core::property::{PanicKind, fixtures};
    use revm::context::result::{OutOfGasError, SuccessReason};

    use super::*;

    fn success(data: Vec<u8>) -> ExecutionResult {
        ExecutionResult::Success {
            reason: SuccessReason::Return,
            gas_used: 0,
            gas_refunded: 0,
            logs: Vec::new(),
            output: Output::Call(Bytes::from(data)),
        }
    }

    #[test]
    fn encodes_calldata_with_selectors_and_words() {
        let suite = fixtures::suite();
        let message = render::property_message(&suite, 1);
        let minus_two = Value::parse(Ty::Int(64), "-2").unwrap();
        let data = calldata(&message, &[minus_two]);
        assert_eq!(data.len(), 36);
        assert_eq!(&data[..4], &keccak256(b"femp_entry_1(int64)")[..4]);
        assert!(data[4..35].iter().all(|byte| *byte == 0xff));
        assert_eq!(data[35], 0xfe);
    }

    #[test]
    fn normalizes_results() {
        let mut word = vec![0u8; 32];
        word[31] = 1;
        assert_eq!(
            outcome(success(word), Ty::Bool),
            Outcome::Returned(Value::boolean(true))
        );
        assert!(matches!(
            outcome(success(vec![0u8; 31]), Ty::Bool),
            Outcome::Trapped(Trap::Crashed(_))
        ));
        let revert = ExecutionResult::Revert {
            gas_used: 0,
            output: Bytes::new(),
        };
        assert_eq!(
            outcome(revert, Ty::Bool),
            Outcome::Panicked(PanicKind::Explicit)
        );
        let halted = ExecutionResult::Halt {
            reason: HaltReason::OutOfGas(OutOfGasError::Basic),
            gas_used: 0,
        };
        assert_eq!(outcome(halted, Ty::Bool), Outcome::Trapped(Trap::OutOfGas));
    }

    // Built from `tests/fixtures/harness.fe` against `tests/fixtures/demo_core`:
    //   fe build --backend sonatina --emit bytecode <harness ingot>
    const HARNESS: &str = include_str!("../../tests/fixtures/FempPropertyHarness.bin");

    fn value(ty: Ty, text: &str) -> Value {
        Value::parse(ty, text).unwrap()
    }

    #[test]
    fn runs_properties_in_the_deployed_harness() {
        let mut executor = EvmExecutor::deploy(HARNESS, fixtures::suite()).unwrap();
        let outcomes = executor
            .call(
                1,
                &[
                    vec![value(Ty::Int(64), "-7")],
                    vec![value(Ty::Int(64), "-9223372036854775808")],
                ],
            )
            .unwrap();
        assert_eq!(
            outcomes,
            [
                Outcome::Returned(value(Ty::Int(64), "7")),
                Outcome::Panicked(PanicKind::Overflow)
            ]
        );
        let wide = executor
            .call(2, &[vec![value(Ty::Uint(256), "300")]])
            .unwrap();
        assert_eq!(wide, [Outcome::Returned(value(Ty::Uint(256), "100"))]);
    }

    #[test]
    fn runs_machines_from_a_fresh_state() {
        use femp_core::property::Step;

        let mut executor = EvmExecutor::deploy(HARNESS, fixtures::suite()).unwrap();
        let sequence = Sequence {
            init: vec![value(Ty::Uint(8), "5")],
            steps: vec![
                Step {
                    command: 0,
                    args: vec![value(Ty::Uint(64), "10")],
                    advance: 3,
                },
                Step {
                    command: 1,
                    args: vec![value(Ty::Int(8), "-128")],
                    advance: 0,
                },
            ],
        };
        for _ in 0..2 {
            let trace = executor.run(0, &sequence).unwrap();
            let values: Vec<String> = trace
                .records
                .iter()
                .map(|record| record.observations[0].to_string())
                .collect();
            assert_eq!(values, ["5", "15"]);
            assert_eq!(
                trace.halt,
                Some(Halt {
                    step: 2,
                    outcome: Outcome::Panicked(PanicKind::Overflow)
                })
            );
        }
    }
}
