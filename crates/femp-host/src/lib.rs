//! Finds the programs on this machine that femp runs compiled code and
//! proofs with: Node.js for WebAssembly, a C compiler to link native
//! executables, and the Z3 solver.

pub mod home;
pub mod node;
pub mod probe;
pub mod process;
pub mod solver;
