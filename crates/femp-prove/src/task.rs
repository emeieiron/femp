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
    /// Specs whose summaries stand in for calls to their targets.
    pub summaries: Vec<Summary>,
    /// The spec this task proves, whose own target call stays as it is.
    pub proving: Option<Summary>,
    /// The specs whose summaries the task used, which must hold for its
    /// result to.
    pub relies_on: Vec<String>,
    /// What a contract's call to another contract that may call back
    /// means; without it, the callee leaves the storage alone.
    pub reentry: Option<Reentry>,
}

/// A contract's call to another contract that may call back into it. The
/// callee may send the contract any of its messages first, each of which
/// keeps the invariants, so the storage afterwards is any storage they
/// allow. Where a task proves the invariants, they must hold at the call
/// too, since those messages start from the storage as it stands.
#[derive(Debug, Clone)]
pub struct Reentry {
    /// The contract's `reenter`, which its code calls with its storage.
    pub func: FuncId,
    /// Each invariant, its name and the arguments it is checked at, at the
    /// call; empty where the task assumes the invariants.
    pub checked: Vec<(FuncId, String, Vec<ValueId>)>,
    /// Each invariant at each argument list the storage afterwards is
    /// assumed to satisfy it at.
    pub assumed: Vec<(FuncId, Vec<ValueId>)>,
}

/// A `#[spec]` function and the function it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub spec: FuncId,
    pub target: FuncId,
    pub name: String,
    pub trusted: bool,
}

impl Task {
    pub fn new(program: Program, entry: FuncId) -> Self {
        Self {
            program,
            entry,
            inputs: Vec::new(),
            assumed: Vec::new(),
            timeout: None,
            summaries: Vec::new(),
            proving: None,
            relies_on: Vec::new(),
            reentry: None,
        }
    }

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
