use femp_core::property::{Outcome, PropertyKind, Trace, Value};

use crate::compare::{Comparator, group, traces_equivalent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Every target agrees, but the property does not hold.
    Violated,
    /// The targets disagree.
    Diverged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Violated { targets: Vec<usize> },
    Diverged { groups: Vec<Vec<usize>> },
}

impl Failure {
    pub fn kind(&self) -> FailureKind {
        match self {
            Self::Violated { .. } => FailureKind::Violated,
            Self::Diverged { .. } => FailureKind::Diverged,
        }
    }
}

/// Judges one case from the outcome on each target. Divergence comes first:
/// it is the more specific finding when targets also violate the property.
pub fn judge_property(
    kind: PropertyKind,
    outcomes: &[Outcome],
    comparator: &dyn Comparator,
) -> Option<Failure> {
    let groups = group(outcomes, |a, b| comparator.equivalent(a, b));
    if groups.len() > 1 {
        return Some(Failure::Diverged { groups });
    }
    let holds = Outcome::Returned(Value::boolean(true));
    let targets: Vec<usize> = outcomes
        .iter()
        .enumerate()
        .filter(|(_, outcome)| match kind {
            PropertyKind::Holds => **outcome != holds,
            // Agreeing on a panic is behavior worth keeping; a trap is not.
            PropertyKind::Agrees => matches!(outcome, Outcome::Trapped(_)),
        })
        .map(|(index, _)| index)
        .collect();
    (!targets.is_empty()).then_some(Failure::Violated { targets })
}

/// Judges one machine run: the targets must produce equivalent traces, no
/// invariant may be false, and no step may halt.
pub fn judge_machine(traces: &[Trace], comparator: &dyn Comparator) -> Option<Failure> {
    let groups = group(traces, |a, b| traces_equivalent(comparator, a, b));
    if groups.len() > 1 {
        return Some(Failure::Diverged { groups });
    }
    let targets: Vec<usize> = traces
        .iter()
        .enumerate()
        .filter(|(_, trace)| {
            trace.halt.is_some()
                || trace
                    .records
                    .iter()
                    .any(|record| record.invariants.contains(&false))
        })
        .map(|(index, _)| index)
        .collect();
    (!targets.is_empty()).then_some(Failure::Violated { targets })
}

#[cfg(test)]
mod tests {
    use femp_core::property::{Halt, PanicKind, Record, StepStatus};

    use super::*;
    use crate::compare::Normalized;

    fn holds(value: bool) -> Outcome {
        Outcome::Returned(Value::boolean(value))
    }

    #[test]
    fn properties_must_hold_everywhere() {
        let all = [holds(true), holds(true)];
        assert_eq!(judge_property(PropertyKind::Holds, &all, &Normalized), None);
        let none = [holds(false), holds(false)];
        assert_eq!(
            judge_property(PropertyKind::Holds, &none, &Normalized),
            Some(Failure::Violated {
                targets: vec![0, 1]
            })
        );
        let split = [holds(true), holds(false), holds(true)];
        assert_eq!(
            judge_property(PropertyKind::Holds, &split, &Normalized),
            Some(Failure::Diverged {
                groups: vec![vec![0, 2], vec![1]]
            })
        );
    }

    #[test]
    fn differentials_accept_agreeing_panics() {
        let panics = [
            Outcome::Panicked(PanicKind::Overflow),
            Outcome::Panicked(PanicKind::Unknown),
        ];
        assert_eq!(
            judge_property(PropertyKind::Agrees, &panics, &Normalized),
            None
        );
        let traps = [Outcome::Trapped(femp_core::property::Trap::Timeout)];
        assert!(judge_property(PropertyKind::Agrees, &traps, &Normalized).is_some());
    }

    fn record(invariant: bool) -> Record {
        Record {
            status: StepStatus::Done,
            invariants: vec![invariant],
            observations: Vec::new(),
        }
    }

    #[test]
    fn machines_check_invariants_halts_and_agreement() {
        let good = Trace {
            records: vec![record(true), record(true)],
            halt: None,
        };
        assert_eq!(
            judge_machine(&[good.clone(), good.clone()], &Normalized),
            None
        );

        let broken = Trace {
            records: vec![record(true), record(false)],
            halt: None,
        };
        assert_eq!(
            judge_machine(&[broken.clone(), broken], &Normalized).map(|f| f.kind()),
            Some(FailureKind::Violated)
        );

        let halted = Trace {
            records: vec![record(true)],
            halt: Some(Halt {
                step: 1,
                outcome: Outcome::Panicked(PanicKind::Overflow),
            }),
        };
        assert_eq!(
            judge_machine(&[good, halted], &Normalized).map(|f| f.kind()),
            Some(FailureKind::Diverged)
        );
    }
}
