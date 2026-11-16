//! Runs a suite's entry points on every target: stored regressions first,
//! then exhaustive or seeded random cases in batches, and shrinks the first
//! failure of each entry to a minimal counterexample.

mod machine;
mod property;
pub mod targets;

use femp_core::property::{Suite, U256};

use crate::choices::{Choices, Tape};
use crate::compare::Comparator;
use crate::generators::Registry;
use crate::regress::{Recorded, Store};
use crate::report::{EntryKind, EntryReport, Status};
use crate::rng::{derive, fnv1a};

pub use targets::Target;

pub const DEFAULT_CASES: u32 = 256;
pub const DEFAULT_RUNS: u32 = 100;
pub const DEFAULT_MAX_SHRINKS: usize = 2000;
/// The largest domain `--exhaustive` enumerates.
pub const EXHAUSTIVE_LIMIT: u64 = 1 << 20;
const BATCH: usize = 64;

#[derive(Debug, Clone)]
pub struct Config {
    pub seed: u64,
    /// Overrides every entry's own number of cases or runs.
    pub cases: Option<u32>,
    pub exhaustive: bool,
    pub shrink: bool,
    pub max_shrinks: usize,
    /// Replay stored regressions and store new ones.
    pub regressions: bool,
    pub fe_version: String,
    pub optimize: Vec<String>,
}

pub struct Engine<'a> {
    pub config: &'a Config,
    pub registry: &'a Registry,
    pub comparator: &'a dyn Comparator,
}

impl Engine<'_> {
    pub fn run_suite(
        &self,
        suite: &Suite,
        targets: &mut [Target<'_>],
        store: &mut Store,
        on_entry: &mut dyn FnMut(&EntryReport),
    ) -> Vec<EntryReport> {
        let mut reports = Vec::new();
        for index in 0..suite.properties.len() {
            let report = property::run(self, suite, index, targets, store);
            on_entry(&report);
            reports.push(report);
        }
        for index in 0..suite.machines.len() {
            let report = machine::run(self, suite, index, targets, store);
            on_entry(&report);
            reports.push(report);
        }
        reports
    }

    fn entry_seed(&self, suite: &Suite, name: &str) -> u64 {
        derive(
            self.config.seed,
            fnv1a(&format!("{}::{name}", suite.module)),
        )
    }

    fn recorded(&self) -> Recorded {
        Recorded::now(&self.config.fe_version, self.config.optimize.clone())
    }
}

pub fn format_seed(seed: u64) -> String {
    format!("{seed:#018x}")
}

fn report(suite: &Suite, name: &str, kind: EntryKind, location: String) -> EntryReport {
    EntryReport {
        module: suite.module.clone(),
        name: name.to_string(),
        kind,
        location,
        ran_on: Vec::new(),
        skipped: Vec::new(),
        cases: 0,
        rejected: 0,
        regressions: 0,
        exhaustive: false,
        notes: Vec::new(),
        status: Status::Passed,
    }
}

/// The tape of the `index`-th generated case: the simplest case, then the
/// largest choices everywhere, then seeded random ones.
fn generated<T>(seed: u64, index: u64, draw: impl Fn(&mut Choices) -> T) -> (T, Tape) {
    let mut choices = match index {
        0 => Choices::replay(&[]),
        1 => Choices::replay(&[U256::MAX; 64]),
        _ => Choices::random(derive(seed, index)),
    };
    let case = draw(&mut choices);
    (case, choices.into_tape())
}

#[cfg(test)]
mod tests;
