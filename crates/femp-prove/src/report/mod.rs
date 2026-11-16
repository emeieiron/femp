//! What a run proved, independent of how it is shown: the text renderer
//! prints it for people and serde writes it as JSON for tools. Nothing in
//! it depends on time, so equal runs give equal reports.

mod text;

use femp_spec::Counterexample;
use serde::Serialize;

pub use text::render;

#[derive(Debug, Clone, Serialize)]
pub struct ProveReport {
    pub targets: Vec<String>,
    pub claims: Vec<ClaimReport>,
}

impl ProveReport {
    /// Whether the run passes: nothing refuted or unknown, and with
    /// `strict`, nothing the prover could not handle.
    pub fn succeeded(&self, strict: bool) -> bool {
        self.claims.iter().all(|claim| match &claim.status {
            ClaimStatus::Proved | ClaimStatus::Skipped { .. } => true,
            ClaimStatus::NotSupported { .. } => !strict,
            ClaimStatus::Refuted(_) | ClaimStatus::Unknown { .. } | ClaimStatus::Error { .. } => {
                false
            }
        })
    }

    /// Every assumption the run relied on, once each.
    pub fn assumed(&self) -> Vec<String> {
        let mut assumed: Vec<String> = Vec::new();
        for claim in &self.claims {
            for assumption in &claim.assumed {
                let line = format!("{}: {assumption}", claim.label());
                if !assumed.contains(&line) {
                    assumed.push(line);
                }
            }
        }
        assumed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaimKind {
    Property,
    Differential,
    Machine,
    Spec,
    /// A deployed contract whose storage invariants hold after every message.
    Contract,
}

impl ClaimKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Property => "property",
            Self::Differential => "differential",
            Self::Machine => "machine",
            Self::Spec => "spec",
            Self::Contract => "contract",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaimReport {
    pub module: String,
    pub name: String,
    pub kind: ClaimKind,
    pub location: String,
    /// The targets whose programs the claim was proved or checked on.
    pub targets: Vec<String>,
    /// What the proof took on trust.
    pub assumed: Vec<String>,
    pub notes: Vec<String>,
    pub status: ClaimStatus,
}

impl ClaimReport {
    pub fn label(&self) -> String {
        format!("{}::{}", self.module, self.name)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ClaimStatus {
    Proved,
    Refuted(Box<Refuted>),
    /// The solver gave up, or a bounded search found nothing.
    Unknown {
        reason: String,
    },
    /// The claim reaches code the prover has no model for.
    NotSupported {
        location: String,
        reason: String,
    },
    Skipped {
        reason: String,
    },
    /// The claim could not be checked at all, such as a declaration that
    /// does not resolve.
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Refuted {
    /// What fails: `returns false`, or a panic and where it happens.
    pub failure: String,
    pub counterexample: Counterexample,
    /// The targets whose programs the solver refuted.
    pub refuted_on: Vec<String>,
    /// What each compiled target did with the counterexample.
    pub replay: Vec<Replay>,
    pub saved_to: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Replay {
    pub target: String,
    pub outcome: String,
}
