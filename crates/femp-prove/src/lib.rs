//! Proves claims about Fe code for all inputs. A claim becomes a task, a
//! procedure of assumptions and assertions around the code it is about;
//! passes reduce the task to straight-line code; its verification
//! conditions go to an SMT solver one assertion at a time; and a failing
//! assertion comes back as a minimized counterexample.

pub mod engine;
pub mod obligation;
pub mod passes;
pub mod report;
pub mod solver;
pub mod task;
pub mod vc;
pub mod verdict;

pub use engine::{Config, Dumps, prove};
pub use task::{Input, NotSupported, Task};
pub use verdict::{Refutation, Verdict};
