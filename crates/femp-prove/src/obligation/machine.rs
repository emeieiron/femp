//! `#[machine]`: proved by induction. `#[init]` must establish every
//! `#[invariant]`; each `#[command]`, from any well-formed state on which
//! the invariants hold without panicking, must keep them, without panicking
//! itself. Observations must not panic either. When induction fails, a
//! bounded search from `#[init]` looks for a real sequence of commands.

use femp_core::property::{Machine, Method, Sequence, Step, Ty, U256, Value};
use femp_spec::Domain;
use femp_vir::{
    BinOp, Check, Const, Cursor, Expr, FuncId, Function, Program, Succ, Terminator, Type, ValueId,
};

use super::inputs::{assume_domain, choose};
use crate::task::{Input, Task};

/// The functions of a machine's methods in a lowered program.
#[derive(Debug, Clone)]
pub struct MachineFns {
    pub init: FuncId,
    pub commands: Vec<FuncId>,
    pub invariants: Vec<FuncId>,
    pub observations: Vec<FuncId>,
}

struct Builder<'a> {
    program: &'a Program,
    fns: &'a MachineFns,
    machine: &'a Machine,
    state: Type,
    entry: Function,
    inputs: Vec<Input>,
    assumed: Vec<String>,
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
        }
    }

    fn result(&self, func: FuncId) -> Type {
        self.program.function(func).result.clone()
    }

    fn cursor(&mut self, block: femp_vir::BlockId) -> Cursor<'_> {
        Cursor::new(&mut self.entry, block)
    }

    /// Calls `#[init]` with chosen arguments.
    fn init(&mut self, block: femp_vir::BlockId) -> Result<ValueId, String> {
        let state = self.state.clone();
        let (init, machine) = (self.fns.init, self.machine);
        let mut cursor = Cursor::new(&mut self.entry, block);
        let args = choose(
            &mut cursor,
            &machine.init.params,
            &mut self.inputs,
            &mut self.assumed,
        )?;
        Ok(cursor.call(init, args, state))
    }

    /// Asserts every invariant of `state`, and calls every observation so
    /// that a panicking one fails too. In an assumed block, the invariants
    /// are assumed instead.
    fn check(&mut self, block: femp_vir::BlockId, state: ValueId, after: &str) {
        let assumed = self.entry.block(block).assumed;
        let invariants: Vec<(FuncId, String)> = self
            .fns
            .invariants
            .iter()
            .zip(&self.machine.invariants)
            .map(|(func, method)| (*func, method.name.clone()))
            .collect();
        let observations: Vec<(FuncId, Type)> = self
            .fns
            .observations
            .iter()
            .map(|func| (*func, self.result(*func)))
            .collect();
        let mut cursor = self.cursor(block);
        for (func, name) in invariants {
            let holds = cursor.call(func, vec![state], Type::Bool);
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
    }

    fn finish(mut self, block: femp_vir::BlockId) -> Task {
        let mut cursor = self.cursor(block);
        let unit = cursor.def(Type::Unit, Expr::Const(Const::Unit));
        cursor.terminate(Terminator::Return(unit));
        let mut program = self.program.clone();
        let entry = program.push(self.entry);
        Task {
            program,
            entry,
            inputs: self.inputs,
            assumed: self.assumed,
            timeout: None,
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
    let block = builder.entry.entry;
    let state = builder.init(block)?;
    builder.check(block, state, &format!("`{}`", machine.init.name));
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
    // The hypothesis: the invariants hold of the arbitrary state, which is
    // also one they can be evaluated on without panicking.
    let hypothesis = builder.entry.new_block();
    builder.entry.block_mut(hypothesis).assumed = true;
    let body = builder.entry.new_block();
    builder
        .cursor(first)
        .terminate(Terminator::Goto(Succ::to(hypothesis)));
    let invariants = fns.invariants.clone();
    let mut holds = Vec::new();
    {
        let mut cursor = builder.cursor(hypothesis);
        for func in &invariants {
            holds.push(cursor.call(*func, vec![state], Type::Bool));
        }
        cursor.terminate(Terminator::Goto(Succ::to(body)));
    }
    let next = {
        let mut cursor = Cursor::new(&mut builder.entry, body);
        for held in holds {
            cursor.assume(held);
        }
        let mut args = vec![state];
        args.extend(choose(
            &mut cursor,
            &method.params,
            &mut builder.inputs,
            &mut builder.assumed,
        )?);
        cursor.call(fns.commands[command], args, state_ty)
    };
    builder.check(body, next, &format!("`{}`", method.name));
    Ok(builder.finish(body))
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
    let mut block = builder.entry.entry;
    builder.entry.block_mut(block).assumed = steps > 0;
    let mut state = builder.init(block)?;
    builder.check(block, state, &format!("`{}`", machine.init.name));
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
        let earlier = step < steps;
        let join = builder.entry.new_block();
        builder.entry.block_mut(join).assumed = earlier;
        let next = builder.entry.new_value(state_ty.clone(), None);
        builder.entry.block_mut(join).params.push(next);
        for (index, command_args) in args.into_iter().enumerate() {
            let run = builder.entry.new_block();
            builder.entry.block_mut(run).assumed = earlier;
            let rest = if index + 1 < count {
                let rest = builder.entry.new_block();
                builder.entry.block_mut(rest).assumed = earlier;
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
        builder.check(join, next, &format!("step {step}"));
        block = join;
        state = next;
    }
    Ok(builder.finish(block))
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

/// The arguments of `method` among a step task's inputs, after its state.
pub fn method_args<'v>(method: &Method, values: &'v [Value]) -> &'v [Value] {
    &values[..method.params.len().min(values.len())]
}
