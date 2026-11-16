//! Removes loops, innermost first, so the task becomes acyclic.
//!
//! A loop whose body states `invariant(..)` is cut at its head: the
//! invariant must hold when the loop starts; then the loop's state is
//! replaced by an arbitrary one satisfying it, one iteration runs, and the
//! invariant must hold again where the iteration would go back to the head.
//! The invariant is evaluated at the head from the loop's state, by copying
//! the code that computes it there.
//!
//! A loop without one is unrolled a fixed number of times. Going back to
//! the head once more is a failed completeness check, so a proof never
//! silently ignores longer runs.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use femp_core::property::PanicKind;
use femp_vir::{
    BlockId, Check, Cursor, Expr, Function, Mode, Span, SpecOp, StmtKind, Succ, Terminator, Type,
    ValueId,
};

use super::Pass;
use crate::task::{NotSupported, Task};

pub struct Loops {
    pub unroll: usize,
}

impl Pass for Loops {
    fn name(&self) -> &'static str {
        "loops"
    }

    fn run(&self, task: &mut Task) -> Result<(), NotSupported> {
        let function = task.entry_mut();
        while let Some(found) = innermost(function)? {
            let invariants = invariants(function, &found);
            if invariants.is_empty() {
                // One copy more than iterations: the last copy's head may
                // still leave the loop, and only its back edge is too far.
                unroll(function, &found, self.unroll.max(1) + 1);
            } else {
                cut(function, &found, &invariants)?;
            }
        }
        // Invariants outside any loop keep their run-time meaning.
        for block in &mut function.blocks {
            for stmt in &mut block.stmts {
                if let StmtKind::Spec {
                    op: SpecOp::Invariant,
                    cond,
                } = stmt.kind
                {
                    stmt.kind = StmtKind::Assert {
                        cond,
                        check: Check::Panic(PanicKind::Assertion),
                    };
                }
            }
        }
        Ok(())
    }
}

struct Loop {
    header: BlockId,
    latches: Vec<BlockId>,
    body: BTreeSet<BlockId>,
}

fn dominators(function: &Function, order: &[BlockId]) -> HashMap<BlockId, BlockId> {
    let position: HashMap<BlockId, usize> = order
        .iter()
        .enumerate()
        .map(|(index, block)| (*block, index))
        .collect();
    let preds = function.predecessors();
    let mut idom: HashMap<BlockId, BlockId> = HashMap::new();
    idom.insert(function.entry, function.entry);
    let intersect = |idom: &HashMap<BlockId, BlockId>, mut a: BlockId, mut b: BlockId| {
        while a != b {
            while position[&a] > position[&b] {
                a = idom[&a];
            }
            while position[&b] > position[&a] {
                b = idom[&b];
            }
        }
        a
    };
    let mut changed = true;
    while changed {
        changed = false;
        for block in order.iter().skip(1) {
            let mut chosen: Option<BlockId> = None;
            for pred in &preds[block.index()] {
                if !idom.contains_key(pred) {
                    continue;
                }
                chosen = Some(match chosen {
                    None => *pred,
                    Some(current) => intersect(&idom, *pred, current),
                });
            }
            if let Some(chosen) = chosen
                && idom.get(block) != Some(&chosen)
            {
                idom.insert(*block, chosen);
                changed = true;
            }
        }
    }
    idom
}

fn dominates(idom: &HashMap<BlockId, BlockId>, a: BlockId, mut b: BlockId) -> bool {
    loop {
        if a == b {
            return true;
        }
        match idom.get(&b) {
            Some(parent) if *parent != b => b = *parent,
            _ => return false,
        }
    }
}

/// A loop that contains no other loop, if any is left.
fn innermost(function: &Function) -> Result<Option<Loop>, NotSupported> {
    let order = function.reverse_postorder();
    let position: HashMap<BlockId, usize> = order
        .iter()
        .enumerate()
        .map(|(index, block)| (*block, index))
        .collect();
    let idom = dominators(function, &order);
    let mut latches: BTreeMap<BlockId, Vec<BlockId>> = BTreeMap::new();
    for block in &order {
        for succ in function.block(*block).term.successors() {
            if position[&succ.block] <= position[block] {
                if !dominates(&idom, succ.block, *block) {
                    let span = function.block(succ.block).span.clone();
                    return Err(NotSupported::new(
                        span,
                        "a loop with several entries, which the prover cannot cut",
                    ));
                }
                latches.entry(succ.block).or_default().push(*block);
            }
        }
    }
    let preds = function.predecessors();
    let mut loops: Vec<Loop> = latches
        .into_iter()
        .map(|(header, latches)| {
            let mut body = BTreeSet::from([header]);
            let mut pending = latches.clone();
            while let Some(block) = pending.pop() {
                if body.insert(block) {
                    pending.extend(preds[block.index()].iter().copied());
                }
            }
            Loop {
                header,
                latches,
                body,
            }
        })
        .collect();
    let headers: Vec<BlockId> = loops.iter().map(|found| found.header).collect();
    let inner = loops.iter().position(|found| {
        headers
            .iter()
            .all(|header| *header == found.header || !found.body.contains(header))
    });
    Ok(inner.map(|index| loops.swap_remove(index)))
}

