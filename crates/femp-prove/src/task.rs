use std::time::Duration;

use femp_core::property::Ty;
use femp_spec::Domain;
use femp_vir::{FuncId, Program, Span, ValueId};

/// One obligation as a procedure: it chooses its inputs, assumes what is
/// known about them, runs the code and asserts what must hold. The prover's
/// passes reduce it to straight-line code the solver can decide.
#[derive(Debug, Clone)]
pub struct Task {
    pub program: Program,
    pub entry: FuncId,
    pub inputs: Vec<Input>,
    /// What the task takes on trust, for the report's "assumed" section.
    pub assumed: Vec<String>,
    pub timeout: Option<Duration>,
}

impl Task {
    pub fn entry(&self) -> &femp_vir::Function {
        self.program.function(self.entry)
    }

    pub fn entry_mut(&mut self) -> &mut femp_vir::Function {
        self.program.function_mut(self.entry)
    }
}

/// A value the solver chooses, reported back when a claim fails.
#[derive(Debug, Clone)]
pub struct Input {
    pub name: String,
    pub value: ValueId,
    /// For a primitive input, its type and the domain it was assumed in,
    /// which minimization searches from its simplest value.
    pub domain: Option<(Ty, Domain)>,
}

/// Code a task reaches that the prover cannot reason about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotSupported {
    pub span: Span,
    pub message: String,
}

impl NotSupported {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for NotSupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.span.is_known() {
            write!(f, "{}: {}", self.span, self.message)
        } else {
            f.write_str(&self.message)
        }
    }
}
