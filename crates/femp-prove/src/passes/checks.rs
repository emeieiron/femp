//! Makes every way the code can panic an assertion tagged with its reason:
//! checked arithmetic that leaves its type, division by zero, a signed
//! `MIN / -1`, an index past the end, a failed `assert`, and `core::panic`.
//! Specification statements left outside a spec get their runtime meaning.

use std::collections::HashMap;

use femp_core::property::{PanicKind, Ty, U256, Value};
use femp_vir::{
    Arith, BinOp, Check, Const, Cursor, Expr, Function, SpecOp, Stmt, StmtKind, Terminator, Type,
    UnOp, ValueId,
};

use super::Pass;
use crate::task::{NotSupported, Task};

pub struct Checks;

const MAX_EXPONENT: u32 = 256;

impl Pass for Checks {
    fn name(&self) -> &'static str {
        "checks"
    }

    fn run(&self, task: &mut Task) -> Result<(), NotSupported> {
        let function = task.entry_mut();
        let constants = constants(function);
        for block in function.block_ids().collect::<Vec<_>>() {
            let stmts = std::mem::take(&mut function.block_mut(block).stmts);
            let mut cursor = Cursor::new(function, block);
            for stmt in stmts {
                cursor.at(stmt.span.clone());
                rewrite(&mut cursor, stmt, &constants)?;
            }
            if let Terminator::Fail(kind) = &cursor.function.block(block).term {
                let kind = kind.clone();
                let span = cursor.function.block(block).span.clone();
                cursor.at(span);
                let never = cursor.boolean(false);
                cursor.assert(never, Check::Panic(kind));
                cursor.terminate(Terminator::Stop);
            }
        }
        Ok(())
    }
}

fn constants(function: &Function) -> HashMap<ValueId, Value> {
    let mut found = HashMap::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            if let StmtKind::Def {
                dst,
                expr: Expr::Const(Const::Int(value)),
            } = &stmt.kind
            {
                found.insert(*dst, *value);
            }
        }
    }
    found
}

fn int_ty(ty: &Type) -> Option<Ty> {
    let (bits, signed) = ty.int_shape()?;
    Some(if signed {
        Ty::Int(bits)
    } else {
        Ty::Uint(bits)
    })
}

fn min(ty: Ty) -> Value {
    Value::from_ordinal(ty, U256::ZERO)
}

fn max(ty: Ty) -> Value {
    Value::from_ordinal(ty, ty.max_ordinal())
}

