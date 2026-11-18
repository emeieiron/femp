//! `#[machine]`: proved by induction. `#[init]` must establish every
//! `#[invariant]`; each `#[command]`, from any well-formed state on which
//! the invariants hold without panicking, must keep them, without panicking
//! itself. Observations must not panic either. When induction fails, a
//! bounded search from `#[init]` looks for a real sequence of commands.
//!
//! A contract is proved as a machine whose initializer and commands may
//! revert: a panic in them undoes the transaction, which then cannot break
//! an invariant, so the execution is not considered further. Its block
//! clock does not run backwards along a searched sequence.

use femp_core::property::{
    Command, Location, Machine, Method, Param, Sequence, Step, Ty, U256, Value,
};
use femp_spec::Domain;
use femp_vir::{
    BinOp, Check, Const, Cursor, Expr, FuncId, Function, Mode, Program, Succ, Terminator, Type,
    ValueId,
};

use super::inputs::{assume_domain, choose, vir_type};
use crate::task::{Input, Reentry, Task};

/// The functions of a machine's methods in a lowered program.
#[derive(Debug, Clone)]
pub struct MachineFns {
    pub init: FuncId,
    pub commands: Vec<FuncId>,
    pub invariants: Vec<FuncId>,
    pub observations: Vec<FuncId>,
    /// The initializer and commands revert rather than fail on a panic.
    pub reverts: bool,
    /// The function a contract's storage goes through where it calls
    /// another contract that may call back.
    pub reenter: Option<FuncId>,
}

/// Parameters of a contract's handlers that never decrease from one
/// transaction to the next.
const MONOTONIC: [&str; 2] = ["ctx.timestamp", "ctx.block_number"];

struct Builder<'a> {
    program: &'a Program,
    fns: &'a MachineFns,
    machine: &'a Machine,
    state: Type,
    entry: Function,
    inputs: Vec<Input>,
    assumed: Vec<String>,
    /// The latest value of each monotonic parameter along a sequence.
    clock: Vec<(&'static str, ValueId)>,
    reentry: Option<Reentry>,
}

impl<'a> Builder<'a> {
    fn new(program: &'a Program, fns: &'a MachineFns, machine: &'a Machine, name: &str) -> Self {
        let init = program.function(fns.init);
        Self {
            program,
            fns,
            machine,
            state: init.result.clone(),
            entry: Function::new(name, Type::Unit, init.span.clone()),
            inputs: Vec::new(),
            assumed: Vec::new(),
            clock: Vec::new(),
            reentry: None,
        }
    }

    /// Makes calls that let other contracts call back keep the invariants:
    /// checked at the call at `checked` when given, and assumed of the
    /// storage afterwards at `assumed`.
    fn reenters(&mut self, checked: Option<&[Vec<ValueId>]>, assumed: Vec<(FuncId, Vec<ValueId>)>) {
        let Some(func) = self.fns.reenter else {
            return;
        };
        let checked = match checked {
            Some(at) => self
                .fns
                .invariants
                .iter()
                .zip(&self.machine.invariants)
                .zip(at)
                .map(|((func, invariant), args)| (*func, invariant.name.clone(), args.clone()))
                .collect(),
            None => Vec::new(),
        };
        self.reentry = Some(Reentry {
            func,
            checked,
            assumed,
        });
    }

    fn result(&self, func: FuncId) -> Type {
        self.program.function(func).result.clone()
    }

