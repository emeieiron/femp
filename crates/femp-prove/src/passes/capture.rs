//! Turns a function's panics into a result: the captured function returns
//! `(panicked, value)` and never fails. Comparing two targets needs both
//! outcomes, so neither side's panic may end the execution.

use femp_vir::{BlockId, Check, Cursor, Expr, Function, StmtKind, Succ, Terminator, Type};

/// Rewrites `function`, which must already be call free with its panics
/// explicit, to return `(bool, T)` instead of `T`.
pub fn capture(function: &mut Function) {
    let result = function.result.clone();
    let wrapped = Type::Tuple(vec![Type::Bool, result.clone()]);
    let originals: Vec<BlockId> = function.block_ids().collect();

    let panicked = function.new_block();
    {
        let mut cursor = Cursor::new(function, panicked);
        let yes = cursor.boolean(true);
        let anything = cursor.havoc(result.clone(), None);
        let outcome = cursor.def(
            wrapped.clone(),
            Expr::Make {
                ty: wrapped.clone(),
                fields: vec![yes, anything],
            },
        );
        cursor.terminate(Terminator::Return(outcome));
    }

    let mut pending = originals.clone();
    while let Some(block) = pending.pop() {
        let position = function.block(block).stmts.iter().position(|stmt| {
            matches!(
                stmt.kind,
                StmtKind::Assert {
                    check: Check::Panic(_),
                    ..
                }
            )
        });
        let Some(position) = position else {
            continue;
        };
        let rest = function.new_block();
        let current = function.block_mut(block);
        let mut tail = current.stmts.split_off(position);
        let assertion = tail.remove(0);
        let StmtKind::Assert { cond, .. } = assertion.kind else {
            continue;
        };
        let term = std::mem::replace(
            &mut current.term,
            Terminator::Branch {
                cond,
                then: Succ::to(rest),
                otherwise: Succ::to(panicked),
            },
        );
        let span = std::mem::replace(&mut current.span, assertion.span);
        let continued = function.block_mut(rest);
        continued.stmts = tail;
        continued.term = term;
        continued.span = span;
        pending.push(rest);
    }

    for block in function.block_ids().collect::<Vec<_>>() {
        if block == panicked {
            continue;
        }
        let Terminator::Return(value) = function.block(block).term else {
            continue;
        };
        let mut cursor = Cursor::new(function, block);
        let no = cursor.boolean(false);
        let outcome = cursor.def(
            wrapped.clone(),
            Expr::Make {
                ty: wrapped.clone(),
                fields: vec![no, value],
            },
        );
        cursor.terminate(Terminator::Return(outcome));
    }
    function.result = wrapped;
}
