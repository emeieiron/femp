//! A module's contracts as both commands see them: lowered with their
//! claims, proved or tested as machines over their messages, and judged on
//! the compiled contract's storage. A run reads the slots of the contract's
//! fields and of the map entries the run names; decoded, the storage is the
//! state the claims take, and the interpreter evaluates them on it.

use std::sync::Arc;

use femp_compiler::{
    ContractRoot, Handler, LoweredContract, MessageRoot, Root, Session, StorageLayout, Word,
};
use femp_core::property::{
    Machine, Method, Param, Sequence, Transaction, Transactions, U256, Value,
};
use femp_prove::obligation::machine::{self, MachineFns};
use femp_spec::{MessageKind, StorageClaim};
use femp_vir::eval::{self, Halt, Val};
use femp_vir::{FuncId, Program, Type};

/// The most instances of a claim's quantified values a run checks.
const MAX_INSTANCES: usize = 64;

pub fn contract_root(claim: &StorageClaim) -> ContractRoot {
    ContractRoot {
        path: claim.path.clone(),
        name: claim.name.clone(),
        invariants: claim
            .invariants
            .iter()
            .map(|invariant| Root::function(&invariant.path, &invariant.name))
            .collect(),
        messages: claim
            .messages
            .iter()
            .map(|message| MessageRoot {
                root: Root::function(&message.path, &message.name),
                message: message.message.clone(),
                after: message.kind == MessageKind::Ensures,
            })
            .collect(),
    }
}

/// A lowered contract as the machine it is proved and tested as: its
/// constructor, then one command per message.
pub fn contract_machine(claim: &StorageClaim, contract: &LoweredContract) -> (Machine, MachineFns) {
    fn handler(handler: &Handler) -> machine::ContractHandler<'_> {
        machine::ContractHandler {
            name: &handler.name,
            params: &handler.params,
            func: handler.func,
        }
    }
    let arms: Vec<machine::ContractHandler<'_>> = contract.arms.iter().map(handler).collect();
    let invariants: Vec<(&str, &[Param], FuncId)> = claim
        .invariants
        .iter()
        .zip(&contract.invariants)
        .map(|(invariant, lowered)| {
            (
                invariant.name.as_str(),
                lowered.params.as_slice(),
                lowered.func,
            )
        })
        .collect();
    machine::contract_machine(
        &claim.name,
        &claim.path,
        claim.location.clone(),
        handler(&contract.init),
        &arms,
        &invariants,
        contract.reenter,
    )
}

/// A sequence of a contract's machine as transactions: each handler takes
/// its context fields first, which the transaction runs in.
pub fn transactions(
    machine: &Machine,
    selectors: &[Option<u32>],
    sequence: &Sequence,
) -> Transactions {
    let transaction = |method: &Method, selector: Option<u32>, values: &[Value]| {
        let mut transaction = Transaction {
            selector,
            name: method.name.clone(),
            args: Vec::new(),
            context: Vec::new(),
        };
        for (param, value) in method.params.iter().zip(values) {
            if param.name.starts_with("ctx.") {
                transaction.context.push((param.name.clone(), *value));
            } else {
                transaction.args.push(*value);
            }
        }
        transaction
    };
    Transactions {
        contract: machine.name.clone(),
        deploy: transaction(&machine.init, None, &sequence.init),
        calls: sequence
            .steps
            .iter()
            .map(|step| {
                transaction(
                    &machine.commands[step.command].method,
                    selectors.get(step.command).copied().flatten(),
                    &step.args,
                )
            })
            .collect(),
        reads: Vec::new(),
    }
}

/// A claim's function and the values it ranges over after the ones a run
/// gives it.
#[derive(Debug, Clone)]
pub struct ClaimFn {
    pub name: String,
    pub func: FuncId,
    pub params: Vec<Param>,
}

/// A lowered contract with what judging its storage needs.
#[derive(Debug, Clone)]
pub struct ContractSide {
    pub program: Arc<Program>,
    pub state: Type,
    pub layout: Result<StorageLayout, String>,
    pub invariants: Vec<ClaimFn>,
    /// The handlers that call other contracts, whose answers the proof
    /// leaves open.
    pub calls_out: Vec<String>,
}