    fn cursor(&mut self, block: femp_vir::BlockId) -> Cursor<'_> {
        Cursor::new(&mut self.entry, block)
    }

    /// Calls `#[init]` with chosen arguments, and returns the state and the
    /// block that continues after it.
    fn init(&mut self, block: femp_vir::BlockId) -> Result<(ValueId, femp_vir::BlockId), String> {
        let state = self.state.clone();
        let (init, machine) = (self.fns.init, self.machine);
        let mut cursor = Cursor::new(&mut self.entry, block);
        let args = choose(
            &mut cursor,
            &machine.init.params,
            &mut self.inputs,
            &mut self.assumed,
        )?;
        for (param, value) in machine.init.params.iter().zip(&args) {
            if let Some(name) = MONOTONIC.iter().find(|name| **name == param.name) {
                self.clock.push((name, *value));
            }
        }
        Ok(self.run(block, init, args, state))
    }

    /// Calls `func` from `block`. A call that may revert runs in a block of
    /// its own where a panic ends the execution instead of failing it; what
    /// follows continues in a new block of `block`'s mode.
    fn run(
        &mut self,
        block: femp_vir::BlockId,
        func: FuncId,
        args: Vec<ValueId>,
        ty: Type,
    ) -> (ValueId, femp_vir::BlockId) {
        if !self.fns.reverts {
            return (self.cursor(block).call(func, args, ty), block);
        }
        let mode = self.entry.block(block).mode;
        let run = self.entry.new_block();
        self.entry.block_mut(run).mode = mode.max(Mode::Trusted);
        let after = self.entry.new_block();
        self.entry.block_mut(after).mode = mode;
        self.cursor(block)
            .terminate(Terminator::Goto(Succ::to(run)));
        let value = self.cursor(run).call(func, args, ty);
        self.cursor(run)
            .terminate(Terminator::Goto(Succ::to(after)));
        (value, after)
    }

    /// Makes the monotonic parameters of every command at a step one value
    /// that is at least the one before it.
    fn tick(&mut self, block: femp_vir::BlockId, args: &[Vec<ValueId>]) {
        let machine = self.machine;
        for name in MONOTONIC {
            let values: Vec<ValueId> = machine
                .commands
                .iter()
                .zip(args)
                .flat_map(|(command, args)| {
                    command
                        .method
                        .params
                        .iter()
                        .zip(args)
                        .filter(|(param, _)| param.name == name)
                        .map(|(_, value)| *value)
                })
                .collect();
            let Some(first) = values.first().copied() else {
                continue;
            };
            let previous = self
                .clock
                .iter()
                .find(|(known, _)| *known == name)
                .map(|(_, value)| *value);
            let mut cursor = Cursor::new(&mut self.entry, block);
            for other in &values[1..] {
                let same = cursor.binary(BinOp::Eq, *other, first);
                cursor.assume(same);
            }
            if let Some(previous) = previous {
                let later = cursor.binary(BinOp::Ge, first, previous);
                cursor.assume(later);
            }
            self.clock.retain(|(known, _)| *known != name);
            self.clock.push((name, first));
        }
    }

    /// Arguments for an invariant's parameters other than the state, which
    /// it holds for every value of: inputs a counterexample reports, or any
    /// values, unreported.
    fn every(
        &mut self,
        block: femp_vir::BlockId,
        method: &Method,
        report: bool,
    ) -> Result<Vec<ValueId>, String> {
        let mut cursor = Cursor::new(&mut self.entry, block);
        if report {
            return choose(
                &mut cursor,
                &method.params,
                &mut self.inputs,
                &mut self.assumed,
            );
        }
        Ok(method
            .params
            .iter()
            .map(|param| cursor.havoc(vir_type(param.ty), Some(param.name.clone())))
            .collect())
    }

    /// Asserts every invariant of `state`, and calls every observation so
    /// that a panicking one fails too. In an assumed block, the invariants
    /// are assumed instead. An invariant with parameters is checked at the
    /// arguments `at` gives it, or at new ones.
    fn check(
        &mut self,
        block: femp_vir::BlockId,
        state: ValueId,
        after: &str,
        at: Option<&[Vec<ValueId>]>,
    ) -> Result<(), String> {
        let assumed = self.entry.block(block).mode == Mode::Assumed;
        let mut invariants: Vec<(FuncId, String, Vec<ValueId>)> = Vec::new();
        for (index, (func, method)) in self
            .fns
            .invariants
            .iter()
            .zip(&self.machine.invariants)
            .enumerate()
        {
            let args = match at {
                Some(at) => at[index].clone(),
                None => self.every(block, method, !assumed)?,
            };
            invariants.push((*func, method.name.clone(), args));
        }
        let observations: Vec<(FuncId, Type)> = self
            .fns
            .observations
            .iter()
            .map(|func| (*func, self.result(*func)))
            .collect();
        let mut cursor = self.cursor(block);
        for (func, name, args) in invariants {
            let mut call_args = vec![state];
            call_args.extend(args);
            let holds = cursor.call(func, call_args, Type::Bool);
            if assumed {
                cursor.assume(holds);
            } else {
                cursor.assert(
                    holds,
                    Check::Claim(format!("invariant `{name}` fails after {after}")),
                );
            }
        }
        for (func, ty) in observations {
            cursor.call(func, vec![state], ty);
        }
        Ok(())
    }

    fn finish(mut self, block: femp_vir::BlockId) -> Task {
        let mut cursor = self.cursor(block);
        let unit = cursor.def(Type::Unit, Expr::Const(Const::Unit));
        cursor.terminate(Terminator::Return(unit));
        let mut program = self.program.clone();
        let entry = program.push(self.entry);
        Task {
            inputs: self.inputs,
            assumed: self.assumed,
            reentry: self.reentry,
            ..Task::new(program, entry)
        }
    }
}

