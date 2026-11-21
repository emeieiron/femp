//! The Fe compiler, linked as a library. Every femp command that compiles
//! goes through this crate: it loads and checks ingots, builds them for the
//! EVM, as native executables and as WebAssembly modules, runs their tests,
//! and lowers the functions claims name into the prover's verification IR.
//! It is the only crate that links the compiler.

mod diagnostics;
mod error;
mod evm;
mod find;
mod guard;
mod intrinsics;
mod lower;
mod module;
mod names;
mod native;
mod session;
mod spans;
mod spec_ops;
mod target;
mod test;
mod toolchain;
mod types;
mod wasm;

pub use error::{CompileError, Unsupported};
pub use evm::{Contract, EvmBuild};
pub use find::Root;
pub use lower::{
    ContractRoot, Handler, Invariant, Lowered, LoweredContract, LoweredMessage, MessageRoot,
    StorageLayout, Word,
};
pub use module::{load, test};
pub use session::Session;
pub use spec_ops::SPEC_INGOT;
pub use target::Target;
pub use test::{CaseResult, TestOptions, TestReport, WasmRunner};
pub use toolchain::{check_pin, toolchain};
pub use wasm::WasmModule;

/// The Fe release whose compiler crates this crate links. A project pinned
/// to another release cannot be compiled by this build of femp.
pub const LINKED_FE_VERSION: &str = "26.7.0";
/// The commit of that release, as the compiler records it in its outputs.
pub const LINKED_FE_COMMIT: &str = "e6e6342";
