//! Replaces every call in the task's entry with a copy of the callee's
//! body, until no call is left. Recursion was rejected before, so this ends.

use femp_vir::{BlockId, Expr, Function, SlotId, StmtKind, Succ, Terminator, ValueId};

use super::Pass;
use crate::task::{NotSupported, Task};

pub struct Inline;

impl Pass for Inline {
    fn name(&self) -> &'static str {
        "inline"
    }

    fn run(&self, task: &mut Task) -> Result<(), NotSupported> {
        while let Some((block, index, dst, func, args)) = first_call(task.entry()) {
            let callee = task.program.function(func).clone();
            splice(task.entry_mut(), block, index, dst, &callee, &args);
        }
        Ok(())
    }
}

type Call = (BlockId, usize, ValueId, femp_vir::FuncId, Vec<ValueId>);

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
    let assumed = call_block.assumed;
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
    cont_block.assumed = assumed;

    let value = |id: ValueId| values[id.index()];
    for (old, new) in callee.blocks.iter().zip(&blocks) {
        let mut copy = old.clone();
        copy.assumed |= assumed;
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
