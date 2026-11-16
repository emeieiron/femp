//! Turns local slots into SSA values. A block with one predecessor starts
//! with that predecessor's slot values; any other block receives every slot
//! as a block parameter, which `simplify` removes where all edges agree.
//! Loop headers are joins too, so loops stay correct for the loops pass.

use femp_vir::{Expr, Function, SlotId, StmtKind, ValueId};

use super::Pass;
use crate::task::{NotSupported, Task};

pub struct Slots;

impl Pass for Slots {
    fn name(&self) -> &'static str {
        "slots"
    }

    fn run(&self, task: &mut Task) -> Result<(), NotSupported> {
        promote(task.entry_mut());
        Ok(())
    }
}

fn promote(function: &mut Function) {
    let count = function.slots.len();
    if count == 0 {
        return;
    }
    let order = function.reverse_postorder();
    let mut reachable = vec![false; function.blocks.len()];
    for block in &order {
        reachable[block.index()] = true;
    }
    let preds: Vec<Vec<femp_vir::BlockId>> = function
        .predecessors()
        .into_iter()
        .map(|preds| preds.into_iter().filter(|p| reachable[p.index()]).collect())
        .collect();
    let joins: Vec<bool> = (0..function.blocks.len())
        .map(|index| index == function.entry.index() || preds[index].len() != 1)
        .collect();

    // Join blocks receive every slot as a parameter, after their own.
    let mut incoming: Vec<Option<Vec<ValueId>>> = vec![None; function.blocks.len()];
    for block in &order {
        if joins[block.index()] && *block != function.entry {
            let params: Vec<ValueId> = (0..count)
                .map(|slot| {
                    let info = &function.slots[slot];
                    function.new_value(info.ty.clone(), info.name.clone())
                })
                .collect();
            function
                .block_mut(*block)
                .params
                .extend(params.iter().copied());
            incoming[block.index()] = Some(params);
        }
    }

    let entry = function.entry;
    let mut outgoing: Vec<Option<Vec<ValueId>>> = vec![None; function.blocks.len()];
    for block in order {
        let mut state = if block == function.entry {
            let mut undefined = Vec::new();
            for slot in 0..count {
                let ty = function.slots[slot].ty.clone();
                undefined.push(function.new_value(ty, function.slots[slot].name.clone()));
            }
            let defs: Vec<femp_vir::Stmt> = undefined
                .iter()
                .map(|value| femp_vir::Stmt {
                    kind: StmtKind::Def {
                        dst: *value,
                        expr: Expr::Havoc,
                    },
                    span: function.span.clone(),
                })
                .collect();
            let entry = function.block_mut(block);
            entry.stmts.splice(0..0, defs);
            undefined
        } else if let Some(params) = &incoming[block.index()] {
            params.clone()
        } else {
            let pred = preds[block.index()][0];
            outgoing[pred.index()]
                .clone()
                .expect("a single predecessor comes first in reverse postorder")
        };
        let stmts = std::mem::take(&mut function.block_mut(block).stmts);
        let mut rewritten = Vec::with_capacity(stmts.len());
        for mut stmt in stmts {
            match &mut stmt.kind {
                StmtKind::Def { expr, .. } => {
                    if let Expr::Load { slot, path } = expr {
                        let current = state[slot.index()];
                        *expr = if path.is_empty() {
                            Expr::Use(current)
                        } else {
                            Expr::Project {
                                value: current,
                                path: std::mem::take(path),
                            }
                        };
                    }
                    rewritten.push(stmt);
                }
                StmtKind::Store { slot, path, value } => {
                    let SlotId(index) = *slot;
                    if path.is_empty() {
                        state[index as usize] = *value;
                    } else {
                        let ty = function.slots[index as usize].ty.clone();
                        let updated = function.new_value(ty, None);
                        let expr = Expr::Update {
                            value: state[index as usize],
                            path: std::mem::take(path),
                            new: *value,
                        };
                        state[index as usize] = updated;
                        rewritten.push(femp_vir::Stmt {
                            kind: StmtKind::Def { dst: updated, expr },
                            span: stmt.span,
                        });
                    }
                }
                _ => rewritten.push(stmt),
            }
        }
        let current = function.block_mut(block);
        current.stmts = rewritten;
        for succ in current.term.successors_mut() {
            if joins[succ.block.index()] && succ.block != entry {
                succ.args.extend(state.iter().copied());
            }
        }
        outgoing[block.index()] = Some(state);
    }
    function.slots.clear();
}
