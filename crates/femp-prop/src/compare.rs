//! Comparison strategies decide when two targets agree. They are pluggable
//! so that stricter or looser notions of agreement need no engine changes.

use femp_core::property::{Outcome, PanicKind, Trace};

pub trait Comparator: Send + Sync {
    fn name(&self) -> &'static str;

    fn equivalent(&self, a: &Outcome, b: &Outcome) -> bool;
}

/// Values must be equal; panics agree unless both report a reason and the
/// reasons differ, because WebAssembly traps carry no reason.
pub struct Normalized;

/// Outcomes must be identical.
pub struct Strict;

/// Values must be equal; any two abnormal outcomes agree.
pub struct Lenient;

impl Comparator for Normalized {
    fn name(&self) -> &'static str {
        "normalized"
    }

    fn equivalent(&self, a: &Outcome, b: &Outcome) -> bool {
        match (a, b) {
            (Outcome::Panicked(a), Outcome::Panicked(b)) => {
                a == b || *a == PanicKind::Unknown || *b == PanicKind::Unknown
            }
            (Outcome::Trapped(a), Outcome::Trapped(b)) => {
                std::mem::discriminant(a) == std::mem::discriminant(b)
            }
            _ => a == b,
        }
    }
}

impl Comparator for Strict {
    fn name(&self) -> &'static str {
        "strict"
    }

    fn equivalent(&self, a: &Outcome, b: &Outcome) -> bool {
        a == b
    }
}

impl Comparator for Lenient {
    fn name(&self) -> &'static str {
        "lenient"
    }

    fn equivalent(&self, a: &Outcome, b: &Outcome) -> bool {
        match (a, b) {
            (Outcome::Returned(a), Outcome::Returned(b)) => a == b,
            (a, b) => a.is_abnormal() && b.is_abnormal(),
        }
    }
}

pub const NAMES: [&str; 3] = ["normalized", "strict", "lenient"];

pub fn by_name(name: &str) -> Option<Box<dyn Comparator>> {
    match name {
        "normalized" => Some(Box::new(Normalized)),
        "strict" => Some(Box::new(Strict)),
        "lenient" => Some(Box::new(Lenient)),
        _ => None,
    }
}

/// Traces agree when every record is equal and they halt at the same step
/// with equivalent outcomes.
pub fn traces_equivalent(comparator: &dyn Comparator, a: &Trace, b: &Trace) -> bool {
    a.records == b.records
        && match (&a.halt, &b.halt) {
            (None, None) => true,
            (Some(a), Some(b)) => a.step == b.step && comparator.equivalent(&a.outcome, &b.outcome),
            _ => false,
        }
}

/// Partitions targets into groups that agree with each group's first member.
pub fn group<T>(items: &[T], same: impl Fn(&T, &T) -> bool) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match groups.iter_mut().find(|group| same(&items[group[0]], item)) {
            Some(group) => group.push(index),
            None => groups.push(vec![index]),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use femp_core::property::{Trap, Ty, Value};

    use super::*;

    fn returned(value: u64) -> Outcome {
        Outcome::Returned(Value::from_u128(Ty::Uint(64), value as u128).unwrap())
    }

    #[test]
    fn normalized_lets_unknown_reasons_match() {
        let overflow = Outcome::Panicked(PanicKind::Overflow);
        let unknown = Outcome::Panicked(PanicKind::Unknown);
        let division = Outcome::Panicked(PanicKind::DivisionByZero);
        assert!(Normalized.equivalent(&overflow, &unknown));
        assert!(!Normalized.equivalent(&overflow, &division));
        assert!(!Normalized.equivalent(&returned(1), &returned(2)));
        assert!(!Normalized.equivalent(&returned(1), &overflow));
        assert!(Normalized.equivalent(
            &Outcome::Trapped(Trap::Crashed("a".into())),
            &Outcome::Trapped(Trap::Crashed("b".into()))
        ));
    }

    #[test]
    fn strict_and_lenient_bracket_normalized() {
        let overflow = Outcome::Panicked(PanicKind::Overflow);
        let unknown = Outcome::Panicked(PanicKind::Unknown);
        assert!(!Strict.equivalent(&overflow, &unknown));
        assert!(Lenient.equivalent(&overflow, &Outcome::Trapped(Trap::Timeout)));
        assert!(!Lenient.equivalent(&returned(3), &overflow));
        for name in NAMES {
            assert_eq!(by_name(name).unwrap().name(), name);
        }
    }

    #[test]
    fn groups_agreeing_targets() {
        let outcomes = [returned(1), returned(2), returned(1)];
        assert_eq!(
            group(&outcomes, |a, b| Normalized.equivalent(a, b)),
            [vec![0, 2], vec![1]]
        );
    }
}
