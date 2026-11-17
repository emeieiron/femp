//! Contracts under property testing. A contract's invariants, and each of
//! its message claims, become a machine over the contract's messages whose
//! runs deploy the compiled contract and send it generated transactions;
//! the storage after each one is decoded and the claims are evaluated on
//! it. A message that reverts leaves the storage as it was, as it does on
//! a chain. Block clocks only move forward: the timestamp advances by the
//! run's clock before each message, and the block number by one.

use std::sync::Arc;

use femp_compiler::{LoweredContract, Session, Target};
use femp_core::property::{
    Delivery, Deployer, Executor, Halt, Machine, Method, Outcome, Param, Record, Sequence, Step,
    StepStatus, Suite, Support, Trace, Ty, U256, Value,
};
use femp_core::{Error, Result};
use femp_spec::{Claims, MessageKind, StorageClaim};
use femp_vir::FuncId;
use femp_vir::eval::Val;

use crate::contracts::{
    self, ContractSide, Reads, Stop, contract_machine, contract_root, instances,
};

const TIMESTAMP: &str = "ctx.timestamp";
const BLOCK_NUMBER: &str = "ctx.block_number";
/// Context fields the EVM decides, which a run cannot choose.
const DECIDED: [&str; 4] = [
    "ctx.origin",
    "ctx.address",
    "ctx.selfbalance",
    "ctx.blobbasefee",
];
/// Addresses come from a few accounts, so that messages meet: the one an
/// account receives with is the one it later sends with.
const ACCOUNTS: &str = "0..=3";

/// What a run checks on the contract's storage.
enum Check {
    /// The invariants, after every transaction.
    Invariants,
    /// A message claim, at every step that sends the message.
    Message {
        command: usize,
        func: FuncId,
        after: bool,
        params: Vec<Param>,
        kind: MessageKind,
    },
}

struct Tested {
    /// The machine with the arguments the contract's handlers take, which
    /// runs draw without the clock's.
    handlers: Machine,
    selectors: Vec<Option<u32>>,
    side: Option<Arc<ContractSide>>,
    check: Check,
    support: Support,
}

/// A module's contracts as machines: the suite the engine runs, and what
/// each machine's runs check.
pub struct Contracts {
    pub suite: Suite,
    tested: Vec<Tested>,
}

impl Contracts {
    /// Lowers the contracts the module's claims are about, or gives `None`
    /// when it makes no claims about contracts.
    pub fn lower(
        module: &femp_core::project::Module,
        claims: &Claims,
        fe_target: &str,
    ) -> anyhow::Result<Option<Self>> {
        if claims.storage.is_empty() {
            return Ok(None);
        }
        let target = Target::parse(fe_target)
            .ok_or_else(|| anyhow::anyhow!("the compiler has no target `{fe_target}`"))?;
        let session = Session::open(&module.dir, target)
            .map_err(|error| anyhow::anyhow!("cannot load `{}`: {error}", module.name))?;
        let roots: Vec<_> = claims.storage.iter().map(contract_root).collect();
        let lowered = session.lower_with(&[], &roots);
        let program = Arc::new(lowered.program.clone());
        let mut contracts = Self {
            suite: Suite {
                module: claims.suite.module.clone(),
                ingot: claims.suite.ingot.clone(),
                properties: Vec::new(),
                machines: Vec::new(),
            },
            tested: Vec::new(),
        };
        for (claim, contract) in claims.storage.iter().zip(&lowered.contracts) {
            let contract = match contract {
                Ok(contract) => contract,
                Err(error) => {
                    contracts.unsupported(claim, &error.to_string());
                    continue;
                }
            };
            let side = Arc::new(ContractSide::new(
                &session,
                program.clone(),
                claim,
                contract,
            ));
            contracts.add(claim, contract, &side);
        }
        Ok(Some(contracts))
    }