pub fn init_task(program: &Program, fns: &MachineFns, machine: &Machine) -> Result<Task, String> {
    let mut builder = Builder::new(
        program,
        fns,
        machine,
        &format!("prove {}::init", machine.name),
    );
    // A contract has no code while `init` runs, so no callee can call
    // back into it.
    let block = builder.entry.entry;
    let (state, block) = builder.init(block)?;
    builder.check(block, state, &format!("`{}`", machine.init.name), None)?;
    Ok(builder.finish(block))
}

pub fn step_task(
    program: &Program,
    fns: &MachineFns,
    machine: &Machine,
    command: usize,
) -> Result<Task, String> {
    let method = &machine.commands[command].method;
    let mut builder = Builder::new(
        program,
        fns,
        machine,
        &format!("prove {}::{}", machine.name, method.name),
    );
    let first = builder.entry.entry;
    let state_ty = builder.state.clone();
    let state = {
        let mut cursor = builder.cursor(first);
        cursor.havoc(state_ty.clone(), Some("state".to_string()))
    };
    builder.inputs.push(Input {
        name: "state".to_string(),
        value: state,
        domain: None,
    });
    let chosen = {
        let mut cursor = Cursor::new(&mut builder.entry, first);
        choose(
            &mut cursor,
            &method.params,
            &mut builder.inputs,
            &mut builder.assumed,
        )?
    };
    // Where each invariant is checked after the command.
    let mut at = Vec::new();
    for invariant in &machine.invariants {
        at.push(builder.every(first, invariant, true)?);
    }
    // The hypothesis: the invariants hold of the arbitrary state, which is
    // also one they can be evaluated on without panicking. One with
    // parameters is assumed where it is checked, and at every value of a
    // parameter's type the command receives.
    let hypothesis = builder.entry.new_block();
    builder.entry.block_mut(hypothesis).mode = Mode::Assumed;
    let body = builder.entry.new_block();
    builder
        .cursor(first)
        .terminate(Terminator::Goto(Succ::to(hypothesis)));
    let mut holds = Vec::new();
    let mut hypotheses = Vec::new();
    {
        let mut cursor = builder.cursor(hypothesis);
        for ((func, invariant), checked) in fns.invariants.iter().zip(&machine.invariants).zip(&at)
        {
            for instance in instances(invariant, checked, &method.params, &chosen) {
                hypotheses.push((*func, instance.clone()));
                let mut args = vec![state];
                args.extend(instance);
                holds.push(cursor.call(*func, args, Type::Bool));
            }
        }
        cursor.terminate(Terminator::Goto(Succ::to(body)));
    }
    builder.reenters(Some(&at), hypotheses);
    {
        let mut cursor = Cursor::new(&mut builder.entry, body);
        for held in holds {
            cursor.assume(held);
        }
    }
    let mut args = vec![state];
    args.extend(chosen);
    let (next, after) = builder.run(body, fns.commands[command], args, state_ty);
    builder.check(after, next, &format!("`{}`", method.name), Some(&at))?;
    Ok(builder.finish(after))
}

