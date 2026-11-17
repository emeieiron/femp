//! Replaces every call in the task's entry with a copy of the callee's
//! body, until no call is left. Recursion was rejected before, so this ends.
//! A contract's call through which another contract may call back becomes
//! what the task's reentry says instead.

use femp_vir::{
    BlockId, Check, Expr, Function, SlotId, Stmt, StmtKind, Succ, Terminator, Type, ValueId,
};

use super::Pass;
use crate::task::{NotSupported, Reentry, Task};

pub struct Inline;

impl Pass for Inline {
    fn name(&self) -> &'static str {
        "inline"
    }

    fn run(&self, task: &mut Task) -> Result<(), NotSupported> {
        while let Some((block, index, dst, func, args)) = first_call(task.entry()) {
            if let Some(reentry) = task.reentry.clone().filter(|reentry| reentry.func == func) {
                reenter(task.entry_mut(), block, index, dst, args[0], &reentry);
                continue;
            }
            let callee = task.program.function(func).clone();
            splice(task.entry_mut(), block, index, dst, &callee, &args);
        }
        Ok(())
    }
}

type Call = (BlockId, usize, ValueId, femp_vir::FuncId, Vec<ValueId>);

/// Replaces `dst = reenter(storage)`: the invariants checked of `storage`,
/// then any storage that satisfies them. The invariants' calls are inlined
/// in turn.
fn reenter(
    function: &mut Function,
    block: BlockId,
    index: usize,
    dst: ValueId,
    storage: ValueId,
    reentry: &Reentry,
) {
    let span = function.block(block).stmts[index].span.clone();
    let state: Type = function.ty(dst).clone();
    let mut stmts = Vec::new();
    let mut stmt = |kind| {
        stmts.push(Stmt {
            kind,
            span: span.clone(),
        })
    };
    for (func, name, args) in &reentry.checked {
        let holds = function.new_value(Type::Bool, None);
        let mut call_args = vec![storage];
        call_args.extend(args);
        stmt(StmtKind::Def {
            dst: holds,
            expr: Expr::Call {
                func: *func,
                args: call_args,
            },
        });
        stmt(StmtKind::Assert {
            cond: holds,
            check: Check::Reentry(name.clone()),
        });
    }
    let after = function.new_value(state, Some("reentered".to_string()));
    stmt(StmtKind::Def {
        dst: after,
        expr: Expr::Havoc,
    });
    for (func, args) in &reentry.assumed {
        let holds = function.new_value(Type::Bool, None);
        let mut call_args = vec![after];
        call_args.extend(args);
        stmt(StmtKind::Def {
            dst: holds,
            expr: Expr::Call {
                func: *func,
                args: call_args,
            },
        });
        stmt(StmtKind::Assume(holds));
    }
    stmt(StmtKind::Def {
        dst,
        expr: Expr::Use(after),
    });
    function.block_mut(block).stmts.splice(index..=index, stmts);
}

fn first_call(function: &Function) -> Option<Call> {
    for id in function.block_ids() {
        for (index, stmt) in function.block(id).stmts.iter().enumerate() {
            if let StmtKind::Def {
                dst,
                expr: Expr::Call { func, args },
            } = &stmt.kind
            {
                return Some((id, index, *dst, *func, args.clone()));
            }
        }
    }
    None
}

fn splice(
    caller: &mut Function,
    block: BlockId,
    index: usize,
    dst: ValueId,
    callee: &Function,
    args: &[ValueId],
) {
    let mut values: Vec<ValueId> = callee
        .values
        .iter()
        .map(|info| caller.new_value(info.ty.clone(), info.name.clone()))
        .collect();
    for (param, arg) in callee.params.iter().zip(args) {
        values[param.index()] = *arg;
    }
    let slots: Vec<SlotId> = callee
        .slots
        .iter()
        .map(|slot| caller.new_slot(slot.ty.clone(), slot.name.clone()))
        .collect();
    let blocks: Vec<BlockId> = callee.blocks.iter().map(|_| caller.new_block()).collect();

    let cont = caller.new_block();
    let call_block = caller.block_mut(block);
    let mode = call_block.mode;
    let tail = call_block.stmts.split_off(index + 1);
    call_block.stmts.pop();
    let term = std::mem::replace(
        &mut call_block.term,
        Terminator::Goto(Succ::to(blocks[callee.entry.index()])),
    );
    let span = call_block.span.clone();
    let cont_block = caller.block_mut(cont);
    cont_block.params = vec![dst];
    cont_block.stmts = tail;
    cont_block.term = term;
    cont_block.span = span;
    cont_block.mode = mode;

    let value = |id: ValueId| values[id.index()];
    for (old, new) in callee.blocks.iter().zip(&blocks) {
        let mut copy = old.clone();
        copy.mode = copy.mode.max(mode);
        copy.params = copy.params.iter().map(|param| value(*param)).collect();
        for stmt in &mut copy.stmts {
            for operand in stmt.kind.operands_mut() {
                *operand = value(*operand);
            }
            match &mut stmt.kind {
                StmtKind::Def { dst, expr } => {
                    *dst = value(*dst);
                    if let Expr::Load { slot, .. } = expr {
                        *slot = slots[slot.index()];
                    }
                }
                StmtKind::Store { slot, .. } => *slot = slots[slot.index()],
                _ => {}
            }
        }
        for operand in copy.term.operands_mut() {
            *operand = value(*operand);
        }
        for succ in copy.term.successors_mut() {
            succ.block = blocks[succ.block.index()];
        }
        if let Terminator::Return(result) = copy.term {
            copy.term = Terminator::Goto(Succ {
                block: cont,
                args: vec![result],
            });
        }
        *caller.block_mut(*new) = copy;
    }
}
