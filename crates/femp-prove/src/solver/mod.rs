//! Talking to an SMT solver: a session receives the script, answers one
//! satisfiability check at a time under push and pop, and reports values of
//! a model.

pub mod minimize;
pub mod model;
pub mod sexp;
mod z3;

use std::time::Duration;

pub use sexp::Sexp;
pub use z3::Z3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Sat,
    Unsat,
    Unknown(String),
}

pub trait Solver {
    /// Sends commands that answer nothing, and fails if the solver rejects
    /// them.
    fn send(&mut self, commands: &str) -> Result<(), String>;

    fn check(&mut self, timeout: Duration) -> Result<Answer, String>;

    /// The model's value of each term, after a satisfiable check.
    fn values(&mut self, terms: &[String]) -> Result<Vec<Sexp>, String>;

    fn push(&mut self) -> Result<(), String> {
        self.send("(push 1)")
    }

    fn pop(&mut self) -> Result<(), String> {
        self.send("(pop 1)")
    }

    fn assert(&mut self, term: &str) -> Result<(), String> {
        self.send(&format!("(assert {term})"))
    }
}
