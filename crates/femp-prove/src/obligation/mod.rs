//! Builds the task of each claim: a procedure of `assume` and `assert`
//! around calls to the claim's functions.

pub mod differential;
mod inputs;
pub mod machine;
pub mod property;

pub use inputs::{choose, vir_type};
