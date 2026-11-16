//! Cleans the entry up before encoding: drops blocks nothing reaches,
//! forwards copies, removes block parameters every edge agrees on, merges
//! straight-line chains, deletes assertions of `true` and unused
//! definitions, and turns masks of low bits into remainders. The task's
//! inputs are kept, since their values are reported.

use std::collections::{HashMap, HashSet};

use femp_core::property::{Ty, U256, Value};
use femp_vir::{Arith, BinOp, BlockId, Const, Expr, Function, Stmt, StmtKind, Terminator, ValueId};

use super::Pass;
use crate::task::{NotSupported, Task};

pub struct Simplify;

impl Pass for Simplify {
    fn name(&self) -> &'static str {
        "simplify"
    }

    fn run(&self, task: &mut Task) -> Result<(), NotSupported> {
        let keep: HashSet<ValueId> = task.inputs.iter().map(|input| input.value).collect();
        simplify(task.entry_mut(), &keep);
        Ok(())
    }
}

pub(crate) fn simplify(function: &mut Function, keep: &HashSet<ValueId>) {
    loop {
        let before = function.clone();
        remove_unreachable(function);
        forward_copies(function, keep);
        drop_agreeing_params(function);
        drop_unread_params(function);
        merge_chains(function);
        fold_masks(function);
        fold_constants(function);
        drop_trivial_checks(function);
        remove_unused(function, keep);
        if *function == before {
            break;
        }
    }
}

fn remove_unreachable(function: &mut Function) {
    let order = function.reverse_postorder();
    if order.len() == function.blocks.len() {
        return;
    }
    let mut renumbered: HashMap<BlockId, BlockId> = HashMap::new();
    let mut ids: Vec<BlockId> = order.clone();
    ids.sort();
    for (index, old) in ids.iter().enumerate() {
        renumbered.insert(*old, BlockId(index as u32));
    }
    let mut blocks = Vec::with_capacity(ids.len());
    for old in &ids {
        let mut block = function.block(*old).clone();
        for succ in block.term.successors_mut() {
            succ.block = renumbered[&succ.block];
        }
        blocks.push(block);
    }
    function.entry = renumbered[&function.entry];
    function.blocks = blocks;
}

fn forward_copies(function: &mut Function, keep: &HashSet<ValueId>) {
    let mut copies: HashMap<ValueId, ValueId> = HashMap::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            if let StmtKind::Def {
                dst,
                expr: Expr::Use(source),
            } = &stmt.kind
                && !keep.contains(dst)
            {
                copies.insert(*dst, *source);
            }
        }
    }
    if copies.is_empty() {
        return;
    }
    let resolve = |mut value: ValueId| {
        let mut steps = 0;
        while let Some(next) = copies.get(&value) {
            value = *next;
            steps += 1;
            if steps > copies.len() {
                break;
            }
        }
        value
    };
    function.rename_uses(&resolve);
    for block in &mut function.blocks {
        block.stmts.retain(|stmt| {
            !matches!(&stmt.kind, StmtKind::Def { dst, expr: Expr::Use(_) } if copies.contains_key(dst))
        });
    }
}

fn drop_agreeing_params(function: &mut Function) {
    loop {
        let preds = function.predecessors();
        let mut replacement: Option<(BlockId, usize, ValueId)> = None;
        'search: for id in function.block_ids() {
            if id == function.entry || preds[id.index()].is_empty() {
                continue;
            }
            let params = function.block(id).params.clone();
            for (position, param) in params.iter().enumerate() {
                let mut agreed: Option<ValueId> = None;
                let mut unanimous = true;
                for pred in function.block_ids() {
                    for succ in function.block(pred).term.successors() {
                        if succ.block != id {
                            continue;
                        }
                        let arg = succ.args[position];
                        if arg == *param {
                            continue;
                        }
                        match agreed {
                            None => agreed = Some(arg),
                            Some(value) if value == arg => {}
                            Some(_) => unanimous = false,
                        }
                    }
                }
                if unanimous && let Some(value) = agreed {
                    replacement = Some((id, position, value));
                    break 'search;
                }
            }
        }
        let Some((block, position, value)) = replacement else {
            return;
        };
        let param = function.block_mut(block).params.remove(position);
        for pred in function.block_ids().collect::<Vec<_>>() {
            for succ in function.block_mut(pred).term.successors_mut() {
                if succ.block == block {
                    succ.args.remove(position);
                }
            }
        }
        function.rename_uses(&|used| if used == param { value } else { used });
    }
}

