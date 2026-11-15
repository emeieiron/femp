use femp_core::property::{Ty, U256, Value};

use super::Generator;
use crate::choices::Choices;

/// `false` or `true`, in that order of simplicity.
#[derive(Debug)]
pub struct Bools;

impl Generator for Bools {
    fn ty(&self) -> Ty {
        Ty::Bool
    }

    fn draw(&self, choices: &mut Choices) -> Value {
        Value::boolean(choices.draw_uniform(U256::from(1u8)) == U256::from(1u8))
    }

    fn domain_size(&self) -> Option<u64> {
        Some(2)
    }

    fn choices_for(&self, rank: u64) -> Vec<U256> {
        vec![U256::from(rank)]
    }
}

/// One of a fixed list of values; earlier ones are simpler.
#[derive(Debug)]
pub struct OneOf {
    values: Vec<Value>,
}

impl OneOf {
    pub fn new(values: Vec<Value>) -> Result<Self, String> {
        if values.is_empty() {
            return Err("`one_of` needs at least one value".to_string());
        }
        Ok(Self { values })
    }
}

impl Generator for OneOf {
    fn ty(&self) -> Ty {
        self.values[0].ty()
    }

    fn draw(&self, choices: &mut Choices) -> Value {
        let count = self.values.len() as u64;
        self.values[choices.draw_index(self.values.len(), |rng| rng.below_u64(count))]
    }

    fn domain_size(&self) -> Option<u64> {
        Some(self.values.len() as u64)
    }

    fn choices_for(&self, rank: u64) -> Vec<U256> {
        vec![U256::from(rank)]
    }
}

/// Always the same value; it consumes no choices.
#[derive(Debug)]
pub struct Constant {
    value: Value,
}

impl Constant {
    pub fn new(value: Value) -> Self {
        Self { value }
    }
}

impl Generator for Constant {
    fn ty(&self) -> Ty {
        self.value.ty()
    }

    fn draw(&self, _choices: &mut Choices) -> Value {
        self.value
    }

    fn domain_size(&self) -> Option<u64> {
        Some(1)
    }

    fn choices_for(&self, _rank: u64) -> Vec<U256> {
        Vec::new()
    }
}