    pub fn executor(&self, deployer: Box<dyn Deployer>) -> ContractExecutor<'_> {
        ContractExecutor {
            contracts: self,
            deployer,
        }
    }

    fn add(&mut self, claim: &StorageClaim, contract: &LoweredContract, side: &Arc<ContractSide>) {
        let (handlers, _) = contract_machine(claim, contract);
        let selectors: Vec<Option<u32>> = contract.arms.iter().map(|arm| arm.selector).collect();
        let mut support = decided(&handlers);
        if let (Support::Yes, Err(reason)) = (&support, &side.layout) {
            support = Support::No(format!("its storage cannot be read: {reason}"));
        }
        if claim.claims_invariants && !claim.invariants.is_empty() {
            let mut drawn = drawn(&handlers);
            drawn.invariants = handlers.invariants.clone();
            self.push(
                drawn,
                Tested {
                    handlers: handlers.clone(),
                    selectors: selectors.clone(),
                    side: Some(side.clone()),
                    check: Check::Invariants,
                    support: support.clone(),
                },
            );
        }
        for (message, lowered) in claim.messages.iter().zip(&contract.messages) {
            let mut drawn = drawn(&handlers);
            drawn.name = message.name.clone();
            drawn.path = message.path.clone();
            drawn.location = message.location.clone();
            drawn.invariants = vec![Method {
                name: message.name.clone(),
                params: Vec::new(),
                effects: Vec::new(),
                location: message.location.clone(),
            }];
            let (check, support) = match lowered {
                Ok(lowered) => {
                    let support = match (&support, message.kind) {
                        (Support::Yes, MessageKind::Reverts) if !lowered.params.is_empty() => {
                            Support::No(format!(
                                "`{}` ranges over values the message does not name",
                                message.name
                            ))
                        }
                        _ => support.clone(),
                    };
                    let check = Check::Message {
                        command: lowered.arm,
                        func: lowered.func,
                        after: lowered.after,
                        params: lowered.params.clone(),
                        kind: message.kind,
                    };
                    (check, support)
                }
                Err(error) => (Check::Invariants, Support::No(error.to_string())),
            };
            self.push(
                drawn,
                Tested {
                    handlers: handlers.clone(),
                    selectors: selectors.clone(),
                    side: Some(side.clone()),
                    check,
                    support,
                },
            );
        }
    }

    /// A contract that does not lower is listed, and skipped with the
    /// reason, under its claims.
    fn unsupported(&mut self, claim: &StorageClaim, reason: &str) {
        let names = claim
            .claims_invariants
            .then(|| {
                (
                    claim.name.clone(),
                    claim.path.clone(),
                    claim.location.clone(),
                )
            })
            .into_iter()
            .chain(claim.messages.iter().map(|message| {
                (
                    message.name.clone(),
                    message.path.clone(),
                    message.location.clone(),
                )
            }));
        for (name, path, location) in names {
            let method = |name: &str| Method {
                name: name.to_string(),
                params: Vec::new(),
                effects: Vec::new(),
                location: location.clone(),
            };
            let machine = Machine {
                name: name.clone(),
                path,
                init: method("init"),
                commands: Vec::new(),
                invariants: Vec::new(),
                observations: Vec::new(),
                steps: "1..=16".to_string(),
                runs: None,
                clock: None,
                location: location.clone(),
            };
            self.push(
                machine.clone(),
                Tested {
                    handlers: machine,
                    selectors: Vec::new(),
                    side: None,
                    check: Check::Invariants,
                    support: Support::No(reason.to_string()),
                },
            );
        }
    }

    fn push(&mut self, drawn: Machine, tested: Tested) {
        self.suite.machines.push(drawn);
        self.tested.push(tested);
    }
}

/// Why no run can choose the context a contract's handlers read.
fn decided(machine: &Machine) -> Support {
    let methods = std::iter::once(&machine.init).chain(machine.commands.iter().map(|c| &c.method));
    for method in methods {
        if let Some(param) = method
            .params
            .iter()
            .find(|param| DECIDED.contains(&param.name.as_str()))
        {
            return Support::No(format!(
                "`{}` reads `{}`, which the EVM decides",
                method.name, param.name
            ));
        }
    }
    Support::Yes
}

/// The machine runs draw: messages without the clock's context, whose
/// steps advance the clock instead, and context from realistic ranges.
fn drawn(handlers: &Machine) -> Machine {
    let mut machine = handlers.clone();
    let mut clocked = false;
    for command in &mut machine.commands {
        command.method.params.retain(|param| {
            let derived = param.name == TIMESTAMP || param.name == BLOCK_NUMBER;
            clocked |= param.name == TIMESTAMP;
            !derived
        });
    }
    let methods = std::iter::once(&mut machine.init)
        .chain(machine.commands.iter_mut().map(|c| &mut c.method));
    for method in methods {
        for param in &mut method.params {
            if let Some(range) = range(param) {
                param.generator = range.to_string();
            }
        }
    }
    machine.clock = clocked.then(|| "any".to_string());
    machine
}

/// The range a context field or an address is drawn from, where the
/// whole of its type is not what a chain gives.
fn range(param: &Param) -> Option<&'static str> {
    match param.name.as_str() {
        "ctx.value" => Some("0..=1000"),
        "ctx.gaslimit" => Some("1000000000..=18446744073709551615"),
        "ctx.basefee" | "ctx.gasprice" => Some("0..=1000"),
        "ctx.chainid" => Some("0..=10"),
        _ if param.ty == Ty::Uint(160) => Some(ACCOUNTS),
        _ => None,
    }
}

/// Runs a module's contracts on one target.
pub struct ContractExecutor<'a> {
    contracts: &'a Contracts,
    deployer: Box<dyn Deployer>,
}