struct Invariant {
    block: BlockId,
    index: usize,
    cond: ValueId,
    span: Span,
}

fn invariants(function: &Function, found: &Loop) -> Vec<Invariant> {
    let mut list = Vec::new();
    for block in &found.body {
        for (index, stmt) in function.block(*block).stmts.iter().enumerate() {
            if let StmtKind::Spec {
                op: SpecOp::Invariant,
                cond,
            } = stmt.kind
            {
                list.push(Invariant {
                    block: *block,
                    index,
                    cond,
                    span: stmt.span.clone(),
                });
            }
        }
    }
    list
}

/// The definitions inside the loop that compute `value`, dependencies
/// first, so they can be repeated where the loop's state is known.
fn slice(
    function: &Function,
    found: &Loop,
    value: ValueId,
    span: &Span,
) -> Result<Vec<(ValueId, Expr)>, NotSupported> {
    let mut defs: HashMap<ValueId, Expr> = HashMap::new();
    let mut params: BTreeSet<ValueId> = BTreeSet::new();
    for block in &found.body {
        let data = function.block(*block);
        if *block != found.header {
            params.extend(data.params.iter().copied());
        }
        for stmt in &data.stmts {
            if let StmtKind::Def { dst, expr } = &stmt.kind {
                defs.insert(*dst, expr.clone());
            }
        }
    }
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![(value, false)];
    while let Some((current, expanded)) = stack.pop() {
        if params.contains(&current) {
            return Err(NotSupported::new(
                span.clone(),
                "a loop invariant must depend only on the loop's state",
            ));
        }
        let Some(expr) = defs.get(&current) else {
            continue;
        };
        if expanded {
            out.push((current, expr.clone()));
            continue;
        }
        if !seen.insert(current) {
            continue;
        }
        if matches!(expr, Expr::Havoc | Expr::Call { .. } | Expr::Load { .. }) {
            return Err(NotSupported::new(
                span.clone(),
                "a loop invariant must be computed from the loop's state alone",
            ));
        }
        stack.push((current, true));
        for operand in expr.operands() {
            stack.push((*operand, false));
        }
    }
    Ok(out)
}

/// Repeats the computation of each invariant with the header's parameters
/// replaced by `state`, and returns the conditions.
fn evaluate(
    cursor: &mut Cursor<'_>,
    header_params: &[ValueId],
    state: &[ValueId],
    slices: &[(Vec<(ValueId, Expr)>, ValueId)],
) -> Vec<ValueId> {
    let mut conditions = Vec::new();
    for (slice, cond) in slices {
        let mut renamed: HashMap<ValueId, ValueId> = header_params
            .iter()
            .copied()
            .zip(state.iter().copied())
            .collect();
        for (dst, expr) in slice {
            let mut copy = expr.clone();
            for operand in copy.operands_mut() {
                if let Some(new) = renamed.get(operand) {
                    *operand = *new;
                }
            }
            let ty = cursor.function.ty(*dst).clone();
            let value = cursor.def(ty, copy);
            renamed.insert(*dst, value);
        }
        conditions.push(renamed.get(cond).copied().unwrap_or(*cond));
    }
    conditions
}

fn check(cursor: &mut Cursor<'_>, cond: ValueId, mode: Mode, text: &str) {
    if mode == Mode::Checked {
        cursor.assert(cond, Check::Claim(text.to_string()));
    } else {
        cursor.assume(cond);
    }
}

fn fresh_params(function: &mut Function, like: &[ValueId]) -> Vec<ValueId> {
    like.iter()
        .map(|value| {
            let info = function.value(*value).clone();
            function.new_value(info.ty, info.name)
        })
        .collect()
}

