use std::fmt;
use std::path::PathBuf;

use super::ty::Ty;

/// Where an entry point was declared, for messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub file: PathBuf,
    pub line: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.file.display(), self.line)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    /// The label callers write before the argument (`label: value`); `None`
    /// for parameters declared as `_ name`.
    pub label: Option<String>,
    pub ty: Ty,
    /// The generator specification from `#[gen(..)]`, `any` by default.
    pub generator: String,
}

/// An effect a machine method declares, such as `create: mut Create`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    pub ty: String,
    pub mutable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyKind {
    /// `#[property]`: returns `bool`, which must be `true` on every target.
    Holds,
    /// `#[differential]`: any primitive result, which every target must agree on.
    Agrees,
    /// A `#[spec]` function: a case its `requires` rejects is discarded;
    /// otherwise it must return without panicking on every target.
    Spec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub kind: PropertyKind,
    /// The module path of the function inside its ingot (empty for the root).
    pub path: Vec<String>,
    pub params: Vec<Param>,
    pub result: Ty,
    pub cases: Option<u32>,
    pub location: Location,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Method {
    pub name: String,
    pub params: Vec<Param>,
    pub effects: Vec<Effect>,
    pub location: Location,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub method: Method,
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub method: Method,
    pub ty: Ty,
}

/// A `#[machine]` struct: an initializer, commands that each return the next
/// state, and the checks made after every step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    pub name: String,
    pub path: Vec<String>,
    pub init: Method,
    pub commands: Vec<Command>,
    pub invariants: Vec<Method>,
    pub observations: Vec<Observation>,
    /// The specification of the number of steps per run, such as `1..=16`.
    pub steps: String,
    pub runs: Option<u32>,
    /// Seconds the EVM block clock advances before each step.
    pub clock: Option<String>,
    pub location: Location,
}

impl Machine {
    pub fn methods(&self) -> impl Iterator<Item = &Method> {
        std::iter::once(&self.init)
            .chain(self.commands.iter().map(|command| &command.method))
            .chain(&self.invariants)
            .chain(
                self.observations
                    .iter()
                    .map(|observation| &observation.method),
            )
    }

    pub fn uses_effects(&self) -> bool {
        self.methods().any(|method| !method.effects.is_empty())
    }

    /// How many lines or words describe the state after one step.
    pub fn record_len(&self) -> usize {
        self.invariants.len() + self.observations.len()
    }
}

/// The entry points of one module. Properties are numbered first, then
/// machines; harnesses use those numbers to dispatch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Suite {
    pub module: String,
    pub ingot: String,
    pub properties: Vec<Property>,
    pub machines: Vec<Machine>,
}

impl Suite {
    pub fn is_empty(&self) -> bool {
        self.properties.is_empty() && self.machines.is_empty()
    }

    pub fn property_id(&self, index: usize) -> usize {
        index
    }

    pub fn machine_id(&self, index: usize) -> usize {
        self.properties.len() + index
    }

    /// The Fe path a harness uses to reach an item of the subject ingot.
    pub fn item_path(&self, path: &[String], name: &str) -> String {
        std::iter::once(self.ingot.as_str())
            .chain(path.iter().map(String::as_str))
            .chain(std::iter::once(name))
            .collect::<Vec<_>>()
            .join("::")
    }
}
