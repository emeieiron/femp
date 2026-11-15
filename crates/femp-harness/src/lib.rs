//! Drives compiled property harnesses that run as child processes and talk
//! a line protocol over their standard streams: the native executable and
//! the Node.js runner for WebAssembly modules.

pub mod session;
pub mod wire;

pub use session::{Reply, Session};
