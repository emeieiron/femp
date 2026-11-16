//! Builds the task of each claim: a procedure of `assume` and `assert`
//! around calls to the claim's functions.

mod inputs;
pub mod property;

pub use inputs::{choose, vir_type};