fn cut(
    function: &mut Function,
    found: &Loop,
    invariants: &[Invariant],
) -> Result<(), NotSupported> {
    let header = found.header;
    let params = function.block(header).params.clone();
    let mode = function.block(header).mode;
    let mut slices = Vec::new();
    for invariant in invariants {
        slices.push((
            slice(function, found, invariant.cond, &invariant.span)?,
            invariant.cond,
        ));
    }
    let span = invariants[0].span.clone();

    // On entry: the invariant holds of the state the loop starts with.
    let entry = function.new_block();
    let entry_params = fresh_params(function, &params);
    function.block_mut(entry).params = entry_params.clone();
    function.block_mut(entry).mode = mode;
    let arbitrary = function.new_block();
    function.block_mut(arbitrary).mode = mode;
    for pred in function.block_ids().collect::<Vec<_>>() {
        if found.body.contains(&pred) {
            continue;
        }
        for succ in function.block_mut(pred).term.successors_mut() {
            if succ.block == header {
                succ.block = entry;
            }
        }
    }
    {
        let mut cursor = Cursor::new(function, entry);
        cursor.at(span.clone());
        for cond in evaluate(&mut cursor, &params, &entry_params, &slices) {
            check(
                &mut cursor,
                cond,
                mode,
                "the loop invariant does not hold when the loop starts",
            );
        }
        cursor.terminate(Terminator::Goto(Succ::to(arbitrary)));
    }

    // Any state the invariant allows, for one more iteration.
    {
        let mut cursor = Cursor::new(function, arbitrary);
        cursor.at(span.clone());
        let state: Vec<ValueId> = params
            .iter()
            .map(|param| {
                let info = cursor.function.value(*param).clone();
                cursor.havoc(info.ty, info.name)
            })
            .collect();
        for cond in evaluate(&mut cursor, &params, &state, &slices) {
            cursor.assume(cond);
        }
        cursor.terminate(Terminator::Goto(Succ {
            block: header,
            args: state,
        }));
    }

    // Where the iteration would go back, the invariant holds again.
    for latch in &found.latches {
        let back = function.new_block();
        let back_params = fresh_params(function, &params);
        function.block_mut(back).params = back_params.clone();
        function.block_mut(back).mode = mode;
        for succ in function.block_mut(*latch).term.successors_mut() {
            if succ.block == header {
                succ.block = back;
            }
        }
        let mut cursor = Cursor::new(function, back);
        cursor.at(span.clone());
        for cond in evaluate(&mut cursor, &params, &back_params, &slices) {
            check(
                &mut cursor,
                cond,
                mode,
                "an iteration does not keep the loop invariant",
            );
        }
        cursor.terminate(Terminator::Stop);
    }

    let mut removed: Vec<(BlockId, usize)> = invariants
        .iter()
        .map(|invariant| (invariant.block, invariant.index))
        .collect();
    removed.sort_unstable_by(|a, b| b.cmp(a));
    for (block, index) in removed {
        function.block_mut(block).stmts.remove(index);
    }
    Ok(())
}

fn unroll(function: &mut Function, found: &Loop, times: usize) {
    let header = found.header;
    let mode = function.block(header).mode;
    let defined: Vec<ValueId> = found
        .body
        .iter()
        .flat_map(|block| {
            let data = function.block(*block);
            data.params
                .iter()
                .copied()
                .chain(data.stmts.iter().filter_map(|stmt| stmt.kind.defined()))
                .collect::<Vec<_>>()
        })
        .collect();

    // Copy 0 is the original; copies 1.. follow it, each entered where the
    // previous one went back to the head.
    let mut copies: Vec<HashMap<BlockId, BlockId>> =
        vec![found.body.iter().map(|block| (*block, *block)).collect()];
    let mut values: Vec<HashMap<ValueId, ValueId>> =
        vec![defined.iter().map(|value| (*value, *value)).collect()];
    for _ in 1..times {
        let blocks: HashMap<BlockId, BlockId> = found
            .body
            .iter()
            .map(|block| (*block, function.new_block()))
            .collect();
        let renamed: HashMap<ValueId, ValueId> = defined
            .iter()
            .map(|value| {
                let info = function.value(*value).clone();
                (*value, function.new_value(info.ty, info.name))
            })
            .collect();
        for (old, new) in &blocks {
            let mut copy = function.block(*old).clone();
            copy.params = copy.params.iter().map(|p| renamed[p]).collect();
            for stmt in &mut copy.stmts {
                for operand in stmt.kind.operands_mut() {
                    if let Some(value) = renamed.get(operand) {
                        *operand = *value;
                    }
                }
                if let StmtKind::Def { dst, .. } = &mut stmt.kind {
                    *dst = renamed[dst];
                }
            }
            for operand in copy.term.operands_mut() {
                if let Some(value) = renamed.get(operand) {
                    *operand = *value;
                }
            }
            for succ in copy.term.successors_mut() {
                if let Some(target) = blocks.get(&succ.block) {
                    succ.block = *target;
                }
            }
            *function.block_mut(*new) = copy;
        }
        copies.push(blocks);
        values.push(renamed);
    }

    let unwound = function.new_block();
    let unwound_params = fresh_params(function, &function.block(header).params.clone());
    function.block_mut(unwound).params = unwound_params;
    function.block_mut(unwound).mode = mode;
    {
        let span = function.block(header).span.clone();
        let mut cursor = Cursor::new(function, unwound);
        cursor.at(span);
        if mode == Mode::Checked {
            let never = cursor.boolean(false);
            cursor.assert(never, Check::Unwind);
        }
        cursor.terminate(Terminator::Stop);
    }
    for (index, copy) in copies.iter().enumerate() {
        let next = copies
            .get(index + 1)
            .map_or(unwound, |following| following[&header]);
        for latch in &found.latches {
            for succ in function.block_mut(copy[latch]).term.successors_mut() {
                if succ.block == copy[&header] {
                    succ.block = next;
                }
            }
        }
    }

    repair(function, &copies, &values, &defined);
}

