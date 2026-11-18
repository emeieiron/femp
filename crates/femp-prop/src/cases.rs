//! Builds the case generators of a suite's entry points: argument lists for
//! properties and operation sequences for machines.

use femp_core::property::{Machine, Param, Property, Sequence, Step, Ty, U256, Value};

use crate::choices::Choices;
use crate::generators::{Registry, Shared};
use femp_spec::domain::{self as spec, Spec};

const STEP_SPAN: &str = "step";
/// The span of one value a run draws, which shrinking can lower together
/// with others.
pub const VALUE_SPAN: &str = "value";
/// The span of a step's clock advance, a value which shrinking can also
/// carry over to the next step when it deletes the step.
pub const CLOCK_SPAN: &str = "clock";

#[derive(Debug)]
pub struct ArgsGen {
    params: Vec<Shared>,
}

impl ArgsGen {
    pub fn new(params: Vec<Shared>) -> Self {
        Self { params }
    }

    pub fn for_params(registry: &Registry, params: &[Param]) -> Result<Self, String> {
        params
            .iter()
            .map(|param| {
                registry
                    .build(param.ty, &param.generator)
                    .map_err(|error| format!("parameter `{}`: {error}", param.name))
            })
            .collect::<Result<_, _>>()
            .map(Self::new)
    }

    pub fn for_property(registry: &Registry, property: &Property) -> Result<Self, String> {
        Self::for_params(registry, &property.params)
    }

    pub fn draw(&self, choices: &mut Choices) -> Vec<Value> {
        self.params
            .iter()
            .map(|param| param.draw(choices))
            .collect()
    }

    /// Draws the arguments, each marked in `reusable` possibly as a value of
    /// `pool`, and adds them to it.
    fn draw_reusing(
        &self,
        reusable: &[bool],
        choices: &mut Choices,
        pool: &mut Vec<Value>,
    ) -> Vec<Value> {
        self.params
            .iter()
            .zip(reusable)
            .map(|(param, reusable)| draw_value(param, *reusable, choices, pool))
            .collect()
    }

    /// The number of distinct argument lists, when every parameter's domain
    /// is small and their product fits in a `u64`.
    pub fn domain_size(&self) -> Option<u64> {
        self.params.iter().try_fold(1u64, |product, param| {
            product.checked_mul(param.domain_size()?)
        })
    }

    /// The tape of the `index`-th argument list in enumeration order, the
    /// last parameter varying fastest.
    pub fn tape_for(&self, mut index: u64) -> Vec<U256> {
        let mut ranks = vec![0; self.params.len()];
        for (rank, param) in ranks.iter_mut().zip(&self.params).rev() {
            let size = param.domain_size().unwrap_or(1).max(1);
            *rank = index % size;
            index /= size;
        }
        self.params
            .iter()
            .zip(ranks)
            .flat_map(|(param, rank)| param.choices_for(rank))
            .collect()
    }
}

#[derive(Debug)]
struct CommandGen {
    weight: u64,
    args: ArgsGen,
    /// Which arguments may repeat a value drawn before them.
    reusable: Vec<bool>,
}

#[derive(Debug)]
pub struct SequenceGen {
    init: ArgsGen,
    init_reusable: Vec<bool>,
    commands: Vec<CommandGen>,
    min_steps: usize,
    max_steps: usize,
    clock: Option<Shared>,
    clock_reusable: bool,
}

/// How a reusable value is drawn: fresh, which is simplest, or as a value
/// drawn before it in the run, as it is or one above or below it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reuse {
    Fresh,
    Same,
    Above,
    Below,
}

impl Reuse {
    const ALL: [Reuse; 4] = [Reuse::Fresh, Reuse::Same, Reuse::Above, Reuse::Below];

    /// Fresh half the time, the same value a quarter, and one off the rest.
    fn draw(choices: &mut Choices) -> Self {
        let choice = choices.draw(U256::from(3u8), |rng| {
            U256::from(match rng.below_u64(8) {
                0..=3 => 0u8,
                4 | 5 => 1,
                6 => 2,
                _ => 3,
            })
        });
        Self::ALL[choice.as_limbs()[0] as usize]
    }

