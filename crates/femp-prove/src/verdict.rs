use femp_core::property::Value;
use femp_vir::{Check, Span};

use crate::task::NotSupported;

/// What proving one task found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Proved,
    Refuted(Refutation),
    /// The solver gave up, or a bounded search found nothing.
    Unknown(String),
    NotSupported(NotSupported),
}

/// An execution that breaks the claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refutation {
    pub check: Check,
    /// Where the failing assertion is.
    pub span: Span,
    /// The primitive inputs, in the task's order, after minimization.
    pub values: Vec<Value>,
    /// Every input as `name = value`, structured ones included.
    pub shown: Vec<(String, String)>,
}

impl Verdict {
    pub fn is_proved(&self) -> bool {
        matches!(self, Self::Proved)
    }
}
