//! Builds the case generators of a suite's entry points: argument lists for
//! properties and operation sequences for machines.

use femp_core::property::{Machine, Param, Property, Sequence, Step, Ty, U256, Value};

use crate::choices::Choices;
use crate::generators::{Registry, Shared};
use crate::spec::{self, Spec};

const STEP_SPAN: &str = "step";

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
}

#[derive(Debug)]
pub struct SequenceGen {
    init: ArgsGen,
    commands: Vec<CommandGen>,
    min_steps: usize,
    max_steps: usize,
    clock: Option<Shared>,
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
            commands,
            min_steps,
            max_steps,
            clock,
        })
    }

    /// Each step is one span with the same layout, so shrinking can delete
    /// whole steps without misaligning the rest. A step starts with a flag
    /// whose simplest value, 0, ends the sequence once it is long enough.
    pub fn draw(&self, choices: &mut Choices) -> Sequence {
        let init = self.init.draw(choices);
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
            let args = self.commands[command].args.draw(choices);
            let advance = self
                .clock
                .as_ref()
                .map_or(0, |clock| clock.draw(choices).to_u64_bits());
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
            location: Location {
                file: "model.fe".into(),
                line: 1,
            },
        }
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
