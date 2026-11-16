//! Cleans the entry up before encoding: drops blocks nothing reaches,
//! forwards copies, removes block parameters every edge agrees on, merges
//! straight-line chains, and deletes assertions of `true` and unused
//! definitions. The task's inputs are kept, since their values are reported.

use std::collections::{HashMap, HashSet};

use femp_vir::{BlockId, Const, Expr, Function, StmtKind, Terminator, ValueId};

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
