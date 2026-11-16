//! What a run found, independent of how it is shown: the text renderer
//! prints it for people and serde writes it as JSON for tools.

mod text;

use serde::{Deserialize, Serialize};

pub use femp_spec::counterexample::{Arg, Counterexample, StepReport};
pub use text::render;

use crate::verdict::FailureKind;

#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    pub seed: String,
    pub targets: Vec<String>,
    pub comparison: String,
    pub entries: Vec<EntryReport>,
}

impl RunReport {
    pub fn succeeded(&self) -> bool {
        self.entries
            .iter()
            .all(|entry| matches!(entry.status, Status::Passed | Status::Skipped { .. }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Property,
    Differential,
    Machine,
    Spec,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntryReport {
    pub module: String,
    pub name: String,
    pub kind: EntryKind,
    pub location: String,
    /// Targets the entry ran on.
    pub ran_on: Vec<String>,
    /// Targets that cannot run it, with the reason.
    pub skipped: Vec<Skip>,
    /// Cases (or machine runs) executed, regressions included.
    pub cases: usize,
    /// Cases a spec's `requires` rejected, among `cases`.
    #[serde(skip_serializing_if = "is_zero")]
    pub rejected: usize,
    pub regressions: usize,
    pub exhaustive: bool,
    /// Things worth knowing that do not fail the run, such as stored
    /// regressions that no longer match the declaration.
    pub notes: Vec<String>,
    pub status: Status,
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

impl EntryReport {
    pub fn label(&self) -> String {
        format!("{}::{}", self.module, self.name)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Skip {
    pub target: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum Status {
    Passed,
    Failed(Box<FailureReport>),
    /// No target can run the entry.
    Skipped {
        reason: String,
    },
    /// The declaration could not be turned into generators.
    Invalid {
        message: String,
    },
    /// A harness failed while running the entry.
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct FailureReport {
    pub kind: FailureKindName,
    /// The case that first failed, counted from 0; `None` when a stored
    /// regression failed.
    pub case: Option<u64>,
    pub seed: String,
    pub counterexample: Counterexample,
    /// What each target did with the counterexample.
    pub outcomes: Vec<TargetOutcome>,
    pub shrink: Option<ShrinkStats>,
    pub saved_to: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailureKindName {
    Violated,
    Diverged,
}

impl From<FailureKind> for FailureKindName {
    fn from(kind: FailureKind) -> Self {
        match kind {
            FailureKind::Violated => Self::Violated,
            FailureKind::Diverged => Self::Diverged,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetOutcome {
    pub target: String,
    pub outcome: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ShrinkStats {
    pub attempts: usize,
    pub improvements: usize,
}
