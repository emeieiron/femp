//! The verification IR: what the prover reasons about. Functions are in SSA
//! form with block parameters, monomorphized and typed, and keep the source
//! line of every instruction. Panics stay implicit in the operations that
//! cause them until the prover's passes turn them into assertions.

pub mod build;
pub mod print;
pub mod program;
pub mod types;
mod visit;

pub use build::Cursor;
pub use program::*;
pub use types::{Field, Type, TypeDef, TypeId, TypeTable, Variant};
