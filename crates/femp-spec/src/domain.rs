//! Generator specifications as written in `#[gen(param = "...")]`, and the
//! set of values each one denotes:
//!
//! - `any`: every value of the type;
//! - `lo..=hi` or `lo..hi`: a range, where a bound is a number (decimal or
//!   `0x` hex, `_` allowed), `min` or `max`;
//! - `name(arg, ..)`: a named generator, such as `one_of(1, 5, 9)`,
//!   `const(7)`, or one a tool registers itself.
//!
//! Property testing draws from a [`Domain`]; proving assumes its arguments
//! lie in it.

use femp_core::property::{Ty, U256, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    Any,
    Range {
        lo: String,
        hi: String,
        inclusive: bool,
    },
    Named {
        name: String,
        args: Vec<String>,
    },
}

pub fn parse(text: &str) -> Result<Spec, String> {
    let text = text.trim();
    if text == "any" {
        return Ok(Spec::Any);
    }
    if let Some((lo, rest)) = text.split_once("..") {
        let (inclusive, hi) = match rest.strip_prefix('=') {
            Some(hi) => (true, hi),
            None => (false, rest),
        };
        let (lo, hi) = (lo.trim(), hi.trim());
        if lo.is_empty() || hi.is_empty() {
            return Err(format!("`{text}` needs both bounds, as in `0..=10`"));
        }
        return Ok(Spec::Range {
            lo: lo.to_string(),
            hi: hi.to_string(),
            inclusive,
        });
    }
    if let Some((name, rest)) = text.split_once('(') {
        let name = name.trim();
        let Some(inner) = rest.trim_end().strip_suffix(')') else {
            return Err(format!("`{text}` is missing a closing parenthesis"));
        };
        if !is_identifier(name) {
            return Err(format!("`{name}` is not a generator name"));
        }
        let args = if inner.trim().is_empty() {
            Vec::new()
        } else {
            inner.split(',').map(|arg| arg.trim().to_string()).collect()
        };
        if args.iter().any(String::is_empty) {
            return Err(format!("`{text}` has an empty argument"));
        }
        return Ok(Spec::Named {
            name: name.to_string(),
            args,
        });
    }
    Err(format!(
        "`{text}` is not a generator: expected `any`, a range such as `0..=10`, or a call such as `one_of(1, 2)`"
    ))
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The named generators every tool understands.
pub const BUILT_IN: [&str; 2] = ["const", "one_of"];

/// The values a generator specification denotes for one type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Domain {
    Any(Ty),
    /// An inclusive, non-empty range.
    Range {
        lo: Value,
        hi: Value,
    },
    /// A non-empty list; earlier values are simpler.
    OneOf(Vec<Value>),
    Const(Value),
    /// A generator registered by name outside this crate. Tools that cannot
    /// run it treat it as the whole type.
    Custom {
        name: String,
        args: Vec<String>,
    },
}

impl Domain {
    pub fn resolve(ty: Ty, text: &str) -> Result<Self, String> {
        match parse(text)? {
            Spec::Any => Ok(Self::Any(ty)),
            Spec::Range { .. } if ty == Ty::Bool => {
                Err("a `bool` takes `any`, `one_of(..)` or `const(..)`, not a range".to_string())
            }
            Spec::Range { lo, hi, inclusive } => {
                let lo = bound(ty, &lo)?;
                let mut hi = bound(ty, &hi)?;
                if !inclusive {
                    if hi <= lo {
                        return Err(format!("the range `{text}` is empty"));
                    }
                    hi -= U256::from(1u8);
                }
                if hi < lo {
                    return Err(format!("the range `{text}` is empty"));
                }
                Ok(Self::Range {
                    lo: Value::from_ordinal(ty, lo),
                    hi: Value::from_ordinal(ty, hi),
                })
            }
            Spec::Named { name, args } => match name.as_str() {
                "one_of" => {
                    let values = args
                        .iter()
                        .map(|arg| literal(ty, arg))
                        .collect::<Result<Vec<_>, _>>()?;
                    if values.is_empty() {
                        return Err("`one_of` needs at least one value".to_string());
                    }
                    Ok(Self::OneOf(values))
                }
                "const" => match args.as_slice() {
                    [arg] => Ok(Self::Const(literal(ty, arg)?)),
                    _ => Err("`const` takes exactly one value".to_string()),
                },
                _ => Ok(Self::Custom { name, args }),
            },
        }
    }

