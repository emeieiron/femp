//! Gives a `#[spec]` function its meaning. A spec calls its target once;
//! every `requires` comes before that call and every `ensures` after it.
//! Proving the spec assumes what it requires and asserts what it ensures
//! around the real call.

use std::collections::{BTreeSet, HashMap};

use femp_vir::{BlockId, Check, Expr, FuncId, Function, SlotId, SpecOp, StmtKind, ValueId};

use super::Pass;
use crate::task::{NotSupported, Task};

pub struct SpecShape;

/// Where a spec calls its target.
pub(crate) struct Shape {
    pub block: BlockId,
    pub index: usize,
}

impl Pass for SpecShape {
    fn name(&self) -> &'static str {
        "spec_shape"
    }

    fn run(&self, task: &mut Task) -> Result<(), NotSupported> {
        let Some(proving) = task.proving.clone() else {
            return Ok(());
        };
        let spec = task.program.function(proving.spec);
        let span = spec.span.clone();
        shape(spec, proving.target, &proving.name)
            .map_err(|message| NotSupported::new(span, message))?;
        for block in &mut task.program.function_mut(proving.spec).blocks {
            for stmt in &mut block.stmts {
                stmt.kind = match stmt.kind {
                    StmtKind::Spec {
                        op: SpecOp::Requires,
                        cond,
                    } => StmtKind::Assume(cond),
                    StmtKind::Spec {
                        op: SpecOp::Ensures,
                        cond,
                    } => StmtKind::Assert {
                        cond,
                        check: Check::Ensures,
                    },
                    _ => continue,
                };
            }
        }
        Ok(())
    }
}

/// Checks the shape of `spec` and finds its call to `target`.
pub(crate) fn shape(spec: &Function, target: FuncId, name: &str) -> Result<Shape, String> {
    let calls: Vec<(BlockId, usize)> = spec
        .block_ids()
        .flat_map(|block| {
            spec.block(block)
                .stmts
                .iter()
                .enumerate()
                .filter(move |(_, stmt)| {
                    matches!(&stmt.kind, StmtKind::Def { expr: Expr::Call { func, .. }, .. } if *func == target)
                })
                .map(move |(index, _)| (block, index))
        })
        .collect();
    let [(block, index)] = calls.as_slice() else {
        return Err(format!(
            "the spec `{name}` must call the function it describes exactly once"
        ));
    };
    let after = reachable_after(spec, *block);
    if after.contains(block) {
        return Err(format!(
            "the spec `{name}` calls the function it describes in a loop"
        ));
    }
    for id in spec.block_ids() {
        for (position, stmt) in spec.block(id).stmts.iter().enumerate() {
            let later = after.contains(&id) || (id == *block && position > *index);
            match stmt.kind {
                StmtKind::Spec {
                    op: SpecOp::Requires,
                    ..
                } if later => {
                    return Err(format!(
                        "in the spec `{name}`, `requires` must come before the call it describes"
                    ));
                }
                StmtKind::Spec {
                    op: SpecOp::Ensures,
                    ..
                } if !later => {
                    return Err(format!(
                        "in the spec `{name}`, `ensures` must come after the call it describes"
                    ));
                }
                StmtKind::Spec {
                    op: SpecOp::Invariant,
                    ..
                } => {
                    return Err(format!(
                        "the spec `{name}` states an `invariant`, which belongs in a loop"
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(Shape {
        block: *block,
        index: *index,
    })
}

fn reachable_after(function: &Function, block: BlockId) -> BTreeSet<BlockId> {
    let mut seen = BTreeSet::new();
    let mut pending: Vec<BlockId> = function
        .block(block)
        .term
        .successors()
        .iter()
        .map(|succ| succ.block)
        .collect();
    while let Some(next) = pending.pop() {
        if seen.insert(next) {
            pending.extend(
                function
                    .block(next)
                    .term
                    .successors()
                    .iter()
                    .map(|s| s.block),
            );
        }
    }
    seen
}

/// Whether the spec passes its own parameters, unchanged and in order, to
/// the call at `shape`. Copies and stores into local slots are followed.
pub(crate) fn passes_parameters(spec: &Function, shape: &Shape) -> bool {
    let StmtKind::Def {
        expr: Expr::Call { args, .. },
        ..
    } = &spec.block(shape.block).stmts[shape.index].kind
    else {
        return false;
    };
    let mut copies: HashMap<ValueId, ValueId> = HashMap::new();
    let mut loads: HashMap<ValueId, SlotId> = HashMap::new();
    let mut stores: HashMap<SlotId, Vec<ValueId>> = HashMap::new();
    for block in &spec.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                StmtKind::Def {
                    dst,
                    expr: Expr::Use(source),
                } => {
                    copies.insert(*dst, *source);
                }
                StmtKind::Def {
                    dst,
                    expr: Expr::Load { slot, path },
                } if path.is_empty() => {
                    loads.insert(*dst, *slot);
                }
                StmtKind::Store { slot, path, value } => {
                    if path.is_empty() {
                        stores.entry(*slot).or_default().push(*value);
                    } else {
                        stores.entry(*slot).or_default().push(ValueId(u32::MAX));
                    }
                }
                _ => {}
            }
        }
    }
    let resolve = |mut value: ValueId| {
        for _ in 0..spec.values.len() + 1 {
            if let Some(source) = copies.get(&value) {
                value = *source;
            } else if let Some(slot) = loads.get(&value)
                && let Some([only]) = stores.get(slot).map(Vec::as_slice)
            {
                value = *only;
            } else {
                break;
            }
        }
        value
    };
    args.len() == spec.params.len()
        && args
            .iter()
            .zip(&spec.params)
            .all(|(arg, param)| resolve(*arg) == *param)
}
