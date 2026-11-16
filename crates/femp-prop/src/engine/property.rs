use femp_core::property::{Outcome, Property, PropertyKind, Suite, U256, Value};

use super::targets::{self, Target};
use super::{BATCH, EXHAUSTIVE_LIMIT, Engine, format_seed, generated, report};
use crate::cases::ArgsGen;
use crate::choices::{Choices, Tape};
use crate::regress::{Regression, Store, encode_tape};
use crate::report::{
    Arg, Counterexample, EntryKind, EntryReport, FailureReport, ShrinkStats, Skip, Status,
    TargetOutcome,
};
use crate::shrink::Shrinker;
use crate::verdict::{Failure, judge_property, rejected};

struct Case {
    args: Vec<Value>,
    /// The tape that regenerates `args`, when there is one to shrink.
    tape: Option<Tape>,
    /// Its position among generated cases; `None` for a stored regression.
    index: Option<u64>,
}

pub fn run(
    engine: &Engine<'_>,
    suite: &Suite,
    index: usize,
    targets: &mut [Target<'_>],
    store: &mut Store,
) -> EntryReport {
    let property = &suite.properties[index];
    let kind = match property.kind {
        PropertyKind::Holds => EntryKind::Property,
        PropertyKind::Agrees => EntryKind::Differential,
        PropertyKind::Spec => EntryKind::Spec,
    };
    let mut entry = report(suite, &property.name, kind, property.location.to_string());
    let generator = match ArgsGen::for_property(engine.registry, property) {
        Ok(generator) => generator,
        Err(message) => {
            entry.status = Status::Invalid {
                message: format!("{}: {message}", property.location),
            };
            return entry;
        }
    };
    let selection = targets::select(targets, |executor| executor.supports_property(index));
    entry.ran_on = selection
        .running
        .iter()
        .map(|target| targets[*target].label.clone())
        .collect();
    entry.skipped = selection
        .skipped
        .into_iter()
        .map(|(target, reason)| Skip { target, reason })
        .collect();
    if selection.running.is_empty() {
        entry.status = Status::Skipped {
            reason: "no target can run it".to_string(),
        };
        return entry;
    }
    let running = selection.running;

    let mut cases = Vec::new();
    if engine.config.regressions {
        match store.load(&suite.module, &property.name) {
            Ok(stored) => {
                for regression in stored {
                    match replayed(&generator, property, regression) {
                        Some(case) => cases.push(case),
                        None => entry.notes.push(
                            "a stored regression no longer matches the declaration and was skipped"
                                .to_string(),
                        ),
                    }
                }
            }
            Err(error) => entry.notes.push(error.to_string()),
        }
    }
    entry.regressions = cases.len();

    let seed = engine.entry_seed(suite, &property.name);
    let budget = engine
        .config
        .cases
        .or(property.cases)
        .unwrap_or(super::DEFAULT_CASES) as u64;
    let exhaustive = generator
        .domain_size()
        .filter(|size| *size <= budget || (engine.config.exhaustive && *size <= EXHAUSTIVE_LIMIT));
    entry.exhaustive = exhaustive.is_some();
    let total = exhaustive.unwrap_or(budget);
    let make = |index: u64| -> Case {
        let (args, tape) = match exhaustive {
            Some(_) => {
                let mut choices = Choices::replay(&generator.tape_for(index));
                let args = generator.draw(&mut choices);
                (args, choices.into_tape())
            }
            None => generated(seed, index, |choices| generator.draw(choices)),
        };
        Case {
            args,
            tape: Some(tape),
            index: Some(index),
        }
    };

    let mut next = 0u64;
    let mut failing = None;
    'batches: loop {
        while cases.len() < BATCH && next < total {
            cases.push(make(next));
            next += 1;
        }
        if cases.is_empty() {
            break;
        }
        let batch: Vec<Vec<Value>> = cases.iter().map(|case| case.args.clone()).collect();
        let outcomes = match targets::call(targets, &running, index, &batch) {
            Ok(outcomes) => outcomes,
            Err(error) => {
                entry.status = Status::Error {
                    message: error.to_string(),
                };
                return entry;
            }
        };
        for (case, outcomes) in cases.drain(..).zip(outcomes) {
            entry.cases += 1;
            if rejected(property.kind, &outcomes) {
                entry.rejected += 1;
                continue;
            }
            if let Some(failure) = judge_property(property.kind, &outcomes, engine.comparator) {
                failing = Some((case, failure));
                break 'batches;
            }
        }
    }

    let Some((case, failure)) = failing else {
        if entry.cases > 0 && entry.rejected == entry.cases {
            entry.notes.push(
                "`requires` rejected every case; its generators never reach the spec's domain"
                    .to_string(),
            );
        }
        return entry;
    };
    let case_index = case.index;
    let minimal = minimize(
        engine, property, index, &generator, targets, &running, case, &failure,
    );
    let counterexample = Counterexample::Args {
        args: property
            .params
            .iter()
            .zip(&minimal.args)
            .map(|(param, value)| Arg {
                name: param.name.clone(),
                value: value.to_string(),
            })
            .collect(),
    };
    let outcomes = running
        .iter()
        .zip(&minimal.outcomes)
        .map(|(target, outcome)| TargetOutcome {
            target: targets[*target].label.clone(),
            outcome: outcome.to_string(),
        })
        .collect();
    let mut failure_report = FailureReport {
        kind: failure.kind().into(),
        case: case_index,
        seed: format_seed(engine.config.seed),
        counterexample,
        outcomes,
        shrink: minimal.stats,
        saved_to: None,
    };
    if engine.config.regressions {
        let regression = Regression {
            failure: failure_report.kind,
            seed: failure_report.seed.clone(),
            case: case_index,
            choices: encode_tape(&minimal.tape),
            counterexample: failure_report.counterexample.clone(),
            outcomes: failure_report.outcomes.clone(),
            recorded: engine.recorded(),
        };
        match store.save(&suite.module, &property.name, regression) {
            Ok(path) => failure_report.saved_to = Some(path.display().to_string()),
            Err(error) => entry.notes.push(error.to_string()),
        }
    }
    entry.status = Status::Failed(Box::new(failure_report));
    entry
}

