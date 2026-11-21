use femp_core::Result;
use femp_core::property::{
    Command, Executor, Halt, Location, Machine, Method, Observation, Outcome, PanicKind, Param,
    Property, PropertyKind, Record, Sequence, StepStatus, Suite, Support, Trace, Ty, Value,
};

use super::*;
use crate::compare::Normalized;
use crate::report::{Counterexample, FailureKindName};

type PropertyFn = Box<dyn Fn(usize, &[Value]) -> Outcome + Send>;
type MachineFn = Box<dyn Fn(&Sequence) -> Trace + Send>;

struct Fake {
    property: PropertyFn,
    machine: MachineFn,
    wide: bool,
}

impl Executor for Fake {
    fn supports_property(&self, index: usize) -> Support {
        if index == 2 && !self.wide {
            Support::No("too wide".to_string())
        } else {
            Support::Yes
        }
    }

    fn supports_machine(&self, _index: usize) -> Support {
        Support::Yes
    }

    fn call(&mut self, property: usize, cases: &[Vec<Value>]) -> Result<Vec<Outcome>> {
        Ok(cases
            .iter()
            .map(|args| (self.property)(property, args))
            .collect())
    }

    fn run(&mut self, _machine: usize, sequence: &Sequence) -> Result<Trace> {
        Ok((self.machine)(sequence))
    }
}

fn number(value: &Value) -> i128 {
    value.to_string().parse().unwrap()
}

fn returned(ty: Ty, value: i128) -> Outcome {
    Outcome::Returned(Value::from_i128(ty, value).unwrap())
}

fn location() -> Location {
    Location {
        file: "lib.fe".into(),
        line: 1,
    }
}

fn param(name: &str, ty: Ty, generator: &str) -> Param {
    Param {
        name: name.into(),
        label: Some(name.into()),
        ty,
        generator: generator.into(),
    }
}

fn method(name: &str, params: Vec<Param>) -> Method {
    Method {
        name: name.into(),
        params,
        effects: Vec::new(),
        location: location(),
    }
}

fn suite() -> Suite {
    let property = |name: &str, kind, params, result| Property {
        name: name.into(),
        kind,
        path: Vec::new(),
        params,
        result,
        cases: None,
        location: location(),
    };
    Suite {
        module: "core".into(),
        ingot: "demo".into(),
        properties: vec![
            // 0: x * 2 < 500, which fails from 250.
            property(
                "doubles_stay_small",
                PropertyKind::Holds,
                vec![param("x", Ty::Uint(64), "any")],
                Ty::Bool,
            ),
            // 1: returns x; the second target is off by one from 1000.
            property(
                "rounding_agrees",
                PropertyKind::Agrees,
                vec![param("x", Ty::Uint(64), "any")],
                Ty::Uint(64),
            ),
            // 2: always holds; the second target cannot run it.
            property(
                "wide_holds",
                PropertyKind::Holds,
                vec![
                    param("flag", Ty::Bool, "any"),
                    param("small", Ty::Uint(8), "0..=9"),
                ],
                Ty::Bool,
            ),
        ],
        machines: vec![Machine {
            name: "Counter".into(),
            path: Vec::new(),
            init: method("new", Vec::new()),
            commands: vec![
                Command {
                    method: method("add", vec![param("amount", Ty::Uint(8), "any")]),
                    weight: 1,
                },
                Command {
                    method: method("noop", Vec::new()),
                    weight: 1,
                },
            ],
            invariants: vec![method("few_big_adds", Vec::new())],
            observations: vec![Observation {
                method: method("big", Vec::new()),
                ty: Ty::Uint(64),
            }],
            steps: "1..=20".into(),
            runs: Some(200),
            clock: None,
            reuse: false,
            location: location(),
        }],
    }
}

// Interprets the counter: the invariant breaks once three steps added at
// least 10 each.
fn counter(sequence: &Sequence) -> Trace {
    let record = |big: u64| Record {
        status: StepStatus::Done,
        invariants: vec![big < 3],
        observations: vec![Value::from_u128(Ty::Uint(64), big as u128).unwrap()],
    };
    let mut big = 0;
    let mut records = vec![record(0)];
    for step in &sequence.steps {
        if step.command == 0 && number(&step.args[0]) >= 10 {
            big += 1;
        }
        records.push(record(big));
    }
    Trace {
        records,
        halt: None,
    }
}

fn fake(off_by_one: bool, wide: bool) -> Fake {
    Fake {
        property: Box::new(move |index, args| match index {
            0 => Outcome::Returned(Value::boolean(number(&args[0]) * 2 < 500)),
            1 => {
                let x = number(&args[0]);
                if off_by_one && x >= 1000 {
                    returned(Ty::Uint(64), x - 1)
                } else {
                    returned(Ty::Uint(64), x)
                }
            }
            _ => Outcome::Returned(Value::boolean(true)),
        }),
        machine: Box::new(counter),
        wide,
    }
}

fn config(seed: u64) -> Config {
    Config {
        seed,
        cases: None,
        exhaustive: false,
        shrink: true,
        max_shrinks: DEFAULT_MAX_SHRINKS,
        regressions: true,
        fe_version: "26.7.0".into(),
        optimize: vec!["1".into()],
    }
}