/// Values defined in the loop and used after it now have one definition
/// per copy; uses outside the loop read the copy they came from, through
/// block parameters where copies meet.
fn repair(
    function: &mut Function,
    copies: &[HashMap<BlockId, BlockId>],
    values: &[HashMap<ValueId, ValueId>],
    defined: &[ValueId],
) {
    let inside: HashMap<BlockId, usize> = copies
        .iter()
        .enumerate()
        .flat_map(|(index, copy)| copy.values().map(move |block| (*block, index)))
        .collect();
    let defined: BTreeSet<ValueId> = defined.iter().copied().collect();
    let outside: Vec<BlockId> = function
        .reverse_postorder()
        .into_iter()
        .filter(|block| !inside.contains_key(block))
        .collect();
    let mut repairer = Repair {
        inside: &inside,
        values,
        starts: HashMap::new(),
    };
    for block in outside {
        let used: BTreeSet<ValueId> = {
            let data = function.block(block);
            data.stmts
                .iter()
                .flat_map(|stmt| stmt.kind.operands())
                .chain(data.term.operands())
                .copied()
                .filter(|value| defined.contains(value))
                .collect()
        };
        for value in used {
            let reached = repairer.start(function, block, value);
            rename_in_block(function, block, value, reached);
        }
    }
}

fn rename_in_block(function: &mut Function, block: BlockId, from: ValueId, to: ValueId) {
    let data = function.block_mut(block);
    for stmt in &mut data.stmts {
        for operand in stmt.kind.operands_mut() {
            if *operand == from {
                *operand = to;
            }
        }
    }
    for operand in data.term.operands_mut() {
        if *operand == from {
            *operand = to;
        }
    }
}

struct Repair<'a> {
    inside: &'a HashMap<BlockId, usize>,
    values: &'a [HashMap<ValueId, ValueId>],
    starts: HashMap<(BlockId, ValueId), ValueId>,
}

impl Repair<'_> {
    fn end(&mut self, function: &mut Function, block: BlockId, value: ValueId) -> ValueId {
        match self.inside.get(&block) {
            Some(copy) => self.values[*copy][&value],
            None => self.start(function, block, value),
        }
    }

    fn start(&mut self, function: &mut Function, block: BlockId, value: ValueId) -> ValueId {
        if let Some(found) = self.starts.get(&(block, value)) {
            return *found;
        }
        let preds: Vec<BlockId> = function.predecessors()[block.index()].clone();
        let mut distinct = preds.clone();
        distinct.dedup();
        if preds.is_empty() {
            return value;
        }
        if preds.len() == 1 {
            let reached = self.end(function, preds[0], value);
            self.starts.insert((block, value), reached);
            return reached;
        }
        let ty: Type = function.ty(value).clone();
        let param = function.new_value(ty, function.value(value).name.clone());
        function.block_mut(block).params.push(param);
        self.starts.insert((block, value), param);
        for pred in distinct {
            let reached = self.end(function, pred, value);
            for succ in function.block_mut(pred).term.successors_mut() {
                if succ.block == block {
                    succ.args.push(reached);
                }
            }
        }
        param
    }
}
