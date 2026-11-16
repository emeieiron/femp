//! The bridge from the Fe compiler to the prover: loads an ingot with the
//! compiler's own crates, finds the functions claims name, and lowers them,
//! with everything they call, into the verification IR. This is the only
//! crate that links the compiler.

mod error;
mod find;
mod intrinsics;
mod load;
mod lower;
mod spans;
mod spec_ops;
mod types;

pub use error::{LoadError, Unsupported};
pub use find::Root;
pub use load::Loaded;
pub use lower::Lowered;
pub use spec_ops::SPEC_INGOT;

/// The Fe release whose compiler crates this crate links. Projects pinned
/// to another release are proved against this one, so `femp prove` warns.
pub const LINKED_FE_VERSION: &str = "26.6.0";
