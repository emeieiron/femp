//! Proves one task: the passes, the verification conditions, then one
//! solver query per assertion in program order. The first assertion the
//! solver can break refutes the task, with minimized inputs.

use std::path::PathBuf;
use std::time::Duration;

use femp_core::property::{Ty, U256, Value};
use femp_spec::Domain;

use crate::passes::{self, Pass};
use crate::solver::minimize::{self, Input};
use crate::solver::{Answer, Solver, Z3, model};
use crate::task::Task;
use crate::vc::{self, Script};
use crate::verdict::{Refutation, Verdict};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// How far from its simplest value an input drawn from its whole type may
/// lie in the executions a search tries first, nearest first. Wide values
/// are what make the solver's nonlinear arithmetic slow, and a small
/// execution is the one minimization moves toward anyway.
const SMALL_INPUTS: [u64; 3] = [1, 0xff, 0xffff];

#[derive(Debug, Clone)]
pub struct Config {
    pub solver: PathBuf,
    /// The longest one query may take.
    pub timeout: Duration,
    /// The longest a sequence of tasks, such as a bounded search, may take
    /// before it stops starting new ones.
    pub search_budget: Duration,
    /// How many times a loop without an `invariant(..)` is unrolled.
    pub unroll: usize,
}

/// Where to write the task as the passes leave it, and its script.
#[derive(Debug, Clone, Default)]
pub struct Dumps {
    pub vir: Option<PathBuf>,
    pub smt: Option<PathBuf>,
}

/// A verdict, with what the proof relied on to reach it.
#[derive(Debug, Clone)]
pub struct Proof {
    pub verdict: Verdict,
    /// Specs whose summaries stood in for their targets.
    pub relies_on: Vec<String>,
    pub assumed: Vec<String>,
}

pub fn prove(mut task: Task, config: &Config, dumps: &Dumps) -> Proof {
    let pipeline = passes::with_unrolling(config.unroll);
    let verdict = prove_with(&mut task, &pipeline, config, dumps);
    Proof {
        verdict,
        relies_on: task.relies_on,
        assumed: task.assumed,
    }
}

/// Proves tasks in order until one is refuted, as a search over growing
/// bounds does, and says which one decided: the refuted task, the last one
/// when every task is proved, and otherwise the first one left undecided.
/// A sequence stops early once it has spent the search budget, undecided
/// at the first task it did not start unless an earlier one was.
pub fn prove_first(tasks: &[Task], config: &Config, dumps: &[Dumps]) -> (usize, Proof) {
    let started = std::time::Instant::now();
    let mut unknown: Option<(usize, String)> = None;
    let mut relies_on: Vec<String> = Vec::new();
    let mut assumed: Vec<String> = Vec::new();
    let finish = |verdict, relies_on, assumed| Proof {
        verdict,
        relies_on,
        assumed,
    };
    for (index, task) in tasks.iter().enumerate() {
        if index > 0 && started.elapsed() > config.search_budget {
            let (index, reason) = unknown
                .unwrap_or_else(|| (index, "the search spent its time budget".to_string()));
            return (index, finish(Verdict::Unknown(reason), relies_on, assumed));
        }
        let dumps = dumps.get(index).cloned().unwrap_or_default();
        let proof = prove(task.clone(), config, &dumps);
        for name in proof.relies_on {
            if !relies_on.contains(&name) {
                relies_on.push(name);
            }
        }
        for note in proof.assumed {
            if !assumed.contains(&note) {
                assumed.push(note);
            }
        }
        match proof.verdict {
            Verdict::Proved => {}
            Verdict::Unknown(reason) => {
                unknown.get_or_insert((index, reason));
            }
            decided => return (index, finish(decided, relies_on, assumed)),
        }
    }
    match unknown {
        Some((index, reason)) => (index, finish(Verdict::Unknown(reason), relies_on, assumed)),
        None => (
            tasks.len().saturating_sub(1),
            finish(Verdict::Proved, relies_on, assumed),
        ),
    }
}

pub fn prove_with(
    task: &mut Task,
    pipeline: &[Box<dyn Pass>],
    config: &Config,
    dumps: &Dumps,
) -> Verdict {
    let mut dumped = String::new();
    let result = passes::run(task, pipeline, &mut |name, task| {
        if dumps.vir.is_some() {
            dumped.push_str(&format!("// after {name}\n"));
            dumped.push_str(&femp_vir::print::function(
                &task.program.types,
                task.entry(),
            ));
            dumped.push('\n');
        }
    });
    if let Some(path) = &dumps.vir {
        let _ = femp_core::artifact::write_file(path, &dumped);
    }
    if let Err(error) = result {
        return Verdict::NotSupported(error);
    }
    let script = match vc::encode(task) {
        Ok(script) => script,
        Err(error) => return Verdict::NotSupported(error),
    };
    if let Some(path) = &dumps.smt {
        let _ = femp_core::artifact::write_file(path, script.text());
        if let Some(text) = script.approximate_text() {
            let _ = femp_core::artifact::write_file(&path.with_extension("approximate.smt2"), text);
        }
    }
    let timeout = task.timeout.unwrap_or(config.timeout);
    match Z3::start(&config.solver) {
        Ok(mut solver) => {
            solve(&mut solver, task, &script, timeout).unwrap_or_else(Verdict::Unknown)
        }
        Err(error) => Verdict::Unknown(error),
    }
}

