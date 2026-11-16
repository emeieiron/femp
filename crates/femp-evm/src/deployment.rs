//! Runs a sequence of transactions against a module's contract in an
//! in-process EVM: deploys the contract with its constructor's arguments,
//! then sends each message, each with the block and transaction context it
//! names.

use std::fs;
use std::path::Path;

use femp_core::artifact::of_kind;
use femp_core::platform::Context;
use femp_core::project::Module;
use femp_core::property::{Delivery, Outcome, Transaction, Transactions, Value};
use femp_core::{Error, Result};
use revm::bytecode::Bytecode;
use revm::context::TxEnv;
use revm::context::result::{ExecutionResult, Output};
use revm::database::InMemoryDB;
use revm::handler::ExecuteCommitEvm;
use revm::primitives::{Address, B256, Bytes, TxKind, U256};
use revm::state::AccountInfo;

use crate::build::{self, BYTECODE, contract_of};
use crate::harness::exec::{Evm, GAS_LIMIT, decode_hex, evm, halt};

/// The account transactions come from unless they name a caller.
const SENDER: u8 = 0xfe;

pub fn run(
    cx: &Context<'_>,
    module: &Module,
    transactions: &Transactions,
    out_dir: &Path,
) -> Result<Vec<Delivery>> {
    let built = build::build(cx, module, out_dir)?;
    let artifact = of_kind(&built, BYTECODE)
        .find(|artifact| contract_of(artifact) == transactions.contract)
        .ok_or_else(|| {
            Error::Artifact(format!(
                "the build of `{}` has no contract `{}`",
                module.name, transactions.contract
            ))
        })?;
    let hex = fs::read_to_string(&artifact.path)
        .map_err(|source| Error::io("read", &artifact.path, source))?;
    let mut data = decode_hex(&hex)?;
    data.extend(words(&transactions.deploy.args));

    let mut evm = evm(InMemoryDB::default());
    let mut deliveries = Vec::new();
    let result = send(&mut evm, &transactions.deploy, TxKind::Create, data)?;
    let address = match result {
        ExecutionResult::Success {
            output: Output::Create(_, Some(address)),
            ..
        } => address,
        other => {
            deliveries.push(Delivery::Rejected(rejection(other)));
            return Ok(deliveries);
        }
    };
    deliveries.push(Delivery::Accepted);
    for call in &transactions.calls {
        let mut data = call
            .selector
            .map(|selector| selector.to_be_bytes().to_vec())
            .unwrap_or_default();
        data.extend(words(&call.args));
        let delivery = match send(&mut evm, call, TxKind::Call(address), data)? {
            ExecutionResult::Success { .. } => Delivery::Accepted,
            other => Delivery::Rejected(rejection(other)),
        };
        deliveries.push(delivery);
    }
    Ok(deliveries)
}

/// The arguments as ABI words, one per primitive.
fn words(args: &[Value]) -> Vec<u8> {
    args.iter()
        .flat_map(|arg| arg.to_word().to_be_bytes::<32>())
        .collect()
}

fn rejection(result: ExecutionResult) -> Outcome {
    match result {
        ExecutionResult::Revert { output, .. } => Outcome::from_revert(&output),
        ExecutionResult::Halt { reason, .. } => Outcome::Trapped(halt(reason)),
        ExecutionResult::Success { .. } => Outcome::Trapped(femp_core::property::Trap::Crashed(
            "the deployment created no contract".to_string(),
        )),
    }
}

/// Sends one transaction in the block and with the sender its context
/// names, after funding the sender with what it pays.
fn send(
    evm: &mut Evm,
    transaction: &Transaction,
    kind: TxKind,
    data: Vec<u8>,
) -> Result<ExecutionResult> {
    let mut caller = Address::with_last_byte(SENDER);
    let mut value = U256::ZERO;
    let mut gas_price = 0u128;
    for (name, given) in &transaction.context {
        let word = U256::from_limbs(given.bits().into_limbs());
        let small = |what: &str| -> Result<u64> {
            u64::try_from(word).map_err(|_| {
                Error::Toolchain(format!("{what} = {word} is beyond what the EVM accepts"))
            })
        };
        let block = &mut evm.ctx.block;
        match name.as_str() {
            "ctx.timestamp" => block.timestamp = word,
            "ctx.block_number" => block.number = word,
            "ctx.coinbase" => block.beneficiary = address(word, name)?,
            "ctx.prevrandao" => block.prevrandao = Some(B256::from(word.to_be_bytes::<32>())),
            "ctx.gaslimit" => block.gas_limit = small(name)?,
            "ctx.basefee" => {
                block.basefee = small(name)?;
                gas_price = gas_price.max(u128::from(block.basefee));
            }
            "ctx.gasprice" => gas_price = gas_price.max(u128::from(small(name)?)),
            "ctx.chainid" => evm.ctx.cfg.chain_id = small(name)?,
            "ctx.caller" => caller = address(word, name)?,
            "ctx.value" => value = word,
            other => {
                return Err(Error::Toolchain(format!(
                    "the EVM decides `{other}`, so a sequence that chooses it cannot be replayed"
                )));
            }
        }
    }
    // Enough for the value and for any gas at any price the block allows.
    let balance =
        value.saturating_add(U256::from(gas_price) * U256::from(GAS_LIMIT) * U256::from(2u8));
    evm.ctx.journaled_state.database.insert_account_info(
        caller,
        AccountInfo::new(balance, 0, Default::default(), Bytecode::default()),
    );
    let tx = TxEnv::builder()
        .caller(caller)
        .gas_limit(GAS_LIMIT)
        .gas_price(gas_price)
        .value(value)
        .kind(kind)
        .data(Bytes::from(data))
        .build()
        .map_err(|error| Error::Toolchain(format!("invalid EVM transaction: {error:?}")))?;
    evm.transact_commit(tx)
        .map_err(|error| Error::Toolchain(format!("the EVM rejected a transaction: {error}")))
}

fn address(word: U256, name: &str) -> Result<Address> {
    if word >> 160 != U256::ZERO {
        return Err(Error::Toolchain(format!(
            "{name} = {word} is not an address"
        )));
    }
    Ok(Address::from_word(B256::from(word.to_be_bytes::<32>())))
}