/// The search for a sequence of `steps` commands from `#[init]` that breaks
/// an invariant or panics at its last step. The steps before it are assumed
/// to go well, since shorter searches checked them. Its inputs are the
/// initializer's arguments, then for each step the command chosen and the
/// arguments of every command.
pub fn search_task(
    program: &Program,
    fns: &MachineFns,
    machine: &Machine,
    steps: usize,
) -> Result<Task, String> {
    let mut builder = Builder::new(
        program,
        fns,
        machine,
        &format!("search {} in {steps} steps", machine.name),
    );
    let (_, block) = sequence(&mut builder, steps, true)?;
    Ok(Task {
        search: true,
        ..builder.finish(block)
    })
}

/// `steps` commands from `#[init]`, each checked against the invariants;
/// only the last step's checks are obligations, and only when `checked`.
/// Returns the state and the block that continues after it.
fn sequence(
    builder: &mut Builder<'_>,
    steps: usize,
    checked: bool,
) -> Result<(ValueId, femp_vir::BlockId), String> {
    let machine = builder.machine;
    let fns = builder.fns;
    let mut block = builder.entry.entry;
    builder.entry.block_mut(block).mode = mode(steps > 0 || !checked);
    let (mut state, after) = builder.init(block)?;
    block = after;
    builder.check(block, state, &format!("`{}`", machine.init.name), None)?;
    let count = machine.commands.len();
    let choice_ty = Ty::Uint(32);
    let state_ty = builder.state.clone();
    for step in 1..=steps {
        let (choice, args) = {
            let mut cursor = Cursor::new(&mut builder.entry, block);
            let choice = cursor.havoc(Type::uint(32), Some(format!("step {step}")));
            let domain = Domain::Range {
                lo: Value::from_ordinal(choice_ty, U256::ZERO),
                hi: Value::from_ordinal(choice_ty, U256::from(count as u64 - 1)),
            };
            assume_domain(&mut cursor, choice, choice_ty, &domain);
            builder.inputs.push(Input {
                name: format!("step {step}"),
                value: choice,
                domain: Some((choice_ty, domain)),
            });
            let mut args = Vec::new();
            for command in &machine.commands {
                args.push(choose(
                    &mut cursor,
                    &command.method.params,
                    &mut builder.inputs,
                    &mut builder.assumed,
                )?);
            }
            (choice, args)
        };
        builder.tick(block, &args);
        let earlier = step < steps || !checked;
        let join = builder.entry.new_block();
        builder.entry.block_mut(join).mode = mode(earlier);
        let next = builder.entry.new_value(state_ty.clone(), None);
        builder.entry.block_mut(join).params.push(next);
        for (index, command_args) in args.into_iter().enumerate() {
            let run = builder.entry.new_block();
            builder.entry.block_mut(run).mode = if fns.reverts {
                mode(earlier).max(Mode::Trusted)
            } else {
                mode(earlier)
            };
            let rest = if index + 1 < count {
                let rest = builder.entry.new_block();
                builder.entry.block_mut(rest).mode = mode(earlier);
                rest
            } else {
                run
            };
            if index + 1 < count {
                let mut cursor = Cursor::new(&mut builder.entry, block);
                let wanted = cursor.int(
                    &Type::uint(32),
                    Value::from_u128(choice_ty, index as u128).unwrap_or(Value::boolean(false)),
                );
                let chosen = cursor.binary(BinOp::Eq, choice, wanted);
                cursor.terminate(Terminator::Branch {
                    cond: chosen,
                    then: Succ::to(run),
                    otherwise: Succ::to(rest),
                });
            } else {
                Cursor::new(&mut builder.entry, block).terminate(Terminator::Goto(Succ::to(run)));
            }
            let mut cursor = Cursor::new(&mut builder.entry, run);
            let mut call_args = vec![state];
            call_args.extend(command_args);
            let after = cursor.call(fns.commands[index], call_args, state_ty.clone());
            cursor.terminate(Terminator::Goto(Succ {
                block: join,
                args: vec![after],
            }));
            block = rest;
        }
        builder.check(join, next, &format!("step {step}"), None)?;
        block = join;
        state = next;
    }
    Ok((state, block))
}

