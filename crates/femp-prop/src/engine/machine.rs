use femp_core::property::{
    Machine, Param, Record, Sequence, Step, StepStatus, Suite, Trace, Value,
};

use super::targets::{self, Target};
use super::{Engine, format_seed, generated, report};
use crate::cases::SequenceGen;
use crate::choices::{Choices, Tape};
use crate::regress::{Regression, Store, encode_tape};
use crate::report::{
    Arg, Counterexample, EntryKind, EntryReport, FailureReport, ShrinkStats, Skip, Status,
    StepReport, TargetOutcome,
};
use crate::shrink::Shrinker;
use crate::verdict::{Failure, judge_machine};

struct Candidate {
    sequence: Sequence,
    tape: Option<Tape>,
    index: Option<u64>,
}

struct Minimized {
    sequence: Sequence,
    tape: Vec<femp_core::property::U256>,
    traces: Vec<Trace>,
    stats: Option<ShrinkStats>,
}

pub fn run(
    engine: &Engine<'_>,
    suite: &Suite,
    index: usize,
    targets: &mut [Target<'_>],
    store: &mut Store,
) -> EntryReport {
    let machine = &suite.machines[index];
    let mut entry = report(
        suite,
        &machine.name,
        EntryKind::Machine,
        machine.location.to_string(),
    );
    let generator = match SequenceGen::for_machine(engine.registry, machine) {
        Ok(generator) => generator,
        Err(message) => {
            entry.status = Status::Invalid {
                message: format!("{}: {message}", machine.location),
            };
            return entry;
        }
    };
    let selection = targets::select(targets, |executor| executor.supports_machine(index));
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

    let mut candidates = Vec::new();
    if engine.config.regressions {
        match store.load(&suite.module, &machine.name) {
            Ok(stored) => {
                for regression in stored {
                    match replayed(&generator, machine, regression) {
                        Some(candidate) => candidates.push(candidate),
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
    entry.regressions = candidates.len();
    let seed = engine.entry_seed(suite, &machine.name);
    let runs = engine
        .config
        .cases
        .or(machine.runs)
        .unwrap_or(super::DEFAULT_RUNS) as u64;
    candidates.extend((0..runs).map(|run| {
        let (sequence, tape) = generated(seed, run, |choices| generator.draw(choices));
        Candidate {
            sequence,
            tape: Some(tape),
            index: Some(run),
        }
    }));

    let mut failing = None;
    for candidate in candidates {
        let traces = match targets::run(targets, &running, index, &candidate.sequence) {
            Ok(traces) => traces,
            Err(error) => {
                entry.status = Status::Error {
                    message: error.to_string(),
                };
                return entry;
            }
        };
        entry.cases += 1;
        if let Some(failure) = judge_machine(&traces, engine.comparator) {
            failing = Some((candidate, failure));
            break;
        }
    }
    let Some((candidate, failure)) = failing else {
        return entry;
    };

    let case = candidate.index;
    let minimal = minimize(
        engine, index, &generator, targets, &running, candidate, &failure,
    );
    let focus = first_difference(&minimal.traces);
    let outcomes = running
        .iter()
        .zip(&minimal.traces)
        .map(|(target, trace)| TargetOutcome {
            target: targets[*target].label.clone(),
            outcome: describe(machine, trace, focus),
        })
        .collect();
    let mut failure_report = FailureReport {
        kind: failure.kind().into(),
        case,
        seed: format_seed(engine.config.seed),
        counterexample: counterexample(machine, &minimal.sequence),
        outcomes,
        shrink: minimal.stats,
        saved_to: None,
    };
    if engine.config.regressions {
        let regression = Regression {
            failure: failure_report.kind,
            seed: failure_report.seed.clone(),
            case,
            choices: encode_tape(&minimal.tape),
            counterexample: failure_report.counterexample.clone(),
            outcomes: failure_report.outcomes.clone(),
            recorded: engine.recorded(),
        };
        match store.save(&suite.module, &machine.name, regression) {
            Ok(path) => failure_report.saved_to = Some(path.display().to_string()),
            Err(error) => entry.notes.push(error.to_string()),
        }
    }
    entry.status = Status::Failed(Box::new(failure_report));
    entry
}

fn minimize(
    engine: &Engine<'_>,
    index: usize,
    generator: &SequenceGen,
    targets: &mut [Target<'_>],
    running: &[usize],
    candidate: Candidate,
    failure: &Failure,
) -> Minimized {
    let wanted = failure.kind();
    let mut sequence = candidate.sequence;
    let mut tape = candidate
        .tape
        .as_ref()
        .map(|tape| tape.choices.clone())
        .unwrap_or_default();
    let mut stats = None;
    if let (true, Some(start)) = (engine.config.shrink, candidate.tape) {
        let shrinker = Shrinker::standard(engine.config.max_shrinks);
        let shrunk = shrinker.shrink(start, &mut |choices| {
            let mut replay = Choices::replay(choices);
            let attempt = generator.draw(&mut replay);
            let traces = targets::run(targets, running, index, &attempt).ok()?;
            judge_machine(&traces, engine.comparator)
                .filter(|failure| failure.kind() == wanted)
                .map(|_| replay.into_tape())
        });
        sequence = generator.draw(&mut Choices::replay(&shrunk.tape.choices));
        tape = shrunk.tape.choices;
        stats = Some(ShrinkStats {
            attempts: shrunk.attempts,
            improvements: shrunk.improvements,
        });
    }
    let traces = targets::run(targets, running, index, &sequence).unwrap_or_default();
    Minimized {
        sequence,
        tape,
        traces,
        stats,
    }
}

fn args(params: &[Param], values: &[Value]) -> Vec<Arg> {
    params
        .iter()
        .zip(values)
        .map(|(param, value)| Arg {
            name: param.name.clone(),
            value: value.to_string(),
        })
        .collect()
}

fn counterexample(machine: &Machine, sequence: &Sequence) -> Counterexample {
    Counterexample::Sequence {
        init: machine.init.name.clone(),
        init_args: args(&machine.init.params, &sequence.init),
        steps: sequence
            .steps
            .iter()
            .map(|step| {
                let command = &machine.commands[step.command].method;
                StepReport {
                    command: command.name.clone(),
                    args: args(&command.params, &step.args),
                    advance: step.advance,
                }
            })
            .collect(),
    }
}

// The first record at which some target's trace differs from the first.
fn first_difference(traces: &[Trace]) -> Option<usize> {
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

fn describe(machine: &Machine, trace: &Trace, focus: Option<usize>) -> String {
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

// A stored regression as a candidate: regenerated from its tape when that
// still yields the stored sequence, or else from the sequence itself.
fn replayed(
    generator: &SequenceGen,
    machine: &Machine,
    regression: &Regression,
) -> Option<Candidate> {
    let Counterexample::Sequence {
        init_args, steps, ..
    } = &regression.counterexample
    else {
        return None;
    };
    let parse = |params: &[Param], stored: &[Arg]| -> Option<Vec<Value>> {
        (params.len() == stored.len())
            .then(|| {
                params
                    .iter()
                    .zip(stored)
                    .map(|(param, arg)| {
                        (param.name == arg.name)
                            .then(|| Value::parse(param.ty, &arg.value))
                            .flatten()
                    })
                    .collect()
            })
            .flatten()
    };
    let init = parse(&machine.init.params, init_args)?;
    let steps = steps
        .iter()
        .map(|step| {
            let command = machine
                .commands
                .iter()
                .position(|command| command.method.name == step.command)?;
            Some(Step {
                command,
                args: parse(&machine.commands[command].method.params, &step.args)?,
                advance: step.advance,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let sequence = Sequence { init, steps };
    let tape = regression.tape().and_then(|choices| {
        let mut replay = Choices::replay(&choices);
        (generator.draw(&mut replay) == sequence).then(|| replay.into_tape())
    });
    Some(Candidate {
        sequence,
        tape,
        index: None,
    })
}
