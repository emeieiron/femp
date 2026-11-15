use std::cmp::Ordering;
use std::fmt;

use ruint::aliases::U256;

use super::ty::Ty;

/// A value of a primitive type. Integers keep their two's-complement bits
/// truncated to the type's width, so every value has exactly one
/// representation and equality is structural.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value {
    ty: Ty,
    bits: U256,
}

impl Value {
    pub fn boolean(value: bool) -> Self {
        Self {
            ty: Ty::Bool,
            bits: U256::from(value as u8),
        }
    }

    pub fn from_bits(ty: Ty, bits: U256) -> Self {
        Self {
            ty,
            bits: bits & ty.mask(),
        }
    }

    pub fn from_u128(ty: Ty, value: u128) -> Option<Self> {
        let bits = U256::from(value);
        (bits <= largest(ty)).then(|| Self::from_bits(ty, bits))
    }

    pub fn from_i128(ty: Ty, value: i128) -> Option<Self> {
        if value >= 0 {
            return Self::from_u128(ty, value as u128);
        }
        if !ty.is_signed() {
            return None;
        }
        let magnitude = U256::from(value.unsigned_abs());
        (magnitude <= ty.zero_ordinal())
            .then(|| Self::from_bits(ty, U256::ZERO.wrapping_sub(magnitude)))
    }

    /// Numbers values from the smallest: unsigned values are their own
    /// ordinal, and signed ones are offset so that the minimum is 0.
    pub fn from_ordinal(ty: Ty, ordinal: U256) -> Self {
        Self::from_bits(ty, ordinal.wrapping_add(ty.zero_ordinal()))
    }

    pub fn ordinal(&self) -> U256 {
        self.bits.wrapping_add(self.ty.zero_ordinal()) & self.ty.mask()
    }

    pub fn ty(&self) -> Ty {
        self.ty
    }

    pub fn bits(&self) -> U256 {
        self.bits
    }

    pub fn as_bool(&self) -> Option<bool> {
        (self.ty == Ty::Bool).then(|| self.bits == U256::from(1u8))
    }

    pub fn is_negative(&self) -> bool {
        self.ty.is_signed() && self.bits >= self.ty.zero_ordinal()
    }

    /// Parses `true`/`false` for `bool` and decimal (or `0x` hex) integers,
    /// with a leading `-` for negative signed values.
    pub fn parse(ty: Ty, text: &str) -> Option<Self> {
        let text = text.trim();
        if ty == Ty::Bool {
            return match text {
                "true" | "1" => Some(Self::boolean(true)),
                "false" | "0" => Some(Self::boolean(false)),
                _ => None,
            };
        }
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let magnitude = parse_magnitude(digits)?;
        if negative {
            if !ty.is_signed() || magnitude > ty.zero_ordinal() {
                return None;
            }
            Some(Self::from_bits(ty, U256::ZERO.wrapping_sub(magnitude)))
        } else {
            (magnitude <= largest(ty)).then(|| Self::from_bits(ty, magnitude))
        }
    }

    /// The 32-byte ABI word: sign-extended for negative values.
    pub fn to_word(&self) -> U256 {
        if self.is_negative() {
            self.bits | !self.ty.mask()
        } else {
            self.bits
        }
    }

    /// Reads a canonically encoded ABI word.
    pub fn from_word(ty: Ty, word: U256) -> Option<Self> {
        let value = Self::from_bits(ty, word);
        (value.to_word() == word).then_some(value)
    }

    /// The low 64 bits of the sign-extended value, as a 64-bit host word
    /// carries it.
    pub fn to_u64_bits(&self) -> u64 {
        self.to_word().as_limbs()[0]
    }

    pub fn from_u64_bits(ty: Ty, word: u64) -> Option<Self> {
        if ty.is_signed() {
            Self::from_i128(ty, word as i64 as i128)
        } else if ty == Ty::Bool {
            (word <= 1).then(|| Self::boolean(word == 1))
        } else {
            Self::from_u128(ty, word as u128)
        }
    }

    /// Compares by numeric value; values of different types are ordered by
    /// type first.
    pub fn numeric_cmp(&self, other: &Self) -> Ordering {
        self.ty
            .cmp(&other.ty)
            .then_with(|| self.ordinal().cmp(&other.ordinal()))
    }
}

fn largest(ty: Ty) -> U256 {
    if ty.is_signed() {
        ty.zero_ordinal() - U256::from(1u8)
    } else {
        ty.mask()
    }
}

fn parse_magnitude(digits: &str) -> Option<U256> {
    let digits: String = digits.chars().filter(|c| *c != '_').collect();
    let (radix, body) = match digits.strip_prefix("0x") {
        Some(hex) => (16, hex),
        None => (10, digits.as_str()),
    };
    if body.is_empty() || !body.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    U256::from_str_radix(body, radix as u64).ok()
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.as_bool() {
            Some(value) => write!(f, "{value}"),
            None if self.is_negative() => {
                write!(f, "-{}", (U256::ZERO.wrapping_sub(self.to_word())))
            }
            None => write!(f, "{}", self.bits),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(ty: &str, text: &str) -> Value {
        Value::parse(Ty::parse(ty).unwrap(), text).unwrap()
    }

    #[test]
    fn parses_and_displays_decimal() {
        for (ty, text) in [
            ("u8", "255"),
            ("i8", "-128"),
            ("i8", "127"),
            ("i64", "-9223372036854775808"),
            ("u64", "18446744073709551615"),
            (
                "u256",
                "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            ),
            (
                "i256",
                "-57896044618658097711785492504343953926634992332820282019728792003956564819968",
            ),
            ("bool", "true"),
        ] {
            assert_eq!(value(ty, text).to_string(), text);
        }
        assert_eq!(value("u32", "0x_ff").to_string(), "255");
        assert_eq!(value("u32", "1_000").to_string(), "1000");
    }

    #[test]
    fn rejects_out_of_range_text() {
        let u8 = Ty::Uint(8);
        let i8 = Ty::Int(8);
        for (ty, text) in [
            (u8, "256"),
            (u8, "-1"),
            (i8, "128"),
            (i8, "-129"),
            (u8, ""),
            (u8, "1e3"),
        ] {
            assert_eq!(Value::parse(ty, text), None, "{text}");
        }
    }

    #[test]
    fn ordinals_preserve_numeric_order() {
        let i8 = Ty::Int(8);
        assert_eq!(Value::from_ordinal(i8, U256::ZERO).to_string(), "-128");
        assert_eq!(Value::from_ordinal(i8, U256::from(128u16)).to_string(), "0");
        assert_eq!(value("i8", "-1").ordinal(), U256::from(127u16));
        assert!(value("i16", "-2").numeric_cmp(&value("i16", "1")).is_lt());
    }

    #[test]
    fn words_are_sign_extended_and_checked() {
        let minus_one = value("i64", "-1");
        assert_eq!(minus_one.to_word(), U256::MAX);
        assert_eq!(Value::from_word(Ty::Int(64), U256::MAX), Some(minus_one));
        assert_eq!(Value::from_word(Ty::Uint(8), U256::from(256u16)), None);
        assert_eq!(Value::from_word(Ty::Bool, U256::from(2u8)), None);
        assert_eq!(minus_one.to_u64_bits(), u64::MAX);
        assert_eq!(
            Value::from_u64_bits(Ty::Int(8), u64::MAX),
            Some(value("i8", "-1"))
        );
        assert_eq!(Value::from_u64_bits(Ty::Uint(8), 300), None);
    }
}