/// Most instances of an invariant's hypothesis an induction step assumes.
const MAX_INSTANCES: usize = 32;

/// Where an invariant with parameters is assumed before a command: at the
/// arguments it is checked at afterwards, and at every argument of the
/// command that has a parameter's type, since those name the parts of the
/// state the command can touch.
fn instances(
    invariant: &Method,
    checked: &[ValueId],
    params: &[Param],
    args: &[ValueId],
) -> Vec<Vec<ValueId>> {
    let mut instances: Vec<Vec<ValueId>> = vec![Vec::new()];
    for (param, at) in invariant.params.iter().zip(checked) {
        let mut candidates = vec![*at];
        candidates.extend(
            params
                .iter()
                .zip(args)
                .filter(|(other, _)| other.ty == param.ty)
                .map(|(_, value)| *value),
        );
        instances = instances
            .into_iter()
            .flat_map(|prefix| {
                candidates.iter().map(move |candidate| {
                    let mut next = prefix.clone();
                    next.push(*candidate);
                    next
                })
            })
            .take(MAX_INSTANCES)
            .collect();
    }
    instances
}

fn mode(assumed: bool) -> Mode {
    if assumed {
        Mode::Assumed
    } else {
        Mode::Checked
    }
}

/// The sequence a refuted search task's inputs describe.
pub fn decode(machine: &Machine, steps: usize, values: &[Value]) -> Option<Sequence> {
    let mut rest = values;
    let take = |rest: &mut &[Value], count: usize| -> Option<Vec<Value>> {
        let (taken, remaining) = (rest.get(..count)?, rest.get(count..)?);
        *rest = remaining;
        Some(taken.to_vec())
    };
    let init = take(&mut rest, machine.init.params.len())?;
    let mut sequence = Vec::new();
    for _ in 0..steps {
        let choice = take(&mut rest, 1)?[0];
        let mut per_command = Vec::new();
        for command in &machine.commands {
            per_command.push(take(&mut rest, command.method.params.len())?);
        }
        let command = choice.bits().to::<u64>() as usize;
        sequence.push(Step {
            command,
            args: per_command.get(command)?.clone(),
            advance: 0,
        });
    }
    Some(Sequence {
        init,
        steps: sequence,
    })
}

/// How many of a search's inputs the deployment and its first `steps`
/// steps take; the values the last state is checked at follow.
pub fn inputs_through(machine: &Machine, steps: usize) -> usize {
    let per_step: usize = 1 + machine
        .commands
        .iter()
        .map(|command| command.method.params.len())
        .sum::<usize>();
    machine.init.params.len() + steps * per_step
}

/// The arguments of `method` among a step task's inputs, after its state.
pub fn method_args<'v>(method: &Method, values: &'v [Value]) -> &'v [Value] {
    &values[..method.params.len().min(values.len())]
}

/// A handler of a contract, as the machine it is proved as calls it.
pub struct ContractHandler<'a> {
    pub name: &'a str,
    pub params: &'a [Param],
    pub func: FuncId,
}

