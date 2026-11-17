//! Shrinking edits a failing case's choice tape and keeps any edit that is
//! simpler (shorter, or equally long and smaller choice by choice) and still
//! fails the same way. Only strictly simpler tapes are accepted, so shrinking
//! always terminates; a budget bounds the number of replays.

mod passes;

use femp_core::property::U256;

use crate::choices::Tape;

pub use passes::{DeleteChoices, DeleteSpans, MinimizeChoices, Redistribute, ZeroSpans};

/// A strategy for proposing simpler tapes. Passes are independent, so new
/// ones can be added without changing the driver.
pub trait ShrinkPass: Send + Sync {
    fn name(&self) -> &'static str;

    /// Returns a simpler tape than `current` that still fails, if the pass
    /// finds one.
    fn run(&self, current: &Tape, probe: &mut Probe<'_>) -> Option<Tape>;
}

/// Replays candidate tapes for a pass. `check` replays the choices and
/// returns the tape actually consumed if the case still fails.
pub struct Probe<'a> {
    check: &'a mut dyn FnMut(&[U256]) -> Option<Tape>,
    attempts: usize,
    budget: usize,
}

impl Probe<'_> {
    /// Replays `choices` and returns the resulting tape if it still fails
    /// and is simpler than `than`.
    pub fn attempt(&mut self, choices: Vec<U256>, than: &Tape) -> Option<Tape> {
        if self.exhausted() || crate::choices::shortlex(&choices, &than.choices).is_ge() {
            return None;
        }
        self.attempts += 1;
        (self.check)(&choices).filter(|tape| tape.simpler_than(than))
    }

    pub fn exhausted(&self) -> bool {
        self.attempts >= self.budget
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shrunk {
    pub tape: Tape,
    pub attempts: usize,
    pub improvements: usize,
}

pub struct Shrinker {
    passes: Vec<Box<dyn ShrinkPass>>,
    budget: usize,
}

impl Shrinker {
    pub fn standard(budget: usize) -> Self {
        Self::with_passes(
            vec![
                Box::new(DeleteSpans),
                Box::new(ZeroSpans),
                Box::new(MinimizeChoices),
                Box::new(Redistribute),
                Box::new(DeleteChoices),
            ],
            budget,
        )
    }

    pub fn with_passes(passes: Vec<Box<dyn ShrinkPass>>, budget: usize) -> Self {
        Self { passes, budget }
    }

    pub fn shrink(&self, start: Tape, check: &mut dyn FnMut(&[U256]) -> Option<Tape>) -> Shrunk {
        let mut probe = Probe {
            check,
            attempts: 0,
            budget: self.budget,
        };
        let mut current = start;
        let mut improvements = 0;
        loop {
            let mut improved = false;
            for pass in &self.passes {
                if probe.exhausted() {
                    break;
                }
                if let Some(better) = pass.run(&current, &mut probe) {
                    current = better;
                    improvements += 1;
                    improved = true;
                }
            }
            if !improved || probe.exhausted() {
                break;
            }
        }
        Shrunk {
            tape: current,
            attempts: probe.attempts,
            improvements,
        }
    }
}

#[cfg(test)]
mod tests {
    use femp_core::property::{Command, Location, Machine, Method, Param, Sequence, Ty, Value};

    use super::*;
    use crate::cases::{ArgsGen, SequenceGen};
    use crate::choices::Choices;
    use crate::generators::Registry;

    fn first_failing<T>(draw: impl Fn(&mut Choices) -> T, fails: impl Fn(&T) -> bool) -> Tape {
        (0..10_000)
            .find_map(|seed| {
                let mut choices = Choices::random(seed);
                let case = draw(&mut choices);
                fails(&case).then(|| choices.into_tape())
            })
            .expect("a failing case")
    }

    fn shrink<T>(draw: impl Fn(&mut Choices) -> T, fails: impl Fn(&T) -> bool) -> T {
        let start = first_failing(&draw, &fails);
        let shrunk = Shrinker::standard(5000).shrink(start, &mut |choices| {
            let mut replay = Choices::replay(choices);
            let case = draw(&mut replay);
            fails(&case).then(|| replay.into_tape())
        });
        draw(&mut Choices::replay(&shrunk.tape.choices))
    }

    fn number(value: &Value) -> u128 {
        value.to_string().parse().unwrap()
    }

    #[test]
    fn shrinks_a_threshold_to_its_boundary() {
        let args = ArgsGen::new(vec![
            Registry::standard().build(Ty::Uint(64), "any").unwrap(),
        ]);
        let minimal = shrink(|c| args.draw(c), |values| number(&values[0]) > 1000);
        assert_eq!(minimal[0].to_string(), "1001");
    }

    #[test]
    fn shrinks_the_first_parameter_before_the_second() {
        let registry = Registry::standard();
        let args = ArgsGen::new(vec![
            registry.build(Ty::Uint(8), "any").unwrap(),
            registry.build(Ty::Uint(8), "any").unwrap(),
        ]);
        let minimal = shrink(
            |c| args.draw(c),
            |values| number(&values[0]) + number(&values[1]) > 100,
        );
        assert_eq!(
            (minimal[0].to_string(), minimal[1].to_string()),
            ("0".into(), "101".into())
        );
    }

    #[test]
    fn shrinks_negative_values_toward_zero() {
        let args = ArgsGen::new(vec![
            Registry::standard().build(Ty::Int(32), "any").unwrap(),
        ]);
        let minimal = shrink(
            |c| args.draw(c),
            |values| values[0].to_string().parse::<i64>().unwrap() < -50,
        );
        assert_eq!(minimal[0].to_string(), "-51");
    }

    fn method(name: &str, params: Vec<Param>) -> Method {
        Method {
            name: name.to_string(),
            params,
            effects: Vec::new(),
            location: Location {
                file: "m.fe".into(),
                line: 1,
            },
        }
    }

    #[test]
    fn removes_irrelevant_steps_from_sequences() {
        let amount = Param {
            name: "amount".to_string(),
            label: Some("amount".to_string()),
            ty: Ty::Uint(8),
            generator: "any".to_string(),
        };
        let machine = Machine {
            name: "Counter".to_string(),
            path: Vec::new(),
            init: method("new", Vec::new()),
            commands: vec![
                Command {
                    method: method("add", vec![amount]),
                    weight: 1,
                },
                Command {
                    method: method("noop", Vec::new()),
                    weight: 1,
                },
            ],
            invariants: Vec::new(),
            observations: Vec::new(),
            steps: "1..=20".to_string(),
            runs: None,
            clock: None,
            reuse: false,
            location: Location {
                file: "m.fe".into(),
                line: 1,
            },
        };
        let generator = SequenceGen::for_machine(&Registry::standard(), &machine).unwrap();
        // Fails once three steps add at least 10 each.
        let fails = |sequence: &Sequence| {
            sequence
                .steps
                .iter()
                .filter(|step| step.command == 0 && number(&step.args[0]) >= 10)
                .count()
                >= 3
        };
        let minimal = shrink(|c| generator.draw(c), fails);
        let described: Vec<String> = minimal
            .steps
            .iter()
            .map(|step| {
                format!(
                    "{}:{}",
                    step.command,
                    step.args.first().map_or(String::new(), |v| v.to_string())
                )
            })
            .collect();
        assert_eq!(described, ["0:10", "0:10", "0:10"]);
    }

    #[test]
    fn stops_at_the_budget() {
        let args = ArgsGen::new(vec![
            Registry::standard().build(Ty::Uint(64), "any").unwrap(),
        ]);
        let start = first_failing(|c| args.draw(c), |values| number(&values[0]) > 1000);
        let mut replays = 0;
        let shrunk = Shrinker::standard(3).shrink(start, &mut |choices| {
            replays += 1;
            let mut replay = Choices::replay(choices);
            let values = args.draw(&mut replay);
            (number(&values[0]) > 1000).then(|| replay.into_tape())
        });
        assert_eq!(replays, 3);
        assert_eq!(shrunk.attempts, 3);
    }
}
