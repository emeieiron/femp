//! Inputs that break a claim, in the form reports print and regression files
//! store. Property testing finds them by search and proving reads them from a
//! solver's model; either can replay what the other found.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arg {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepReport {
    pub command: String,
    pub args: Vec<Arg>,
    /// Seconds the block clock advanced before the step.
    pub advance: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum Counterexample {
    Args {
        args: Vec<Arg>,
    },
    Sequence {
        init: String,
        init_args: Vec<Arg>,
        steps: Vec<StepReport>,
    },
}