fn run(config: &Config, store: &mut Store) -> Vec<EntryReport> {
    let mut first = fake(false, true);
    let mut second = fake(true, false);
    let mut targets = [
        Target {
            label: "a".into(),
            executor: &mut first,
        },
        Target {
            label: "b".into(),
            executor: &mut second,
        },
    ];
    let registry = Registry::standard();
    let engine = Engine {
        config,
        registry: &registry,
        comparator: &Normalized,
    };
    engine.run_suite(&suite(), &mut targets, store, &mut |_| {})
}

fn failure(entry: &EntryReport) -> &crate::report::FailureReport {
    match &entry.status {
        Status::Failed(failure) => failure,
        other => panic!("{} did not fail: {other:?}", entry.name),
    }
}

fn args(failure: &crate::report::FailureReport) -> Vec<String> {
    match &failure.counterexample {
        Counterexample::Args { args } => args.iter().map(|arg| arg.value.clone()).collect(),
        Counterexample::Sequence { .. } => panic!("expected arguments"),
    }
}

#[test]
fn shrinks_violations_and_divergences_to_their_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path());
    let reports = run(&config(7), &mut store);

    let violated = failure(&reports[0]);
    assert_eq!(violated.kind, FailureKindName::Violated);
    assert_eq!(args(violated), ["250"]);

    let diverged = failure(&reports[1]);
    assert_eq!(diverged.kind, FailureKindName::Diverged);
    assert_eq!(args(diverged), ["1000"]);
    let outcomes: Vec<(&str, &str)> = diverged
        .outcomes
        .iter()
        .map(|o| (o.target.as_str(), o.outcome.as_str()))
        .collect();
    assert_eq!(outcomes, [("a", "returned 1000"), ("b", "returned 999")]);
    assert_eq!(
        diverged.saved_to.as_deref(),
        Some("regressions/core/rounding_agrees.json")
    );
}

#[test]
fn small_domains_run_exhaustively_and_skips_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let reports = run(&config(1), &mut Store::open(dir.path()));
    let wide = &reports[2];
    assert!(matches!(wide.status, Status::Passed));
    assert!(wide.exhaustive);
    assert_eq!(wide.cases, 20);
    assert_eq!(wide.ran_on, ["a"]);
    assert_eq!(wide.skipped[0].target, "b");
}

#[test]
fn machines_shrink_to_the_steps_that_matter() {
    let dir = tempfile::tempdir().unwrap();
    let reports = run(&config(3), &mut Store::open(dir.path()));
    let machine = failure(&reports[3]);
    assert_eq!(machine.kind, FailureKindName::Violated);
    let Counterexample::Sequence { steps, .. } = &machine.counterexample else {
        panic!("expected a sequence");
    };
    let steps: Vec<String> = steps
        .iter()
        .map(|step| {
            format!(
                "{}:{}",
                step.command,
                step.args.first().map_or("", |a| &a.value)
            )
        })
        .collect();
    assert_eq!(steps, ["add:10", "add:10", "add:10"]);
    assert_eq!(
        machine.outcomes[0].outcome,
        "invariant `few_big_adds` fails after step 3"
    );
}

#[test]
fn stored_regressions_are_replayed_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path());
    run(&config(7), &mut store);

    let mut reopened = Store::open(dir.path());
    let reports = run(&config(99), &mut reopened);
    let diverged = failure(&reports[1]);
    assert_eq!(diverged.case, None);
    assert_eq!(reports[1].regressions, 1);
    assert_eq!(reports[1].cases, 1);
    assert_eq!(args(diverged), ["1000"]);
}

#[test]
fn the_same_seed_gives_the_same_report() {
    let mut config = config(42);
    config.regressions = false;
    let dir = tempfile::tempdir().unwrap();
    let first = format!("{:?}", run(&config, &mut Store::open(dir.path())));
    let second = format!("{:?}", run(&config, &mut Store::open(dir.path())));
    assert_eq!(first, second);
    assert!(!dir.path().join("regressions").exists());
}

#[test]
fn halts_are_failures_and_describe_their_step() {
    let mut halting = Fake {
        property: Box::new(|_, _| Outcome::Returned(Value::boolean(true))),
        machine: Box::new(|sequence| Trace {
            records: vec![Record {
                status: StepStatus::Done,
                invariants: vec![true],
                observations: vec![Value::from_u128(Ty::Uint(64), 0).unwrap()],
            }],
            halt: (!sequence.steps.is_empty()).then_some(Halt {
                step: 1,
                outcome: Outcome::Panicked(PanicKind::Overflow),
            }),
        }),
        wide: true,
    };
    let mut targets = [Target {
        label: "a".into(),
        executor: &mut halting,
    }];
    let registry = Registry::standard();
    let config = config(5);
    let engine = Engine {
        config: &config,
        registry: &registry,
        comparator: &Normalized,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut suite = suite();
    suite.properties.clear();
    let reports = engine.run_suite(
        &suite,
        &mut targets,
        &mut Store::open(dir.path()),
        &mut |_| {},
    );
    let machine = failure(&reports[0]);
    assert_eq!(
        machine.outcomes[0].outcome,
        "panicked (arithmetic overflow) at step 1"
    );
}
