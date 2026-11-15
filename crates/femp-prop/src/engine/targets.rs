//! Runs one batch of work on several executors at once, one thread each.

use std::thread;

use femp_core::property::{Executor, Outcome, Sequence, Support, Trace, Value};
use femp_core::{Error, Result};

/// An executor and the label its column carries in reports.
pub struct Target<'a> {
    pub label: String,
    pub executor: &'a mut dyn Executor,
}

/// The targets an entry point runs on, and why the others do not run it.
pub struct Selection {
    pub running: Vec<usize>,
    pub skipped: Vec<(String, String)>,
}

pub fn select(targets: &[Target<'_>], supports: impl Fn(&dyn Executor) -> Support) -> Selection {
    let mut running = Vec::new();
    let mut skipped = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        match supports(&*target.executor) {
            Support::Yes => running.push(index),
            Support::No(reason) => skipped.push((target.label.clone(), reason)),
        }
    }
    Selection { running, skipped }
}

/// Outcomes of every case, one row per case and one column per running
/// target.
pub fn call(
    targets: &mut [Target<'_>],
    running: &[usize],
    property: usize,
    cases: &[Vec<Value>],
) -> Result<Vec<Vec<Outcome>>> {
    let columns = each(targets, running, |executor| executor.call(property, cases))?;
    Ok((0..cases.len())
        .map(|case| columns.iter().map(|column| column[case].clone()).collect())
        .collect())
}

/// The trace of one sequence on every running target.
pub fn run(
    targets: &mut [Target<'_>],
    running: &[usize],
    machine: usize,
    sequence: &Sequence,
) -> Result<Vec<Trace>> {
    each(targets, running, |executor| executor.run(machine, sequence))
}

fn each<T: Send>(
    targets: &mut [Target<'_>],
    running: &[usize],
    work: impl Fn(&mut dyn Executor) -> Result<T> + Sync,
) -> Result<Vec<T>> {
    let work = &work;
    thread::scope(|scope| {
        let handles: Vec<_> = targets
            .iter_mut()
            .enumerate()
            .filter(|(index, _)| running.contains(index))
            .map(|(_, target)| {
                let label = target.label.clone();
                let executor = &mut *target.executor;
                (label, scope.spawn(move || work(executor)))
            })
            .collect();
        handles
            .into_iter()
            .map(|(label, handle)| {
                handle
                    .join()
                    .map_err(|_| Error::Toolchain(format!("the `{label}` executor panicked")))?
                    .map_err(|error| Error::Toolchain(format!("{label}: {error}")))
            })
            .collect()
    })
}
