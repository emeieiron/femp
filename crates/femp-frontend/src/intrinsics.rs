//! Models of the `extern` functions of the core library that pure code
//! reaches. The integer operations of `core::num` are named `__op_type`;
//! checked and saturating variants are generic.

use femp_vir::{Arith, BinOp, Intrinsic, UnOp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Model {
    Binary(BinOp),
    Unary(UnOp),
    Intrinsic(Intrinsic),
    /// Returns its argument.
    Identity,
    /// Never returns.
    Panic,
}

pub(crate) fn model(name: &str) -> Option<Model> {
    match name {
        "__bitcast" => return Some(Model::Intrinsic(Intrinsic::Bitcast)),
        "__saturating_add" => return Some(Model::Intrinsic(Intrinsic::SaturatingAdd)),
        "__saturating_sub" => return Some(Model::Intrinsic(Intrinsic::SaturatingSub)),
        "__saturating_mul" => return Some(Model::Intrinsic(Intrinsic::SaturatingMul)),
        "panic" | "panic_with_value" | "panic_code" | "todo" => return Some(Model::Panic),
        "black_box" => return Some(Model::Identity),
        _ => {}
    }
    if let Some(op) = name.strip_prefix("__checked_") {
        return operation(op, Arith::Checked);
    }
    let rest = name.strip_prefix("__")?;
    let (op, ty) = rest.rsplit_once('_')?;
    is_int_name(ty).then(|| operation(op, Arith::Wrapping))?
}

fn is_int_name(ty: &str) -> bool {
    matches!(
        ty,
        "u8" | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "u256"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "i256"
            | "isize"
    )
}

fn operation(op: &str, arith: Arith) -> Option<Model> {
    Some(match op {
        "add" => Model::Binary(BinOp::Add(arith)),
        "sub" => Model::Binary(BinOp::Sub(arith)),
        "mul" => Model::Binary(BinOp::Mul(arith)),
        // Division by zero is modeled as a panic on every target.
        "div" => Model::Binary(BinOp::Div(Arith::Checked)),
        "rem" => Model::Binary(BinOp::Rem(Arith::Checked)),
        "pow" => Model::Binary(BinOp::Pow(arith)),
        "neg" => Model::Unary(UnOp::Neg(arith)),
        "shl" => Model::Binary(BinOp::Shl),
        "shr" => Model::Binary(BinOp::Shr),
        "bitand" => Model::Binary(BinOp::BitAnd),
        "bitor" => Model::Binary(BinOp::BitOr),
        "bitxor" => Model::Binary(BinOp::BitXor),
        "bitnot" => Model::Unary(UnOp::BitNot),
        "eq" => Model::Binary(BinOp::Eq),
        "ne" => Model::Binary(BinOp::Ne),
        "lt" => Model::Binary(BinOp::Lt),
        "le" => Model::Binary(BinOp::Le),
        "gt" => Model::Binary(BinOp::Gt),
        "ge" => Model::Binary(BinOp::Ge),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_the_core_externs() {
        assert_eq!(
            model("__add_usize"),
            Some(Model::Binary(BinOp::Add(Arith::Wrapping)))
        );
        assert_eq!(
            model("__checked_mul"),
            Some(Model::Binary(BinOp::Mul(Arith::Checked)))
        );
        assert_eq!(model("__bitnot_i8"), Some(Model::Unary(UnOp::BitNot)));
        assert_eq!(
            model("__bitcast"),
            Some(Model::Intrinsic(Intrinsic::Bitcast))
        );
        assert_eq!(model("panic"), Some(Model::Panic));
        assert_eq!(model("__add_string"), None);
        assert_eq!(model("leading_zeros"), None);
    }
}