struct Minimized {
    args: Vec<Value>,
    tape: Vec<U256>,
    outcomes: Vec<Outcome>,
    stats: Option<ShrinkStats>,
}

// Shrinks the failing case when it has a tape and shrinking is on, then
// runs the result once more for the report.
#[allow(clippy::too_many_arguments)]
fn minimize(
    engine: &Engine<'_>,
    property: &Property,
    index: usize,
    generator: &ArgsGen,
    targets: &mut [Target<'_>],
    running: &[usize],
    case: Case,
    failure: &Failure,
) -> Minimized {
    let wanted = failure.kind();
    let mut args = case.args;
    let mut tape = case
        .tape
        .as_ref()
        .map(|tape| tape.choices.clone())
        .unwrap_or_default();
    let mut stats = None;
    if let (true, Some(start)) = (engine.config.shrink, case.tape) {
        let shrinker = Shrinker::standard(engine.config.max_shrinks);
        let shrunk = shrinker.shrink(start, &mut |choices| {
            let mut replay = Choices::replay(choices);
            let candidate = generator.draw(&mut replay);
            let outcomes = targets::call(targets, running, index, &[candidate]).ok()?;
            judge_property(property.kind, &outcomes[0], engine.comparator)
                .filter(|failure| failure.kind() == wanted)
                .map(|_| replay.into_tape())
        });
        args = generator.draw(&mut Choices::replay(&shrunk.tape.choices));
        tape = shrunk.tape.choices;
        stats = Some(ShrinkStats {
            attempts: shrunk.attempts,
            improvements: shrunk.improvements,
        });
    }
    let outcomes = targets::call(targets, running, index, std::slice::from_ref(&args))
        .map(|mut rows| rows.remove(0))
        .unwrap_or_default();
    Minimized {
        args,
        tape,
        outcomes,
        stats,
    }
}

// A stored regression as a case: regenerated from its tape when that still
// yields the stored arguments, or else from the arguments themselves.
fn replayed(generator: &ArgsGen, property: &Property, regression: &Regression) -> Option<Case> {
    let Counterexample::Args { args: stored } = &regression.counterexample else {
        return None;
    };
    if stored.len() != property.params.len() {
        return None;
    }
    let values: Option<Vec<Value>> = property
        .params
        .iter()
        .zip(stored)
        .map(|(param, arg)| {
            (arg.name == param.name)
                .then(|| Value::parse(param.ty, &arg.value))
                .flatten()
        })
        .collect();
    let values = values?;
    let tape = regression.tape().and_then(|choices| {
        let mut replay = Choices::replay(&choices);
        (generator.draw(&mut replay) == values).then(|| replay.into_tape())
    });
    Some(Case {
        args: values,
        tape,
        index: None,
    })
}