impl Executor for ContractExecutor<'_> {
    fn supports_property(&self, _index: usize) -> Support {
        Support::No("contracts have no properties".to_string())
    }

    fn supports_machine(&self, index: usize) -> Support {
        self.contracts.tested[index].support.clone()
    }

    fn call(
        &mut self,
        _property: usize,
        _cases: &[Vec<Value>],
    ) -> Result<Vec<femp_core::property::Outcome>> {
        Err(Error::Artifact("contracts have no properties".to_string()))
    }

    fn run(&mut self, machine: usize, sequence: &Sequence) -> Result<Trace> {
        let tested = &self.contracts.tested[machine];
        let side = tested
            .side
            .as_deref()
            .ok_or_else(|| Error::Artifact("the contract did not lower".to_string()))?;
        let full = clocked(&tested.handlers, sequence);
        let named: Vec<Value> = full
            .init
            .iter()
            .chain(full.steps.iter().flat_map(|step| &step.args))
            .copied()
            .collect();
        let reads = side.reads(&named);
        let mut transactions = contracts::transactions(&tested.handlers, &tested.selectors, &full);
        transactions.reads = reads.slots.clone();
        let run = self.deployer.run(&transactions)?;
        let judge = Judge {
            side,
            reads: &reads,
            named: &named,
            check: &tested.check,
        };
        judge.trace(&full, &run)
    }
}

/// A drawn sequence with the clock's context put back: the deployment's
/// own, or zero and block 1, then the steps' advances and one block each.
fn clocked(handlers: &Machine, sequence: &Sequence) -> Sequence {
    let drawn = |name: &str| {
        handlers
            .init
            .params
            .iter()
            .position(|param| param.name == name)
            .and_then(|index| sequence.init.get(index))
            .map(Value::bits)
    };
    let mut timestamp = drawn(TIMESTAMP).unwrap_or(U256::ZERO);
    let mut number = drawn(BLOCK_NUMBER).unwrap_or(U256::from(1u8));
    let steps = sequence
        .steps
        .iter()
        .map(|step| {
            timestamp = timestamp.saturating_add(U256::from(step.advance));
            number = number.saturating_add(U256::from(1u8));
            let mut drawn = step.args.iter();
            let args = handlers.commands[step.command]
                .method
                .params
                .iter()
                .filter_map(|param| match param.name.as_str() {
                    TIMESTAMP => Some(Value::from_bits(param.ty, timestamp)),
                    BLOCK_NUMBER => Some(Value::from_bits(param.ty, number)),
                    _ => drawn.next().copied(),
                })
                .collect();
            Step {
                command: step.command,
                args,
                advance: step.advance,
            }
        })
        .collect();
    Sequence {
        init: sequence.init.clone(),
        steps,
    }
}

/// Judges one run on the storage after each of its transactions.
struct Judge<'a> {
    side: &'a ContractSide,
    reads: &'a Reads,
    named: &'a [Value],
    check: &'a Check,
}

impl Judge<'_> {
    fn trace(&self, full: &Sequence, run: &femp_core::property::ContractRun) -> Result<Trace> {
        let mut trace = Trace::default();
        // A constructor that reverts deploys nothing to check.
        if run.deliveries.first() != Some(&Delivery::Accepted) {
            return Ok(trace);
        }
        let mut before = self.state(run, 0)?;
        match self.holds(&before, None) {
            Ok(holds) => trace.records.push(record(StepStatus::Done, holds)),
            Err(outcome) => return halted(trace, 0, outcome),
        }
        for (index, step) in full.steps.iter().enumerate() {
            let at = index + 1;
            let after = self.state(run, at)?;
            let status = match &run.deliveries[at] {
                Delivery::Accepted => StepStatus::Done,
                Delivery::Rejected(Outcome::Reverted(data)) => StepStatus::Rejected(data.clone()),
                Delivery::Rejected(_) => StepStatus::Rejected(Vec::new()),
            };
            let sent = Sent {
                step,
                before: &before,
                accepted: status == StepStatus::Done,
            };
            match self.holds(&after, Some(sent)) {
                Ok(holds) => trace.records.push(record(status, holds)),
                Err(outcome) => return halted(trace, at, outcome),
            }
            before = after;
        }
        Ok(trace)
    }

    fn state(&self, run: &femp_core::property::ContractRun, at: usize) -> Result<Val> {
        let words = run
            .storage
            .get(at)
            .ok_or_else(|| Error::Artifact("the contract's storage was not read".to_string()))?;
        self.side.decode(self.reads, words).map_err(Error::Artifact)
    }

    /// Whether each claim of the run holds on `state`, which `sent` led to
    /// from the state before it.
    fn holds(&self, state: &Val, sent: Option<Sent<'_>>) -> std::result::Result<Vec<bool>, Stop> {
        match self.check {
            Check::Invariants => self
                .side
                .invariants
                .iter()
                .map(|invariant| {
                    for instance in instances(&invariant.params, self.named) {
                        let mut args = vec![state.clone()];
                        args.extend(instance);
                        if !self.side.holds(invariant.func, args)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                })
                .collect(),
            Check::Message {
                command,
                func,
                after,
                params,
                kind,
            } => {
                let Some(sent) = sent.filter(|sent| sent.step.command == *command) else {
                    return Ok(vec![true]);
                };
                let mut args = vec![sent.before.clone()];
                if *after {
                    args.push(state.clone());
                }
                args.extend(sent.step.args.iter().map(Val::of));
                let holds = match kind {
                    MessageKind::Ensures if !sent.accepted => true,
                    MessageKind::Ensures => {
                        let mut holds = true;
                        for instance in instances(params, self.named) {
                            let mut args = args.clone();
                            args.extend(instance);
                            if !self.side.holds(*func, args)? {
                                holds = false;
                                break;
                            }
                        }
                        holds
                    }
                    // It reverts exactly where the claim holds.
                    MessageKind::Reverts => self.side.holds(*func, args)? != sent.accepted,
                };
                Ok(vec![holds])
            }
        }
    }
}