/// A contract as a machine: its `init` is the initializer, its message
/// handlers the commands, and its storage invariants the invariants.
pub fn contract_machine(
    name: &str,
    path: &[String],
    location: Location,
    init: ContractHandler<'_>,
    arms: &[ContractHandler<'_>],
    invariants: &[(&str, &[Param], FuncId)],
    reenter: Option<FuncId>,
) -> (Machine, MachineFns) {
    let method = |name: &str, params: &[Param]| Method {
        name: name.to_string(),
        params: params.to_vec(),
        effects: Vec::new(),
        location: location.clone(),
    };
    let machine = Machine {
        name: name.to_string(),
        path: path.to_vec(),
        init: method(init.name, init.params),
        commands: arms
            .iter()
            .map(|arm| Command {
                method: method(arm.name, arm.params),
                weight: 1,
            })
            .collect(),
        invariants: invariants
            .iter()
            .map(|(name, params, _)| method(name, params))
            .collect(),
        observations: Vec::new(),
        steps: "1..=16".to_string(),
        runs: None,
        clock: None,
        reuse: false,
        location: location.clone(),
    };
    let fns = MachineFns {
        init: init.func,
        commands: arms.iter().map(|arm| arm.func).collect(),
        invariants: invariants.iter().map(|(_, _, func)| *func).collect(),
        observations: Vec::new(),
        reverts: true,
        reenter,
    };
    (machine, fns)
}

/// What a message claim asks of its message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageCheck {
    /// The claim holds after every transaction of the message that goes
    /// through.
    Ensures,
    /// Where the claim holds, the message reverts.
    Reverts,
    /// Where the claim does not hold, the message goes through.
    GoesThrough,
}

/// A message claim of a contract proved as a machine.
pub struct MessageFn<'a> {
    pub name: &'a str,
    pub func: FuncId,
    /// The index of the message's command.
    pub command: usize,
    /// The values the claim ranges over.
    pub params: &'a [Param],
}

/// The claim, from any state the invariants allow.
pub fn message_task(
    program: &Program,
    fns: &MachineFns,
    machine: &Machine,
    claim: &MessageFn<'_>,
    check: MessageCheck,
) -> Result<Task, String> {
    let mut builder = Builder::new(
        program,
        fns,
        machine,
        &format!("prove {}::{}", machine.name, claim.name),
    );
    let first = builder.entry.entry;
    let state_ty = builder.state.clone();
    let state = builder
        .cursor(first)
        .havoc(state_ty, Some("state".to_string()));
    builder.inputs.push(Input {
        name: "state".to_string(),
        value: state,
        domain: None,
    });
    let block = message_step(&mut builder, first, state, claim, check, true)?;
    Ok(builder.finish(block))
}

/// The claim at the end of a real sequence: `before` commands from
/// `#[init]`, then the message.
pub fn message_search_task(
    program: &Program,
    fns: &MachineFns,
    machine: &Machine,
    claim: &MessageFn<'_>,
    check: MessageCheck,
    before: usize,
) -> Result<Task, String> {
    let mut builder = Builder::new(
        program,
        fns,
        machine,
        &format!(
            "search {}::{} after {before} steps",
            machine.name, claim.name
        ),
    );
    let (state, block) = sequence(&mut builder, before, false)?;
    let last = builder.entry.new_block();
    builder
        .cursor(block)
        .terminate(Terminator::Goto(Succ::to(last)));
    let block = message_step(&mut builder, last, state, claim, check, false)?;
    Ok(Task {
        search: true,
        ..builder.finish(block)
    })
}