fn drop_unread_params(function: &mut Function) {
    let counts = function.use_counts();
    for block in function.block_ids().collect::<Vec<_>>() {
        if block == function.entry {
            continue;
        }
        let params = function.block(block).params.clone();
        for (position, param) in params.iter().enumerate().rev() {
            if counts[param.index()] > 0 {
                continue;
            }
            function.block_mut(block).params.remove(position);
            for pred in function.block_ids().collect::<Vec<_>>() {
                for succ in function.block_mut(pred).term.successors_mut() {
                    if succ.block == block {
                        succ.args.remove(position);
                    }
                }
            }
        }
    }
}

fn merge_chains(function: &mut Function) {
    loop {
        let preds = function.predecessors();
        let candidate = function
            .block_ids()
            .find_map(|id| match &function.block(id).term {
                Terminator::Goto(succ)
                    if succ.block != id
                        && succ.block != function.entry
                        && preds[succ.block.index()].len() == 1 =>
                {
                    Some((id, succ.block))
                }
                _ => None,
            });
        let Some((first, second)) = candidate else {
            return;
        };
        let Terminator::Goto(succ) = function.block(first).term.clone() else {
            return;
        };
        let next = std::mem::take(function.block_mut(second));
        let renames: HashMap<ValueId, ValueId> =
            next.params.iter().copied().zip(succ.args).collect();
        let head = function.block_mut(first);
        head.stmts.extend(next.stmts);
        head.term = next.term;
        head.span = next.span;
        // The emptied block is left without predecessors and removed later.
        function.block_mut(second).term = Terminator::Unreachable;
        function.rename_uses(&|value| renames.get(&value).copied().unwrap_or(value));
    }
}

/// `x & m` of an unsigned `x` and a constant `m` of low bits is `x % (m + 1)`:
/// the solver decides remainders by constants in integer arithmetic, where
/// a bitwise `and` needs bit-vectors, which are slow for wide integers.
fn fold_masks(function: &mut Function) {
    let constants: HashMap<ValueId, Value> = function
        .blocks
        .iter()
        .flat_map(|block| &block.stmts)
        .filter_map(|stmt| match &stmt.kind {
            StmtKind::Def {
                dst,
                expr: Expr::Const(Const::Int(value)),
            } => Some((*dst, *value)),
            _ => None,
        })
        .collect();
    for block in 0..function.blocks.len() {
        let mut stmts = Vec::new();
        for stmt in std::mem::take(&mut function.blocks[block].stmts) {
            let StmtKind::Def {
                dst,
                expr:
                    Expr::Binary {
                        op: BinOp::BitAnd,
                        lhs,
                        rhs,
                    },
            } = &stmt.kind
            else {
                stmts.push(stmt);
                continue;
            };
            let ty = function.ty(*dst).clone();
            let (mask, other) = match (constants.get(lhs), constants.get(rhs)) {
                (_, Some(mask)) => (*mask, *lhs),
                (Some(mask), None) => (*mask, *rhs),
                (None, None) => {
                    stmts.push(stmt);
                    continue;
                }
            };
            let Some((bits, false)) = ty.int_shape() else {
                stmts.push(stmt);
                continue;
            };
            let shape = Ty::Uint(bits);
            let mask = mask.bits();
            let expr = if mask.is_zero() {
                Expr::Const(Const::Int(Value::from_ordinal(shape, U256::ZERO)))
            } else if mask == shape.max_ordinal() {
                Expr::Use(other)
            } else if (mask + U256::from(1u8)).is_power_of_two() {
                let divisor = function.new_value(ty.clone(), None);
                stmts.push(Stmt {
                    kind: StmtKind::Def {
                        dst: divisor,
                        expr: Expr::Const(Const::Int(Value::from_ordinal(
                            shape,
                            mask + U256::from(1u8),
                        ))),
                    },
                    span: stmt.span.clone(),
                });
                Expr::Binary {
                    op: BinOp::Rem(Arith::Exact),
                    lhs: other,
                    rhs: divisor,
                }
            } else {
                stmts.push(stmt);
                continue;
            };
            stmts.push(Stmt {
                kind: StmtKind::Def { dst: *dst, expr },
                span: stmt.span,
            });
        }
        function.blocks[block].stmts = stmts;
    }
}

