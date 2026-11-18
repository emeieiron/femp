//! Shrinking edits a failing case's choice tape and keeps any edit that is
//! simpler (shorter, or equally long and smaller choice by choice) and still
//! fails the same way. Only strictly simpler tapes are accepted, so shrinking
//! always terminates; a budget bounds the number of replays.

mod passes;

use femp_core::property::U256;

use crate::choices::Tape;

pub use passes::{
    CarryClock, DeleteChoices, DeleteSpans, DrawFresh, MinimizeChoices, Redistribute, Rescale,
    ZeroSpans,
};

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
                Box::new(DrawFresh),
                Box::new(DeleteSpans),
                Box::new(CarryClock),
                Box::new(ZeroSpans),
                Box::new(Rescale),
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

    fn param(name: &str) -> Param {
        Param {
            name: name.to_string(),
            label: Some(name.to_string()),
            ty: Ty::Uint(64),
            generator: "any".to_string(),
        }
    }

    /// A vesting contract's run as `femp prop` draws it: whole-type
    /// arguments that may repeat earlier ones, and a clock that each step
    /// advances before its message.
    fn vesting() -> Machine {
        let command = |name: &str, params: Vec<Param>| Command {
            method: method(name, params),
            weight: 1,
        };
        Machine {
            name: "Vesting".to_string(),
            path: Vec::new(),
            init: method(
                "init",
                vec![param("total"), param("start"), param("cliff"), param("duration")],
            ),
            commands: vec![
                command("VestedAt", vec![param("timestamp")]),
                command("ClaimableAt", vec![param("timestamp")]),
                command("Claimable", Vec::new()),
                command("Claimed", Vec::new()),
                command("Claim", vec![param("amount")]),
            ],
            invariants: Vec::new(),
            observations: Vec::new(),
            steps: "1..=16".to_string(),
            runs: None,
            clock: Some("any".to_string()),
            reuse: true,
            location: Location {
                file: "lib.fe".into(),
                line: 1,
            },
        }
    }

    const CLAIM: usize = 4;

    /// Whether claims pass the total when `Claim` checks the amount only
    /// against what has vested and forgets earlier claims.
    fn forgets_earlier_claims(sequence: &Sequence) -> bool {
        let [total, start, cliff, duration] = [0, 1, 2, 3].map(|i| number(&sequence.init[i]));
        let max = u128::from(u64::MAX);
        if total == 0 || duration == 0 || cliff > duration || start + duration > max {
            return false;
        }
        let vested_at = |now: u128| {
            if now < start + cliff {
                0
            } else if now - start >= duration {
                total
            } else {
                total * (now - start) / duration
            }
        };
        let (mut now, mut claimed) = (0u128, 0u128);
        for step in &sequence.steps {
            now += u128::from(step.advance);
            if step.command != CLAIM {
                continue;
            }
            let amount = number(&step.args[0]);
            if amount == 0 || amount > vested_at(now & max) || claimed + amount > max {
                continue;
            }
            claimed += amount;
            if claimed > total {
                return true;
            }
        }
        false
    }

    fn shown(machine: &Machine, sequence: &Sequence) -> String {
        let init: Vec<String> = sequence.init.iter().map(ToString::to_string).collect();
        let mut text = format!("init({})", init.join(", "));
        for step in &sequence.steps {
            let args: Vec<String> = step.args.iter().map(ToString::to_string).collect();
            text.push_str(&format!(
                " {}({}) +{}",
                machine.commands[step.command].method.name,
                args.join(", "),
                step.advance
            ));
        }
        text
    }

    /// Shrinks a failing run of the vesting model with the replays
    /// `femp prop` allows by default.
    fn shrink_vesting(start: Tape) -> (String, usize) {
        let machine = vesting();
        let generator = SequenceGen::for_machine(&Registry::standard(), &machine).unwrap();
        let shrunk = Shrinker::standard(2000).shrink(start, &mut |choices| {
            let mut replay = Choices::replay(choices);
            let sequence = generator.draw(&mut replay);
            forgets_earlier_claims(&sequence).then(|| replay.into_tape())
        });
        let minimal = generator.draw(&mut Choices::replay(&shrunk.tape.choices));
        (shown(&machine, &minimal), shrunk.attempts)
    }

    const TWO_CLAIMS: &str = "init(1, 0, 0, 1) Claim(1) +1 Claim(1) +0";

    #[test]
    fn shrinks_a_run_whose_steps_tie_values_together() {
        // Where `femp prop --seed 0x2a` stopped: a claim of one more than
        // the first step's clock advance, which is one less than the
        // total, after a step that only lets time pass. Lowering any one
        // value, or deleting that step, no longer fails.
        let stopped: Vec<U256> = [
            0u64, 1271573, 0, 0, 0, 0, 0, 1, 0, 3, 0, 1271572, 1, 4, 2, 4, 0, 0, 1, 4, 0, 2, 0, 0,
            0,
        ]
        .map(U256::from)
        .to_vec();
        let machine = vesting();
        let generator = SequenceGen::for_machine(&Registry::standard(), &machine).unwrap();
        let mut replay = Choices::replay(&stopped);
        let sequence = generator.draw(&mut replay);
        assert_eq!(
            shown(&machine, &sequence),
            "init(1271573, 0, 0, 1) Claimed() +1271572 Claim(1271573) +0 Claim(2) +0"
        );
        assert!(forgets_earlier_claims(&sequence));
        let (minimal, attempts) = shrink_vesting(replay.into_tape());
        assert_eq!(minimal, TWO_CLAIMS, "after {attempts} replays");
    }

    #[test]
    fn a_step_that_only_lets_time_pass_gives_its_time_to_the_next() {
        let machine = vesting();
        let generator = SequenceGen::for_machine(&Registry::standard(), &machine).unwrap();
        // Each value is drawn fresh: a zero, then its magnitude.
        let tape: Vec<U256> = [
            0u64, 1, 0, 0, 0, 0, 0, 1, // init(1, 0, 0, 1)
            0, 0, 0, 7, 0, 1, // VestedAt(7) +1
            1, 4, 0, 1, 0, 0, // Claim(1) +0
            1, 4, 0, 1, 0, 0, // Claim(1) +0
            0,
        ]
        .map(U256::from)
        .to_vec();
        let mut replay = Choices::replay(&tape);
        let sequence = generator.draw(&mut replay);
        assert_eq!(
            shown(&machine, &sequence),
            "init(1, 0, 0, 1) VestedAt(7) +1 Claim(1) +0 Claim(1) +0"
        );
        // Deleting the first step alone leaves both claims at time zero.
        let shrunk = Shrinker::with_passes(vec![Box::new(DeleteSpans), Box::new(CarryClock)], 100)
            .shrink(replay.into_tape(), &mut |choices| {
                let mut replay = Choices::replay(choices);
                let sequence = generator.draw(&mut replay);
                forgets_earlier_claims(&sequence).then(|| replay.into_tape())
            });
        let minimal = generator.draw(&mut Choices::replay(&shrunk.tape.choices));
        assert_eq!(shown(&machine, &minimal), TWO_CLAIMS);
    }

    #[test]
    fn shrinks_every_failing_vesting_run_to_two_claims_of_one() {
        let generator = SequenceGen::for_machine(&Registry::standard(), &vesting()).unwrap();
        let mut failing = 0;
        for seed in 0..3000 {
            let mut choices = Choices::random(seed);
            if !forgets_earlier_claims(&generator.draw(&mut choices)) {
                continue;
            }
            failing += 1;
            let (minimal, attempts) = shrink_vesting(choices.into_tape());
            assert_eq!(minimal, TWO_CLAIMS, "seed {seed}, after {attempts} replays");
        }
        assert!(failing > 20, "{failing} failing runs");
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