/// Checks each assertion against a fresh copy of the script. A solver that
/// has answered other queries in the same context is noticeably weaker on
/// nonlinear arithmetic, so only minimization uses push and pop.
pub fn solve(
    solver: &mut dyn Solver,
    task: &Task,
    script: &Script,
    timeout: Duration,
) -> Result<Verdict, String> {
    let mut unknown: Option<String> = None;
    for obligation in &script.obligations {
        if let Some(approximate) = &script.approximate {
            solver.reset()?;
            solver.send(approximate)?;
            solver.assert(&obligation.violation)?;
            if solver.check(timeout / 4)? == Answer::Unsat {
                continue;
            }
        }
        if task.search
            && let Some(refutation) = small_counterexample(solver, task, script, obligation, timeout)?
        {
            return Ok(Verdict::Refuted(refutation));
        }
        solver.reset()?;
        solver.send(&script.prelude)?;
        solver.assert(&obligation.violation)?;
        match solver.check(timeout)? {
            Answer::Unsat => {}
            Answer::Unknown(reason) => {
                unknown.get_or_insert_with(|| {
                    format!(
                        "the solver could not decide `{}` at {} ({reason})",
                        obligation.check, obligation.span
                    )
                });
            }
            Answer::Sat => {
                let refutation = counterexample(solver, task, timeout)?;
                return Ok(Verdict::Refuted(Refutation {
                    check: obligation.check.clone(),
                    span: obligation.span.clone(),
                    ..refutation
                }));
            }
        }
    }
    Ok(match unknown {
        Some(reason) => Verdict::Unknown(reason),
        None => Verdict::Proved,
    })
}

/// Looks for an execution that breaks `obligation` among ever larger
/// small inputs, each within a quarter of the timeout, and minimizes it
/// within those bounds. A search's queries over wide integers can take the
/// solver far longer than over small ones, where an execution often is;
/// finding none there leaves the query to the full check.
fn small_counterexample(
    solver: &mut dyn Solver,
    task: &Task,
    script: &Script,
    obligation: &vc::Obligation,
    timeout: Duration,
) -> Result<Option<Refutation>, String> {
    let wide: Vec<Input<'_>> = primitives(task)
        .into_iter()
        .filter(|input| {
            input.ty != Ty::Bool && matches!(input.domain, Domain::Any(_) | Domain::Custom { .. })
        })
        .collect();
    for distance in SMALL_INPUTS.map(U256::from) {
        if !wide.iter().any(|input| minimize::narrows(input, distance)) {
            break;
        }
        solver.reset()?;
        solver.send(&script.prelude)?;
        solver.assert(&obligation.violation)?;
        for input in &wide {
            solver.assert(&minimize::within(input, distance))?;
        }
        if solver.check(timeout / 4)? == Answer::Sat {
            let refutation = counterexample(solver, task, timeout)?;
            return Ok(Some(Refutation {
                check: obligation.check.clone(),
                span: obligation.span.clone(),
                ..refutation
            }));
        }
    }
    Ok(None)
}

/// The task's inputs of primitive type, as minimization moves them.
fn primitives(task: &Task) -> Vec<Input<'_>> {
    task.inputs
        .iter()
        .filter_map(|input| {
            let (ty, domain) = input.domain.as_ref()?;
            Some(Input {
                term: format!("v{}", input.value.0),
                ty: *ty,
                domain,
            })
        })
        .collect()
}

fn counterexample(
    solver: &mut dyn Solver,
    task: &Task,
    timeout: Duration,
) -> Result<Refutation, String> {
    let primitives = primitives(task);
    solver.push()?;
    let values: Vec<Value> = minimize::minimize(solver, &primitives, timeout)?;
    let terms: Vec<String> = task
        .inputs
        .iter()
        .map(|input| format!("v{}", input.value.0))
        .collect();
    let model = solver.values(&terms)?;
    solver.pop()?;
    let function = task.entry();
    let shown = task
        .inputs
        .iter()
        .zip(&model)
        .map(|(input, sexp)| {
            (
                input.name.clone(),
                model::show(sexp, function.ty(input.value), &task.program.types),
            )
        })
        .collect();
    Ok(Refutation {
        check: femp_vir::Check::Unwind,
        span: femp_vir::Span::default(),
        values,
        shown,
    })
}
