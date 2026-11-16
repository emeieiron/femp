//! How a machine run ended, in words, for reports and for the prover's
//! replays.

use femp_core::property::{Machine, Record, StepStatus, Trace};

// The first record at which some target's trace differs from the first.
pub fn first_difference(traces: &[Trace]) -> Option<usize> {
    let first = traces.first()?;
    let longest = traces.iter().map(|trace| trace.records.len()).max()?;
    (0..longest).find(|at| {
        traces
            .iter()
            .any(|trace| trace.records.get(*at) != first.records.get(*at))
    })
}

fn step_name(machine: &Machine, step: usize) -> String {
    if step == 0 {
        format!("`{}`", machine.init.name)
    } else {
        format!("step {step}")
    }
}

pub fn describe(machine: &Machine, trace: &Trace, focus: Option<usize>) -> String {
    if let Some(at) = focus {
        return match trace.records.get(at) {
            Some(record) => format!(
                "after {}: {}",
                step_name(machine, at),
                describe_record(machine, record)
            ),
            None => match &trace.halt {
                Some(halt) => format!("{} at {}", halt.outcome, step_name(machine, halt.step)),
                None => format!("stopped before {}", step_name(machine, at)),
            },
        };
    }
    if let Some(halt) = &trace.halt {
        return format!("{} at {}", halt.outcome, step_name(machine, halt.step));
    }
    for (at, record) in trace.records.iter().enumerate() {
        if let Some(invariant) = record.invariants.iter().position(|holds| !holds) {
            return format!(
                "invariant `{}` fails after {}",
                machine.invariants[invariant].name,
                step_name(machine, at)
            );
        }
    }
    format!("completed {} steps", trace.records.len().saturating_sub(1))
}

fn describe_record(machine: &Machine, record: &Record) -> String {
    let mut parts = Vec::new();
    if let StepStatus::Rejected(data) = &record.status {
        parts.push(format!(
            "rejected (0x{})",
            femp_core::property::outcome::hex(data)
        ));
    }
    for (invariant, holds) in machine.invariants.iter().zip(&record.invariants) {
        if !holds {
            parts.push(format!("`{}` fails", invariant.name));
        }
    }
    for (observation, value) in machine.observations.iter().zip(&record.observations) {
        parts.push(format!("{} = {value}", observation.method.name));
    }
    if parts.is_empty() {
        "every invariant holds".to_string()
    } else {
        parts.join(", ")
    }
}
