use std::path::Path;
use std::time::Duration;

use crate::error::Result;
use crate::project::Module;

use super::outcome::Outcome;
use super::suite::Suite;
use super::value::Value;

/// Whether an executor can run an entry point, and why not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    Yes,
    No(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub command: usize,
    pub args: Vec<Value>,
    /// Seconds the block clock advances before the step; zero elsewhere.
    pub advance: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sequence {
    pub init: Vec<Value>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StepStatus {
    Done,
    /// The command's transaction reverted with this data, so its effects
    /// were rolled back. Only contract machines reject commands.
    Rejected(Vec<u8>),
}

/// The state after one step: the invariants in declaration order, then the
/// observations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Record {
    pub status: StepStatus,
    pub invariants: Vec<bool>,
    pub observations: Vec<Value>,
}

/// A run stopped abnormally. `step` 0 is the initializer and `k` the k-th
/// command.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Halt {
    pub step: usize,
    pub outcome: Outcome,
}

/// What one target observed while running a sequence. `records[0]` follows
/// the initializer and `records[k]` the k-th command.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Trace {
    pub records: Vec<Record>,
    pub halt: Option<Halt>,
}

/// Runs the entry points of one suite against one compiled target. Cases of
/// one call are independent; a machine run starts from a fresh state.
pub trait Executor: Send {
    fn supports_property(&self, index: usize) -> Support;

    fn supports_machine(&self, index: usize) -> Support;

    fn call(&mut self, property: usize, cases: &[Vec<Value>]) -> Result<Vec<Outcome>>;

    fn run(&mut self, machine: usize, sequence: &Sequence) -> Result<Trace>;
}

/// Asks a platform to compile a harness for `suite`, whose entry points live
/// in `subject`, and to start an executor for it.
pub struct HarnessRequest<'a> {
    pub subject: &'a Module,
    pub suite: &'a Suite,
    pub out_dir: &'a Path,
    /// The longest a single case may run before it counts as a timeout.
    pub timeout: Duration,
}
