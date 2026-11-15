//! Generators turn choices into values. They are built from the
//! specifications in `#[gen(..)]` through a [`Registry`], where custom
//! generators can be added by name without touching the engine.

mod int;
mod pick;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use femp_core::property::{Ty, U256, Value};

use crate::choices::Choices;
use crate::spec::{self, Spec};

pub use int::IntRange;
pub use pick::{Bools, Constant, OneOf};

pub trait Generator: Send + Sync + fmt::Debug {
    fn ty(&self) -> Ty;

    fn draw(&self, choices: &mut Choices) -> Value;

    /// How many distinct values the generator produces, when that fits in a
    /// `u64`; exhaustive runs enumerate generators whose domains are small.
    fn domain_size(&self) -> Option<u64>;

    /// The choices that make `draw` produce its `rank`-th simplest value.
    fn choices_for(&self, rank: u64) -> Vec<U256>;
}

pub type Shared = Arc<dyn Generator>;

/// Builds a named generator for a type from its textual arguments.
pub type Factory = Box<dyn Fn(Ty, &[String]) -> Result<Shared, String> + Send + Sync>;

pub struct Registry {
    named: BTreeMap<String, Factory>,
}

impl Registry {
    /// The built-in named generators: `one_of(..)` and `const(..)`.
    pub fn standard() -> Self {
        let mut registry = Self {
            named: BTreeMap::new(),
        };
        registry.register(
            "one_of",
            Box::new(|ty, args| {
                let values = args
                    .iter()
                    .map(|arg| literal(ty, arg))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Arc::new(OneOf::new(values)?))
            }),
        );
        registry.register(
            "const",
            Box::new(|ty, args| match args {
                [arg] => Ok(Arc::new(Constant::new(literal(ty, arg)?))),
                _ => Err("`const` takes exactly one value".to_string()),
            }),
        );
        registry
    }

    pub fn register(&mut self, name: &str, factory: Factory) {
        self.named.insert(name.to_string(), factory);
    }

    pub fn build(&self, ty: Ty, text: &str) -> Result<Shared, String> {
        match spec::parse(text)? {
            Spec::Any if ty == Ty::Bool => Ok(Arc::new(Bools)),
            Spec::Any => Ok(Arc::new(IntRange::full(ty))),
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
                Ok(Arc::new(IntRange::new(ty, lo, hi)))
            }
            Spec::Named { name, args } => match self.named.get(&name) {
                Some(factory) => factory(ty, &args),
                None => Err(format!(
                    "unknown generator `{name}` (known: any, ranges, {})",
                    self.named.keys().cloned().collect::<Vec<_>>().join(", ")
                )),
            },
        }
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::standard()
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

fn literal(ty: Ty, text: &str) -> Result<Value, String> {
    Value::parse(ty, text).ok_or_else(|| format!("`{text}` is not a `{ty}` value"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(generator: &Shared, count: u64) -> Vec<String> {
        (0..count)
            .map(|rank| {
                let mut choices = Choices::replay(&generator.choices_for(rank));
                generator.draw(&mut choices).to_string()
            })
            .collect()
    }

    #[test]
    fn builds_generators_from_specs() {
        let registry = Registry::standard();
        let u8 = Ty::Uint(8);
        assert_eq!(
            values(&registry.build(u8, "3..=5").unwrap(), 3),
            ["3", "4", "5"]
        );
        assert_eq!(values(&registry.build(u8, "3..5").unwrap(), 2), ["3", "4"]);
        assert_eq!(
            values(&registry.build(u8, "250..=max").unwrap(), 2),
            ["250", "251"]
        );
        assert_eq!(
            values(&registry.build(Ty::Int(8), "min..=-126").unwrap(), 3),
            ["-126", "-127", "-128"]
        );
        assert_eq!(
            values(&registry.build(u8, "one_of(9, 2, 7)").unwrap(), 3),
            ["9", "2", "7"]
        );
        assert_eq!(
            values(&registry.build(Ty::Bool, "any").unwrap(), 2),
            ["false", "true"]
        );
        assert_eq!(
            values(&registry.build(Ty::Bool, "const(true)").unwrap(), 1),
            ["true"]
        );
    }

    #[test]
    fn reports_invalid_specs() {
        let registry = Registry::standard();
        let u8 = Ty::Uint(8);
        for (ty, text) in [
            (u8, "5..=3"),
            (u8, "3..3"),
            (u8, "0..=256"),
            (u8, "one_of()"),
            (u8, "const(1, 2)"),
            (u8, "fancy(1)"),
            (Ty::Bool, "0..=1"),
        ] {
            assert!(registry.build(ty, text).is_err(), "{text}");
        }
    }

    #[test]
    fn custom_generators_are_registered_by_name() {
        let mut registry = Registry::standard();
        registry.register(
            "even",
            Box::new(|ty, _| {
                Ok(Arc::new(OneOf::new(vec![
                    Value::from_u128(ty, 2).unwrap(),
                ])?))
            }),
        );
        assert_eq!(
            values(&registry.build(Ty::Uint(32), "even()").unwrap(), 1),
            ["2"]
        );
    }
}
