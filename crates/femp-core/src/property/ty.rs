use std::fmt;

use ruint::aliases::U256;

const WIDTHS: [u16; 6] = [8, 16, 32, 64, 128, 256];

/// A primitive type that can cross every harness boundary: `bool` and the
/// fixed-width integers. `usize` and `isize` are excluded because their
/// width differs between backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ty {
    Bool,
    Uint(u16),
    Int(u16),
}

impl Ty {
    pub fn parse(name: &str) -> Option<Self> {
        if name == "bool" {
            return Some(Self::Bool);
        }
        let (signed, digits) = match name.as_bytes().first()? {
            b'u' => (false, &name[1..]),
            b'i' => (true, &name[1..]),
            _ => return None,
        };
        let bits: u16 = digits.parse().ok()?;
        if !WIDTHS.contains(&bits) || digits.starts_with('0') {
            return None;
        }
        Some(if signed {
            Self::Int(bits)
        } else {
            Self::Uint(bits)
        })
    }

    pub fn bits(self) -> u16 {
        match self {
            Self::Bool => 1,
            Self::Uint(bits) | Self::Int(bits) => bits,
        }
    }

    pub fn is_signed(self) -> bool {
        matches!(self, Self::Int(_))
    }

    /// The Solidity ABI name of the type.
    pub fn sol_name(self) -> String {
        match self {
            Self::Bool => "bool".to_string(),
            Self::Uint(bits) => format!("uint{bits}"),
            Self::Int(bits) => format!("int{bits}"),
        }
    }

    /// All bits of the type's width set.
    pub fn mask(self) -> U256 {
        match self.bits() {
            256 => U256::MAX,
            bits => (U256::from(1u8) << bits as usize) - U256::from(1u8),
        }
    }

    /// The largest ordinal; ordinals number the values from the smallest.
    pub fn max_ordinal(self) -> U256 {
        self.mask()
    }

    /// The ordinal of zero (`false` for `bool`).
    pub fn zero_ordinal(self) -> U256 {
        match self {
            Self::Int(bits) => U256::from(1u8) << (bits as usize - 1),
            _ => U256::ZERO,
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => f.write_str("bool"),
            Self::Uint(bits) => write!(f, "u{bits}"),
            Self::Int(bits) => write!(f, "i{bits}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixed_width_primitives() {
        assert_eq!(Ty::parse("bool"), Some(Ty::Bool));
        assert_eq!(Ty::parse("u8"), Some(Ty::Uint(8)));
        assert_eq!(Ty::parse("i256"), Some(Ty::Int(256)));
        for rejected in [
            "usize", "isize", "u7", "u512", "u064", "U8", "", "u", "String",
        ] {
            assert_eq!(Ty::parse(rejected), None, "{rejected}");
        }
    }

    #[test]
    fn names_round_trip() {
        for name in ["bool", "u16", "i64", "u256"] {
            assert_eq!(Ty::parse(name).unwrap().to_string(), name);
        }
        assert_eq!(Ty::Int(32).sol_name(), "int32");
        assert_eq!(Ty::Uint(256).sol_name(), "uint256");
    }

    #[test]
    fn ordinals_cover_the_width() {
        assert_eq!(Ty::Uint(8).max_ordinal(), U256::from(255u16));
        assert_eq!(Ty::Int(8).zero_ordinal(), U256::from(128u16));
        assert_eq!(Ty::Uint(256).max_ordinal(), U256::MAX);
        assert_eq!(Ty::Bool.max_ordinal(), U256::from(1u8));
    }
}