/// Sends the message from `state` and checks the claim; with `hypothesis`,
/// the invariants are first assumed of the state.
fn message_step(
    builder: &mut Builder<'_>,
    block: femp_vir::BlockId,
    state: ValueId,
    claim: &MessageFn<'_>,
    check: MessageCheck,
    hypothesis: bool,
) -> Result<femp_vir::BlockId, String> {
    let machine = builder.machine;
    let fns = builder.fns;
    let method = &machine.commands[claim.command].method;
    let state_ty = builder.state.clone();
    let (chosen, every) = {
        let mut cursor = Cursor::new(&mut builder.entry, block);
        let chosen = choose(
            &mut cursor,
            &method.params,
            &mut builder.inputs,
            &mut builder.assumed,
        )?;
        let every = choose(
            &mut cursor,
            claim.params,
            &mut builder.inputs,
            &mut builder.assumed,
        )?;
        (chosen, every)
    };
    // The values the message and the claim name, by type, where the
    // invariants are assumed.
    let mut params: Vec<Param> = method.params.clone();
    params.extend(claim.params.iter().cloned());
    let mut values = chosen.clone();
    values.extend(every.iter().copied());

    let assumed = builder.entry.new_block();
    builder.entry.block_mut(assumed).mode = Mode::Assumed;
    builder
        .cursor(block)
        .terminate(Terminator::Goto(Succ::to(assumed)));
    if hypothesis {
        let mut holds = Vec::new();
        let mut hypotheses = Vec::new();
        for (func, invariant) in fns.invariants.iter().zip(&machine.invariants) {
            let at = builder.every(assumed, invariant, false)?;
            let mut cursor = builder.cursor(assumed);
            for instance in instances(invariant, &at, &params, &values) {
                hypotheses.push((*func, instance.clone()));
                let mut args = vec![state];
                args.extend(instance);
                holds.push(cursor.call(*func, args, Type::Bool));
            }
        }
        builder.reenters(None, hypotheses);
        let mut cursor = builder.cursor(assumed);
        for held in holds {
            cursor.assume(held);
        }
    }
    let mut command_args = vec![state];
    command_args.extend(chosen.iter().copied());
    let mut claim_args = vec![state];
    let body = builder.entry.new_block();
    match check {
        MessageCheck::Ensures => {
            builder
                .cursor(assumed)
                .terminate(Terminator::Goto(Succ::to(body)));
            let (after, block) =
                builder.run(body, fns.commands[claim.command], command_args, state_ty);
            claim_args.push(after);
            claim_args.extend(values);
            let mut cursor = builder.cursor(block);
            let holds = cursor.call(claim.func, claim_args, Type::Bool);
            cursor.assert(
                holds,
                Check::Claim(format!(
                    "`{}` does not hold after `{}`",
                    claim.name, method.name
                )),
            );
            Ok(block)
        }
        MessageCheck::Reverts | MessageCheck::GoesThrough => {
            claim_args.extend(values);
            {
                let mut cursor = builder.cursor(assumed);
                let holds = cursor.call(claim.func, claim_args, Type::Bool);
                let wanted = if check == MessageCheck::Reverts {
                    holds
                } else {
                    cursor.not(holds)
                };
                cursor.assume(wanted);
                cursor.terminate(Terminator::Goto(Succ::to(body)));
            }
            if check == MessageCheck::Reverts {
                let (_, block) =
                    builder.run(body, fns.commands[claim.command], command_args, state_ty);
                let mut cursor = builder.cursor(block);
                let never = cursor.boolean(false);
                cursor.assert(
                    never,
                    Check::Claim(format!(
                        "`{}` goes through although `{}` holds",
                        method.name, claim.name
                    )),
                );
                Ok(block)
            } else {
                // Any panic or revert of the message is a failure here.
                builder
                    .cursor(body)
                    .call(fns.commands[claim.command], command_args, state_ty);
                Ok(body)
            }
        }
    }
}

/// The sequence a refuted message search task's inputs describe: the steps
/// before the message, then the message.
pub fn decode_message(
    machine: &Machine,
    command: usize,
    before: usize,
    values: &[Value],
) -> Option<Sequence> {
    let mut sequence = decode(machine, before, values)?;
    let consumed = inputs_through(machine, before);
    let count = machine.commands[command].method.params.len();
    let args = values.get(consumed..consumed + count)?.to_vec();
    sequence.steps.push(Step {
        command,
        args,
        advance: 0,
    });
    Some(sequence)
}