impl ContractSide {
    pub fn new(
        session: &Session,
        program: Arc<Program>,
        claim: &StorageClaim,
        contract: &LoweredContract,
    ) -> Self {
        Self {
            program,
            state: contract.state.clone(),
            layout: session
                .storage_layout(&claim.path, &claim.name)
                .map_err(|error| error.message),
            invariants: claim
                .invariants
                .iter()
                .zip(&contract.invariants)
                .map(|(invariant, lowered)| ClaimFn {
                    name: invariant.name.clone(),
                    func: lowered.func,
                    params: lowered.params.clone(),
                })
                .collect(),
            calls_out: std::iter::once(&contract.init)
                .chain(&contract.arms)
                .filter(|handler| handler.calls_out)
                .map(|handler| handler.name.clone())
                .collect(),
        }
    }

    /// The slots to read after each transaction of a run that names
    /// `values`.
    pub fn reads<'v>(&self, values: impl IntoIterator<Item = &'v Value>) -> Reads {
        let keys = keys(values);
        let slots = match &self.layout {
            Ok(layout) => layout.slots(&self.state, &self.program.types, &keys),
            Err(_) => Vec::new(),
        };
        Reads { keys, slots }
    }

    /// The storage after one transaction, from the words at `reads.slots`.
    pub fn decode(&self, reads: &Reads, words: &[Word]) -> Result<Val, String> {
        let layout = self.layout.as_ref().map_err(Clone::clone)?;
        let read = |slot: &Word| {
            reads
                .slots
                .iter()
                .position(|read| read == slot)
                .and_then(|index| words.get(index).copied())
                .unwrap_or([0; 32])
        };
        layout
            .decode(&self.state, &self.program.types, &reads.keys, &read)
            .ok_or_else(|| "its storage does not decode as the contract's fields".to_string())
    }

    /// Calls a claim's function; it stops with a panic where the claim
    /// itself panics.
    pub fn holds(&self, func: FuncId, args: Vec<Val>) -> Result<bool, Stop> {
        match eval::call(&self.program, func, args) {
            Ok(value) => value
                .as_bool()
                .ok_or_else(|| Stop::Unsupported("the claim does not give a boolean".to_string())),
            Err(Halt::Panic(kind)) => Err(Stop::Panic(kind)),
            Err(halt) => Err(Stop::Unsupported(format!(
                "evaluating the claim stops: {halt}"
            ))),
        }
    }

    /// The first invariant `state` breaks, each at the values `extra`
    /// gives it in turn.
    pub fn broken(&self, state: &Val, extra: &[Val]) -> Result<Option<&str>, Stop> {
        let mut rest = extra.iter().cloned();
        for invariant in &self.invariants {
            let mut args = vec![state.clone()];
            args.extend(rest.by_ref().take(invariant.params.len()));
            if !self.holds(invariant.func, args)? {
                return Ok(Some(&invariant.name));
            }
        }
        Ok(None)
    }
}

/// Why a claim could not be evaluated on a contract's storage.
#[derive(Debug, Clone)]
pub enum Stop {
    Panic(femp_core::property::PanicKind),
    Unsupported(String),
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stop::Panic(kind) => write!(f, "the claim panics ({kind})"),
            Stop::Unsupported(reason) => f.write_str(reason),
        }
    }
}

/// The slots a run reads, for the map keys it can have touched.
pub struct Reads {
    keys: Vec<Val>,
    pub slots: Vec<Word>,
}

/// The map keys a run can touch: every value it names, as a key itself
/// and as the one field of a struct such as an address.
fn keys<'v>(values: impl IntoIterator<Item = &'v Value>) -> Vec<Val> {
    let mut keys: Vec<Val> = Vec::new();
    for value in values {
        let key = Val::of(value);
        let wrapped = Val::Adt {
            variant: 0,
            fields: vec![key.clone()],
        };
        for key in [key, wrapped] {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// Where a claim's quantified values are checked in a run: at every
/// combination of the values of their types the run names, and zero.
pub fn instances(params: &[Param], named: &[Value]) -> Vec<Vec<Val>> {
    let mut instances: Vec<Vec<Val>> = vec![Vec::new()];
    for param in params {
        let mut candidates = vec![Value::from_bits(param.ty, U256::ZERO)];
        for value in named.iter().filter(|value| value.ty() == param.ty) {
            if !candidates.contains(value) {
                candidates.push(*value);
            }
        }
        instances = instances
            .iter()
            .flat_map(|instance| {
                candidates.iter().map(move |candidate| {
                    let mut next = instance.clone();
                    next.push(Val::of(candidate));
                    next
                })
            })
            .take(MAX_INSTANCES)
            .collect();
    }
    instances
}
