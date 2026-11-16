//! Integer arithmetic over SMT integers: literals and bounds of each type,
//! wrapping by `mod`, division that truncates toward zero, and bitwise
//! operations through bit-vector round trips.

use std::collections::BTreeSet;
use std::fmt::Write;

use femp_core::property::{U256, Value};

/// 2^256, which no `U256` holds.
const TWO_TO_256: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639936";

pub fn pow2(bits: u16) -> String {
    if bits >= 256 {
        TWO_TO_256.to_string()
    } else {
        (U256::from(1u8) << bits as usize).to_string()
    }
}

fn pow2_minus_one(bits: u16) -> String {
    if bits >= 256 {
        U256::MAX.to_string()
    } else {
        ((U256::from(1u8) << bits as usize) - U256::from(1u8)).to_string()
    }
}

/// The smallest and largest value of an integer type, as SMT terms.
pub fn bounds(bits: u16, signed: bool) -> (String, String) {
    if signed {
        let half = pow2(bits - 1);
        let high = ((U256::from(1u8) << (bits as usize - 1)) - U256::from(1u8)).to_string();
        (format!("-{half}"), high)
    } else {
        ("0".to_string(), pow2_minus_one(bits))
    }
}

/// A decimal number as an SMT term, negative ones as `(- n)`.
pub fn literal(text: &str) -> String {
    match text.strip_prefix('-') {
        Some(magnitude) => format!("(- {magnitude})"),
        None => text.to_string(),
    }
}

pub fn value(value: &Value) -> String {
    match value.as_bool() {
        Some(flag) => flag.to_string(),
        None => literal(&value.to_string()),
    }
}

pub fn suffix(bits: u16, signed: bool) -> String {
    format!("{}{bits}", if signed { 'i' } else { 'u' })
}

pub fn wrap(bits: u16, signed: bool, term: &str) -> String {
    format!("(wrap.{} {term})", suffix(bits, signed))
}

pub fn to_bits(bits: u16, term: &str) -> String {
    format!("((_ int2bv {bits}) {term})")
}

pub fn from_bits(bits: u16, signed: bool, term: &str) -> String {
    let unsigned = format!("(bv2nat {term})");
    if signed {
        wrap(bits, true, &unsigned)
    } else {
        unsigned
    }
}

/// The helper functions the encoded task uses: truncating division and
/// remainder, and wrapping into each integer type that occurs.
pub fn prelude(types: &BTreeSet<(u16, bool)>) -> String {
    let mut out = String::new();
    out.push_str(
        "(define-fun tdiv ((a Int) (b Int)) Int (ite (>= a 0) (ite (>= b 0) (div a b) (- (div a (- b)))) (ite (>= b 0) (- (div (- a) b)) (div (- a) (- b)))))\n",
    );
    out.push_str("(define-fun trem ((a Int) (b Int)) Int (- a (* b (tdiv a b))))\n");
    for (bits, signed) in types {
        let name = suffix(*bits, *signed);
        let modulus = pow2(*bits);
        if *signed {
            let half = pow2(bits - 1);
            let _ = writeln!(
                out,
                "(define-fun wrap.{name} ((x Int)) Int (- (mod (+ x {half}) {modulus}) {half}))"
            );
        } else {
            let _ = writeln!(
                out,
                "(define-fun wrap.{name} ((x Int)) Int (mod x {modulus}))"
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_cover_each_type() {
        assert_eq!(bounds(8, false), ("0".into(), "255".into()));
        assert_eq!(bounds(8, true), ("-128".into(), "127".into()));
        assert_eq!(bounds(256, false).1, U256::MAX.to_string());
        assert_eq!(literal("-128"), "(- 128)");
        assert_eq!(pow2(256), TWO_TO_256);
    }
}
