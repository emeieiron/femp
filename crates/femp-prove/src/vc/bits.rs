//! Shifts and bitwise operations in integer arithmetic. A shift by a known
//! amount multiplies or divides by a power of two, and a bitwise operation
//! with a known operand splits into the runs of ones of that operand, each
//! a division and a remainder by powers of two. Both are exact and stay in
//! the linear arithmetic the solver decides well, where a round trip
//! through 256-bit vectors does not. Operations on two unknown values keep
//! the round trip, with bounds the solver can use before it.

use femp_core::property::U256;
use femp_vir::BinOp;

use super::ints;

/// The most runs of ones a known operand may have before its operation
/// keeps the round trip through bit vectors.
const MAX_RUNS: usize = 16;

/// `x << amount` or `x >> amount` for a known amount, or `None` for
/// another operation.
pub fn shift(op: BinOp, bits: u16, signed: bool, x: &str, amount: U256) -> Option<String> {
    let past = amount >= U256::from(bits);
    Some(match op {
        BinOp::Shl if past => "0".to_string(),
        BinOp::Shl => {
            let scaled = format!("(* {x} {})", ints::pow2(amount.to::<u16>()));
            ints::wrap(bits, signed, &scaled)
        }
        // A signed value shifted out entirely keeps only its sign.
        BinOp::Shr if past && signed => format!("(ite (< {x} 0) (- 1) 0)"),
        BinOp::Shr if past => "0".to_string(),
        // `div` rounds toward negative infinity for a positive divisor,
        // which is an arithmetic shift for signed values.
        BinOp::Shr => format!("(div {x} {})", ints::pow2(amount.to::<u16>())),
        _ => return None,
    })
}

/// `x & known`, `x | known` or `x ^ known`, where `known` is the bit
/// pattern of the other operand in the type's width.
pub fn with_known(op: BinOp, bits: u16, signed: bool, x: &str, known: U256) -> Option<String> {
    let known = pattern(bits, known);
    let runs = runs(bits, known);
    if runs.len() > MAX_RUNS {
        return None;
    }
    let unsigned = if signed {
        format!("(mod {x} {})", ints::pow2(bits))
    } else {
        x.to_string()
    };
    let and = and(&unsigned, &runs);
    let result = match op {
        BinOp::BitAnd => and,
        BinOp::BitOr => format!("(- (+ {unsigned} {known}) {and})"),
        BinOp::BitXor => format!("(- (+ {unsigned} {known}) (* 2 {and}))"),
        _ => return None,
    };
    Some(if signed {
        ints::wrap(bits, true, &result)
    } else {
        result
    })
}

/// Facts about `result = a op b` for unsigned values that are not both
/// known, which hold of every bit pattern.
pub fn bounds(op: BinOp, result: &str, a: &str, b: &str) -> Vec<String> {
    match op {
        BinOp::BitAnd => vec![format!("(<= {result} {a})"), format!("(<= {result} {b})")],
        BinOp::BitOr => vec![
            format!("(>= {result} {a})"),
            format!("(>= {result} {b})"),
            format!("(<= {result} (+ {a} {b}))"),
        ],
        BinOp::BitXor => vec![format!("(<= {result} (+ {a} {b}))")],
        BinOp::Shr => vec![format!("(<= {result} {a})")],
        _ => Vec::new(),
    }
}

/// `result = a op b` where `a`'s low `zeros` bits are zero, as for a value
/// shifted left by `zeros`: where `b` fits below them, the two share no
/// bit, so `|` and `^` add and `&` gives zero.
pub fn disjoint(op: BinOp, result: &str, b: &str, a: &str, zeros: u16) -> Option<String> {
    let combined = match op {
        BinOp::BitOr | BinOp::BitXor => format!("(+ {a} {b})"),
        BinOp::BitAnd => "0".to_string(),
        _ => return None,
    };
    Some(format!(
        "(=> (< {b} {}) (= {result} {combined}))",
        ints::pow2(zeros)
    ))
}

/// `value`'s low `bits` bits.
fn pattern(bits: u16, value: U256) -> U256 {
    if bits >= 256 {
        value
    } else {
        value & ((U256::from(1u8) << bits as usize) - U256::from(1u8))
    }
}

/// The runs of ones in `known`, as their lowest bit and length.
fn runs(bits: u16, known: U256) -> Vec<(u16, u16)> {
    let mut runs = Vec::new();
    let mut bit = 0u16;
    while bit < bits {
        if !known.bit(bit as usize) {
            bit += 1;
            continue;
        }
        let start = bit;
        while bit < bits && known.bit(bit as usize) {
            bit += 1;
        }
        runs.push((start, bit - start));
    }
    runs
}

/// The bits of `x` in `runs`: for each run, the digits of `x` in base
/// `2^len` at position `start`.
fn and(x: &str, runs: &[(u16, u16)]) -> String {
    let parts: Vec<String> = runs
        .iter()
        .map(|(start, len)| {
            let shifted = if *start == 0 {
                x.to_string()
            } else {
                format!("(div {x} {})", ints::pow2(*start))
            };
            let digits = format!("(mod {shifted} {})", ints::pow2(*len));
            if *start == 0 {
                digits
            } else {
                format!("(* {digits} {})", ints::pow2(*start))
            }
        })
        .collect();
    match parts.as_slice() {
        [] => "0".to_string(),
        [one] => one.clone(),
        _ => format!("(+ {})", parts.join(" ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_runs_of_a_mask() {
        assert_eq!(runs(16, U256::from(0xff00u32)), [(8, 8)]);
        assert_eq!(
            runs(8, U256::from(0b1011_0001u32)),
            [(0, 1), (4, 2), (7, 1)]
        );
        assert_eq!(runs(8, U256::ZERO), []);
        // Only the type's own bits count.
        assert_eq!(runs(8, U256::from(0x1ffu32)), [(0, 8)]);
    }

    #[test]
    fn known_operands_become_arithmetic() {
        assert_eq!(
            with_known(BinOp::BitAnd, 16, false, "x", U256::from(0xff00u32)).unwrap(),
            "(* (mod (div x 256) 256) 256)"
        );
        assert_eq!(
            with_known(BinOp::BitOr, 8, false, "x", U256::from(1u8)).unwrap(),
            "(- (+ x 1) (mod x 2))"
        );
        let alternating = U256::from_str_radix(&"55".repeat(32), 16).unwrap();
        assert!(with_known(BinOp::BitAnd, 256, false, "x", alternating).is_none());
    }

    #[test]
    fn known_shifts_scale_by_powers_of_two() {
        assert_eq!(
            shift(BinOp::Shr, 256, false, "x", U256::from(1u8)).unwrap(),
            "(div x 2)"
        );
        assert_eq!(
            shift(BinOp::Shl, 8, false, "x", U256::from(4u8)).unwrap(),
            "(wrap.u8 (* x 16))"
        );
        assert_eq!(
            shift(BinOp::Shr, 8, false, "x", U256::from(9u8)).unwrap(),
            "0"
        );
        assert_eq!(
            shift(BinOp::Shr, 8, true, "x", U256::from(8u8)).unwrap(),
            "(ite (< x 0) (- 1) 0)"
        );
    }
}