    /// The ordinal range covering every value of the domain, which is the
    /// whole type for a custom generator.
    pub fn hull(&self, ty: Ty) -> (U256, U256) {
        let values = match self {
            Self::Any(_) | Self::Custom { .. } => return (U256::ZERO, ty.max_ordinal()),
            Self::Range { lo, hi } => return (lo.ordinal(), hi.ordinal()),
            Self::OneOf(values) => values.as_slice(),
            Self::Const(value) => std::slice::from_ref(value),
        };
        let ordinals = values.iter().map(Value::ordinal);
        (
            ordinals.clone().min().unwrap_or_default(),
            ordinals.max().unwrap_or_default(),
        )
    }
}

/// A range bound as an ordinal: a literal, `min` or `max`.
fn bound(ty: Ty, text: &str) -> Result<U256, String> {
    match text {
        "min" => Ok(U256::ZERO),
        "max" => Ok(ty.max_ordinal()),
        _ => literal(ty, text).map(|value| value.ordinal()),
    }
}

pub fn literal(ty: Ty, text: &str) -> Result<Value, String> {
    Value::parse(ty, text).ok_or_else(|| format!("`{text}` is not a `{ty}` value"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(lo: &str, hi: &str, inclusive: bool) -> Spec {
        Spec::Range {
            lo: lo.into(),
            hi: hi.into(),
            inclusive,
        }
    }

    #[test]
    fn parses_every_form() {
        assert_eq!(parse(" any "), Ok(Spec::Any));
        assert_eq!(parse("0..=1_000"), Ok(range("0", "1_000", true)));
        assert_eq!(parse("-5..max"), Ok(range("-5", "max", false)));
        assert_eq!(
            parse("one_of(1, 0x10, -3)"),
            Ok(Spec::Named {
                name: "one_of".into(),
                args: vec!["1".into(), "0x10".into(), "-3".into()]
            })
        );
        assert_eq!(
            parse("const(true)"),
            Ok(Spec::Named {
                name: "const".into(),
                args: vec!["true".into()]
            })
        );
    }

    #[test]
    fn explains_malformed_specs() {
        for text in [
            "..=3",
            "1..",
            "one_of(1,",
            "one_of(1,,2)",
            "9lives(1)",
            "sometimes",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }

    fn value(ty: Ty, text: &str) -> Value {
        Value::parse(ty, text).unwrap()
    }

    #[test]
    fn resolves_domains_against_types() {
        let u8 = Ty::Uint(8);
        let i8 = Ty::Int(8);
        assert_eq!(Domain::resolve(u8, "any"), Ok(Domain::Any(u8)));
        assert_eq!(
            Domain::resolve(u8, "3..5"),
            Ok(Domain::Range {
                lo: value(u8, "3"),
                hi: value(u8, "4")
            })
        );
        assert_eq!(
            Domain::resolve(i8, "min..=-126"),
            Ok(Domain::Range {
                lo: value(i8, "-128"),
                hi: value(i8, "-126")
            })
        );
        assert_eq!(
            Domain::resolve(u8, "one_of(9, 2)"),
            Ok(Domain::OneOf(vec![value(u8, "9"), value(u8, "2")]))
        );
        assert_eq!(
            Domain::resolve(Ty::Bool, "const(true)"),
            Ok(Domain::Const(Value::boolean(true)))
        );
        assert_eq!(
            Domain::resolve(u8, "even()"),
            Ok(Domain::Custom {
                name: "even".into(),
                args: Vec::new()
            })
        );
    }

    #[test]
    fn rejects_domains_that_do_not_fit_the_type() {
        let u8 = Ty::Uint(8);
        for (ty, text) in [
            (u8, "5..=3"),
            (u8, "3..3"),
            (u8, "0..=256"),
            (u8, "one_of()"),
            (u8, "const(1, 2)"),
            (Ty::Bool, "0..=1"),
        ] {
            assert!(Domain::resolve(ty, text).is_err(), "{text}");
        }
    }

    #[test]
    fn hulls_cover_the_domain() {
        let i8 = Ty::Int(8);
        let ordinal = |text| value(i8, text).ordinal();
        assert_eq!(
            Domain::resolve(i8, "one_of(5, -3, 2)").unwrap().hull(i8),
            (ordinal("-3"), ordinal("5"))
        );
        assert_eq!(
            Domain::resolve(i8, "fancy(1)").unwrap().hull(i8),
            (U256::ZERO, i8.max_ordinal())
        );
    }
}