    fn apply(self, value: Value) -> Value {
        let ty = value.ty();
        let one = U256::from(1u8);
        let ordinal = value.ordinal();
        match self {
            Reuse::Fresh | Reuse::Same => value,
            Reuse::Above if ordinal < ty.max_ordinal() => Value::from_ordinal(ty, ordinal + one),
            Reuse::Below if ordinal > U256::ZERO => Value::from_ordinal(ty, ordinal - one),
            Reuse::Above | Reuse::Below => value,
        }
    }
}

/// Draws one value, which may repeat one of `pool`, the values drawn
/// before it, and adds it to them. A repeated value notes how to draw it
/// afresh, so that shrinking can untie it from the value it repeats.
fn draw_value(
    generator: &Shared,
    reusable: bool,
    choices: &mut Choices,
    pool: &mut Vec<Value>,
) -> Value {
    draw_value_as(VALUE_SPAN, generator, reusable, choices, pool)
}

fn draw_value_as(
    label: &'static str,
    generator: &Shared,
    reusable: bool,
    choices: &mut Choices,
    pool: &mut Vec<Value>,
) -> Value {
    choices.start_span(label);
    let value = if reusable {
        let start = choices.position();
        let how = Reuse::draw(choices);
        let ty = generator.ty();
        let earlier: Vec<Value> = pool
            .iter()
            .filter(|value| value.ty() == ty)
            .copied()
            .collect();
        if how == Reuse::Fresh || earlier.is_empty() {
            generator.draw(choices)
        } else {
            let index =
                choices.draw_index(earlier.len(), |rng| rng.below_u64(earlier.len() as u64));
            let value = how.apply(earlier[index]);
            if let Some(fresh) = generator.choices_of(value) {
                let mut drawn = vec![U256::ZERO];
                drawn.extend(fresh);
                choices.rewrite(start, drawn);
            }
            value
        }
    } else {
        generator.draw(choices)
    };
    choices.end_span();
    pool.push(value);
    value
}

/// Which of `params` may repeat earlier values in a machine that reuses
/// them: those drawn from their whole type, where any value of it fits.
fn reusable(machine: &Machine, params: &[Param]) -> Vec<bool> {
    params
        .iter()
        .map(|param| machine.reuse && param.generator == "any" && param.ty != Ty::Bool)
        .collect()
}