/// A message a step sent, and the storage it started from.
#[derive(Clone, Copy)]
struct Sent<'a> {
    step: &'a Step,
    before: &'a Val,
    accepted: bool,
}

fn record(status: StepStatus, invariants: Vec<bool>) -> Record {
    Record {
        status,
        invariants,
        observations: Vec::new(),
    }
}

/// A claim that panics on the storage ends the run, as a panicking
/// invariant ends a machine's; one that cannot be evaluated is an error.
fn halted(mut trace: Trace, step: usize, stop: Stop) -> Result<Trace> {
    match stop {
        Stop::Panic(kind) => {
            trace.halt = Some(Halt {
                step,
                outcome: Outcome::Panicked(kind),
            });
            Ok(trace)
        }
        Stop::Unsupported(reason) => Err(Error::Artifact(reason)),
    }
}

#[cfg(test)]
mod tests {
    use femp_core::property::{Command, Location};

    use super::*;

    fn param(name: &str, ty: Ty) -> Param {
        Param {
            name: name.to_string(),
            label: None,
            ty,
            generator: "any".to_string(),
        }
    }

    fn method(name: &str, params: Vec<Param>) -> Method {
        Method {
            name: name.to_string(),
            params,
            effects: Vec::new(),
            location: Location {
                file: "lib.fe".into(),
                line: 1,
            },
        }
    }

    fn handlers() -> Machine {
        let location = Location {
            file: "lib.fe".into(),
            line: 1,
        };
        Machine {
            name: "Vault".to_string(),
            path: Vec::new(),
            init: method("init", vec![param(TIMESTAMP, Ty::Uint(256))]),
            commands: vec![Command {
                method: method(
                    "Pay",
                    vec![
                        param(TIMESTAMP, Ty::Uint(256)),
                        param("ctx.caller", Ty::Uint(160)),
                        param("amount", Ty::Uint(64)),
                    ],
                ),
                weight: 1,
            }],
            invariants: Vec::new(),
            observations: Vec::new(),
            steps: "1..=16".to_string(),
            runs: None,
            clock: None,
            location,
        }
    }

    fn value(ty: Ty, number: u128) -> Value {
        Value::from_u128(ty, number).unwrap()
    }

    #[test]
    fn runs_draw_messages_without_the_clock() {
        let drawn = drawn(&handlers());
        let pay = &drawn.commands[0].method.params;
        let names: Vec<&str> = pay.iter().map(|param| param.name.as_str()).collect();
        assert_eq!(names, ["ctx.caller", "amount"]);
        assert_eq!(pay[0].generator, ACCOUNTS);
        assert_eq!(pay[1].generator, "any");
        // The deployment keeps its own time, which the steps start from.
        assert_eq!(drawn.init.params.len(), 1);
        assert_eq!(drawn.clock.as_deref(), Some("any"));
    }

    #[test]
    fn steps_take_the_time_the_clock_has_reached() {
        let step = |advance: u64, amount: u128| Step {
            command: 0,
            args: vec![value(Ty::Uint(160), 1), value(Ty::Uint(64), amount)],
            advance,
        };
        let sequence = Sequence {
            init: vec![value(Ty::Uint(256), 100)],
            steps: vec![step(5, 7), step(0, 8), step(20, 9)],
        };
        let full = clocked(&handlers(), &sequence);
        let times: Vec<Value> = full.steps.iter().map(|step| step.args[0]).collect();
        let at = |seconds| value(Ty::Uint(256), seconds);
        assert_eq!(times, [at(105), at(105), at(125)]);
        assert_eq!(full.steps[2].args[1..], sequence.steps[2].args[..]);
    }
}