fn rewrite(
    cursor: &mut Cursor<'_>,
    stmt: Stmt,
    constants: &HashMap<ValueId, Value>,
) -> Result<(), NotSupported> {
    match stmt.kind {
        StmtKind::Def {
            dst,
            expr: Expr::Binary { op, lhs, rhs },
        } if op.arith() == Some(Arith::Checked) => {
            let ty = cursor.function.ty(dst).clone();
            let Some(prim) = int_ty(&ty) else {
                cursor.stmt(StmtKind::Def {
                    dst,
                    expr: Expr::Binary { op, lhs, rhs },
                });
                return Ok(());
            };
            checked_binary(cursor, dst, op, lhs, rhs, prim, constants, &stmt.span)
        }
        StmtKind::Def {
            dst,
            expr:
                Expr::Binary {
                    op: BinOp::Pow(Arith::Wrapping),
                    lhs,
                    rhs,
                },
        } => {
            let exponent = exponent(constants, rhs, &stmt.span)?;
            power(cursor, dst, lhs, exponent, Arith::Wrapping, None);
            Ok(())
        }
        StmtKind::Def {
            dst,
            expr:
                Expr::Unary {
                    op: UnOp::Neg(Arith::Checked),
                    value,
                },
        } => {
            let ty = cursor.function.ty(dst).clone();
            if let Some(prim) = int_ty(&ty).filter(|prim| prim.is_signed()) {
                let lowest = cursor.int(&ty, min(prim));
                let fits = cursor.binary(BinOp::Ne, value, lowest);
                cursor.assert(fits, Check::Panic(PanicKind::Overflow));
            }
            cursor.stmt(StmtKind::Def {
                dst,
                expr: Expr::Unary {
                    op: UnOp::Neg(Arith::Exact),
                    value,
                },
            });
            Ok(())
        }
        StmtKind::CheckIndex { index, len } => {
            let ty = cursor.function.ty(index).clone();
            if let Some(prim) = int_ty(&ty)
                && let Some(bound) = Value::from_u128(prim, len as u128)
            {
                let bound = cursor.int(&ty, bound);
                let inside = cursor.binary(BinOp::Lt, index, bound);
                cursor.assert(inside, Check::Panic(PanicKind::Bounds));
            }
            Ok(())
        }
        StmtKind::Spec { op, cond } => {
            let kind = match op {
                SpecOp::Requires => PanicKind::Explicit,
                SpecOp::Ensures | SpecOp::Invariant => PanicKind::Assertion,
            };
            cursor.assert(cond, Check::Panic(kind));
            Ok(())
        }
        kind => {
            cursor.stmt(kind);
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn checked_binary(
    cursor: &mut Cursor<'_>,
    dst: ValueId,
    op: BinOp,
    lhs: ValueId,
    rhs: ValueId,
    prim: Ty,
    constants: &HashMap<ValueId, Value>,
    span: &femp_vir::Span,
) -> Result<(), NotSupported> {
    let ty = cursor.function.ty(dst).clone();
    match op {
        BinOp::Div(_) | BinOp::Rem(_) => {
            let zero = cursor.int(&ty, Value::from_bits(prim, U256::ZERO));
            let nonzero = cursor.binary(BinOp::Ne, rhs, zero);
            cursor.assert(nonzero, Check::Panic(PanicKind::DivisionByZero));
            if matches!(op, BinOp::Div(_)) && prim.is_signed() {
                let lowest = cursor.int(&ty, min(prim));
                let minus_one = cursor.int(&ty, Value::from_bits(prim, U256::MAX));
                let at_min = cursor.binary(BinOp::Eq, lhs, lowest);
                let by_minus_one = cursor.binary(BinOp::Eq, rhs, minus_one);
                let both = cursor.binary(BinOp::And, at_min, by_minus_one);
                let fits = cursor.not(both);
                cursor.assert(fits, Check::Panic(PanicKind::Overflow));
            }
            cursor.stmt(StmtKind::Def {
                dst,
                expr: Expr::Binary {
                    op: op.with_arith(Arith::Exact),
                    lhs,
                    rhs,
                },
            });
        }
        BinOp::Pow(_) => {
            let exponent = exponent(constants, rhs, span)?;
            power(cursor, dst, lhs, exponent, Arith::Exact, Some(prim));
        }
        _ => {
            cursor.stmt(StmtKind::Def {
                dst,
                expr: Expr::Binary {
                    op: op.with_arith(Arith::Exact),
                    lhs,
                    rhs,
                },
            });
            assert_fits(cursor, dst, prim);
        }
    }
    Ok(())
}

fn assert_fits(cursor: &mut Cursor<'_>, value: ValueId, prim: Ty) {
    let ty = cursor.function.ty(value).clone();
    let lowest = cursor.int(&ty, min(prim));
    let highest = cursor.int(&ty, max(prim));
    let above = cursor.binary(BinOp::Ge, value, lowest);
    let below = cursor.binary(BinOp::Le, value, highest);
    let fits = cursor.binary(BinOp::And, above, below);
    cursor.assert(fits, Check::Panic(PanicKind::Overflow));
}

fn exponent(
    constants: &HashMap<ValueId, Value>,
    value: ValueId,
    span: &femp_vir::Span,
) -> Result<u32, NotSupported> {
    constants
        .get(&value)
        .and_then(|value| {
            let bits = value.bits();
            (!value.is_negative() && bits <= U256::from(MAX_EXPONENT)).then(|| bits.to::<u32>())
        })
        .ok_or_else(|| {
            NotSupported::new(
                span.clone(),
                "a power with an exponent that is not a small constant",
            )
        })
}

/// `base` to a constant power as repeated multiplication. Checked powers
/// overflow exactly when some partial product does, since magnitudes only
/// grow.
fn power(
    cursor: &mut Cursor<'_>,
    dst: ValueId,
    base: ValueId,
    exponent: u32,
    arith: Arith,
    checked: Option<Ty>,
) {
    let ty = cursor.function.ty(dst).clone();
    let one = Value::from_bits(int_ty(&ty).unwrap_or(Ty::Uint(256)), U256::from(1u8));
    let mut acc = cursor.int(&ty, one);
    for _ in 0..exponent {
        acc = cursor.def(
            ty.clone(),
            Expr::Binary {
                op: BinOp::Mul(arith),
                lhs: acc,
                rhs: base,
            },
        );
        if let Some(prim) = checked {
            assert_fits(cursor, acc, prim);
        }
    }
    cursor.stmt(StmtKind::Def {
        dst,
        expr: Expr::Use(acc),
    });
}