impl SequenceGen {
    pub fn for_machine(registry: &Registry, machine: &Machine) -> Result<Self, String> {
        let init = ArgsGen::for_params(registry, &machine.init.params)
            .map_err(|error| format!("`{}`: {error}", machine.init.name))?;
        let commands = machine
            .commands
            .iter()
            .map(|command| {
                Ok(CommandGen {
                    weight: command.weight.max(1) as u64,
                    args: ArgsGen::for_params(registry, &command.method.params)
                        .map_err(|error| format!("`{}`: {error}", command.method.name))?,
                    reusable: reusable(machine, &command.method.params),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if commands.is_empty() {
            return Err("a machine needs at least one `#[command]`".to_string());
        }
        let (min_steps, max_steps) =
            count_range(&machine.steps).map_err(|error| format!("`steps`: {error}"))?;
        let clock = machine
            .clock
            .as_deref()
            .map(|text| registry.build(Ty::Uint(64), text))
            .transpose()
            .map_err(|error| format!("`clock`: {error}"))?;
        Ok(Self {
            init,
            init_reusable: reusable(machine, &machine.init.params),
            commands,
            min_steps,
            max_steps,
            clock,
            clock_reusable: machine.reuse && machine.clock.as_deref() == Some("any"),
        })
    }

    /// Each step is one span with the same layout, so shrinking can delete
    /// whole steps without misaligning the rest. A step starts with a flag
    /// whose simplest value, 0, ends the sequence once it is long enough.
    pub fn draw(&self, choices: &mut Choices) -> Sequence {
        let mut pool = Vec::new();
        let init = self
            .init
            .draw_reusing(&self.init_reusable, choices, &mut pool);
        let stop_odds = 1 + (self.max_steps - self.min_steps) as u64 / 2;
        let total: u64 = self.commands.iter().map(|command| command.weight).sum();
        let mut steps = Vec::new();
        for index in 0..self.max_steps {
            choices.start_span(STEP_SPAN);
            let required = index < self.min_steps;
            let more = choices.draw(U256::from(1u8), |rng| {
                U256::from((required || !rng.chance(1, stop_odds)) as u8)
            });
            if more == U256::ZERO && !required {
                choices.end_span();
                break;
            }
            let pick = choices.draw(U256::from(total - 1), |rng| {
                U256::from(rng.below_u64(total))
            });
            let command = self.pick(pick.as_limbs()[0]);
            let generator = &self.commands[command];
            let args = generator
                .args
                .draw_reusing(&generator.reusable, choices, &mut pool);
            let advance = self.clock.as_ref().map_or(0, |clock| {
                draw_value_as(CLOCK_SPAN, clock, self.clock_reusable, choices, &mut pool)
                    .to_u64_bits()
            });
            choices.end_span();
            steps.push(Step {
                command,
                args,
                advance,
            });
        }
        Sequence { init, steps }
    }

    fn pick(&self, mut point: u64) -> usize {
        for (index, command) in self.commands.iter().enumerate() {
            if point < command.weight {
                return index;
            }
            point -= command.weight;
        }
        self.commands.len() - 1
    }
}

/// Parses a range of counts such as `1..=16`.
pub fn count_range(text: &str) -> Result<(usize, usize), String> {
    let Spec::Range { lo, hi, inclusive } = spec::parse(text)? else {
        return Err(format!("`{text}` is not a range such as `1..=16`"));
    };
    let number = |bound: &str| -> Result<usize, String> {
        bound
            .replace('_', "")
            .parse()
            .map_err(|_| format!("`{bound}` is not a count"))
    };
    let lo = number(&lo)?;
    let hi = match (number(&hi)?, inclusive) {
        (hi, true) => Some(hi),
        (hi, false) => hi.checked_sub(1),
    };
    match hi {
        Some(hi) if hi >= lo => Ok((lo, hi)),
        _ => Err(format!("the range `{text}` is empty")),
    }
}

#[cfg(test)]
mod tests {
    use femp_core::property::{Command, Location, Method};

    use super::*;

    fn param(name: &str, ty: Ty, generator: &str) -> Param {
        Param {
            name: name.to_string(),
            label: Some(name.to_string()),
            ty,
            generator: generator.to_string(),
        }
    }

    fn method(name: &str, params: Vec<Param>) -> Method {
        Method {
            name: name.to_string(),
            params,
            effects: Vec::new(),
            location: Location {
                file: "model.fe".into(),
                line: 1,
            },
        }
    }

    fn machine() -> Machine {
        Machine {
            name: "Counter".to_string(),
            path: Vec::new(),
            init: method("new", vec![param("start", Ty::Uint(8), "0..=9")]),
            commands: vec![
                Command {
                    method: method("add", vec![param("amount", Ty::Uint(8), "any")]),
                    weight: 3,
                },
                Command {
                    method: method("reset", Vec::new()),
                    weight: 1,
                },
            ],
            invariants: Vec::new(),
            observations: Vec::new(),
            steps: "2..=6".to_string(),
            runs: None,
            clock: Some("0..=100".to_string()),
            reuse: false,
            location: Location {
                file: "model.fe".into(),
                line: 1,
            },
        }
    }

    /// A deployment's total, then claims of an amount, both from their
    /// whole type, as a contract's runs draw them.
    fn reusing() -> Machine {
        Machine {
            init: method("init", vec![param("total", Ty::Uint(64), "any")]),
            commands: vec![Command {
                method: method("claim", vec![param("amount", Ty::Uint(64), "any")]),
                weight: 1,
            }],
            steps: "1..=1".to_string(),
            clock: None,
            reuse: true,
            ..machine()
        }
    }

    #[test]
    fn reusing_runs_repeat_earlier_values_and_their_neighbours() {
        let generator = SequenceGen::for_machine(&Registry::standard(), &reusing()).unwrap();
        let (mut same, mut near) = (0, 0);
        for seed in 0..400 {
            let mut choices = Choices::random(seed);
            let sequence = generator.draw(&mut choices);
            let total = sequence.init[0].ordinal();
            let amount = sequence.steps[0].args[0].ordinal();
            if total > U256::from(1u8) && amount == total {
                same += 1;
            }
            if total > U256::from(1u8)
                && (amount == total + U256::from(1u8) || amount + U256::from(1u8) == total)
            {
                near += 1;
            }
            // The tape regenerates the same run.
            let tape = choices.into_tape();
            assert_eq!(
                generator.draw(&mut Choices::replay(&tape.choices)),
                sequence
            );
        }
        assert!(same > 40, "{same} of 400 claims repeat the total");
        assert!(near > 20, "{near} of 400 claims are next to the total");
    }

    #[test]
    fn repeated_values_note_how_to_draw_them_afresh() {
        let generator = SequenceGen::for_machine(&Registry::standard(), &reusing()).unwrap();
        let mut rewritten = 0;
        for seed in 0..200 {
            let mut choices = Choices::random(seed);
            let sequence = generator.draw(&mut choices);
            let tape = choices.into_tape();
            let mut fresh = tape.choices.clone();
            for rewrite in tape.rewrites.iter().rev() {
                fresh.splice(rewrite.start..rewrite.end, rewrite.choices.iter().copied());
                rewritten += 1;
            }
            // The same run, with nothing left that repeats a value.
            let mut replay = Choices::replay(&fresh);
            assert_eq!(generator.draw(&mut replay), sequence);
            assert!(replay.into_tape().rewrites.is_empty());
        }
        assert!(rewritten > 40, "{rewritten} values repeated");
    }

    #[test]
    fn the_simplest_reusing_run_draws_fresh_zeros() {
        let generator = SequenceGen::for_machine(&Registry::standard(), &reusing()).unwrap();
        let sequence = generator.draw(&mut Choices::replay(&[]));
        assert_eq!(sequence.init[0].ordinal(), U256::ZERO);
        assert_eq!(sequence.steps[0].args[0].ordinal(), U256::ZERO);
    }

    #[test]
    fn enumerates_small_domains() {
        let registry = Registry::standard();
        let args = ArgsGen::new(vec![
            registry.build(Ty::Bool, "any").unwrap(),
            registry.build(Ty::Uint(8), "0..=2").unwrap(),
        ]);
        assert_eq!(args.domain_size(), Some(6));
        let listed: Vec<String> = (0..6)
            .map(|index| {
                let values = args.draw(&mut Choices::replay(&args.tape_for(index)));
                format!("{}/{}", values[0], values[1])
            })
            .collect();
        assert_eq!(
            listed,
            [
                "false/0", "false/1", "false/2", "true/0", "true/1", "true/2"
            ]
        );
        let wide = ArgsGen::new(vec![registry.build(Ty::Uint(64), "any").unwrap()]);
        assert_eq!(wide.domain_size(), None);
    }

    #[test]
    fn sequences_respect_their_bounds() {
        let generator = SequenceGen::for_machine(&Registry::standard(), &machine()).unwrap();
        for seed in 0..50 {
            let sequence = generator.draw(&mut Choices::random(seed));
            assert!((2..=6).contains(&sequence.steps.len()));
            assert!(sequence.steps.iter().all(|step| step.advance <= 100));
            assert_eq!(sequence.init.len(), 1);
        }
    }

    #[test]
    fn the_empty_tape_is_the_shortest_sequence_of_first_commands() {
        let generator = SequenceGen::for_machine(&Registry::standard(), &machine()).unwrap();
        let mut choices = Choices::replay(&[]);
        let sequence = generator.draw(&mut choices);
        assert_eq!(sequence.steps.len(), 2);
        assert!(
            sequence
                .steps
                .iter()
                .all(|step| step.command == 0 && step.advance == 0)
        );
        let tape = choices.into_tape();
        assert_eq!(
            tape.spans
                .iter()
                .filter(|span| span.label == STEP_SPAN)
                .count(),
            3
        );
    }

    #[test]
    fn parses_count_ranges() {
        assert_eq!(count_range("1..=16"), Ok((1, 16)));
        assert_eq!(count_range("0..4"), Ok((0, 3)));
        assert!(count_range("3..=1").is_err());
        assert!(count_range("0..0").is_err());
        assert!(count_range("any").is_err());
    }
}