/// Compares constants, and follows branches on a known condition, so that
/// code no execution reaches is not encoded.
fn fold_constants(function: &mut Function) {
    let mut ints: HashMap<ValueId, Value> = HashMap::new();
    let mut bools: HashMap<ValueId, bool> = HashMap::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            if let StmtKind::Def {
                dst,
                expr: Expr::Const(constant),
            } = &stmt.kind
            {
                match constant {
                    Const::Int(value) => {
                        ints.insert(*dst, *value);
                    }
                    Const::Bool(value) => {
                        bools.insert(*dst, *value);
                    }
                    Const::Unit => {}
                }
            }
        }
    }
    for block in &mut function.blocks {
        for stmt in &mut block.stmts {
            let StmtKind::Def {
                dst,
                expr: Expr::Binary { op, lhs, rhs },
            } = &stmt.kind
            else {
                continue;
            };
            let (Some(a), Some(b)) = (ints.get(lhs), ints.get(rhs)) else {
                continue;
            };
            let (a, b) = (a.ordinal(), b.ordinal());
            let result = match op {
                BinOp::Eq => a == b,
                BinOp::Ne => a != b,
                BinOp::Lt => a < b,
                BinOp::Le => a <= b,
                BinOp::Gt => a > b,
                BinOp::Ge => a >= b,
                _ => continue,
            };
            bools.insert(*dst, result);
            stmt.kind = StmtKind::Def {
                dst: *dst,
                expr: Expr::Const(Const::Bool(result)),
            };
        }
        if let Terminator::Branch {
            cond,
            then,
            otherwise,
        } = &block.term
            && let Some(known) = bools.get(cond)
        {
            block.term = Terminator::Goto(if *known {
                then.clone()
            } else {
                otherwise.clone()
            });
        }
    }
}

fn drop_trivial_checks(function: &mut Function) {
    let truths: HashSet<ValueId> = function
        .blocks
        .iter()
        .flat_map(|block| &block.stmts)
        .filter_map(|stmt| match &stmt.kind {
            StmtKind::Def {
                dst,
                expr: Expr::Const(Const::Bool(true)),
            } => Some(*dst),
            _ => None,
        })
        .collect();
    for block in &mut function.blocks {
        block.stmts.retain(|stmt| match &stmt.kind {
            StmtKind::Assert { cond, .. } | StmtKind::Assume(cond) => !truths.contains(cond),
            _ => true,
        });
    }
}

fn remove_unused(function: &mut Function, keep: &HashSet<ValueId>) {
    loop {
        let counts = function.use_counts();
        let mut removed = false;
        for block in &mut function.blocks {
            let before = block.stmts.len();
            block.stmts.retain(|stmt| match &stmt.kind {
                StmtKind::Def { dst, expr } => {
                    counts[dst.index()] > 0
                        || keep.contains(dst)
                        || matches!(expr, Expr::Call { .. })
                }
                _ => true,
            });
            removed |= block.stmts.len() != before;
        }
        if !removed {
            return;
        }
    }
}
