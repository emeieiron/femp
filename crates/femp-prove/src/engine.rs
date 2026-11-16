//! Proves one task: the passes, the verification conditions, then one
//! solver query per assertion in program order. The first assertion the
//! solver can break refutes the task, with minimized inputs.

use std::path::PathBuf;
use std::time::Duration;

use femp_core::property::Value;

use crate::passes::{self, Pass};
use crate::solver::minimize::{self, Input};
use crate::solver::{Answer, Solver, Z3, model};
use crate::task::Task;
use crate::vc::{self, Script};
use crate::verdict::{Refutation, Verdict};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub struct Config {
    pub solver: PathBuf,
    /// The longest one query may take.
    pub timeout: Duration,
    /// The longest a sequence of tasks, such as a bounded search, may take
    /// before it stops starting new ones.
    pub search_budget: Duration,
}

/// Where to write the task as the passes leave it, and its script.
#[derive(Debug, Clone, Default)]
pub struct Dumps {
    pub vir: Option<PathBuf>,
    pub smt: Option<PathBuf>,
}

pub fn prove(mut task: Task, config: &Config, dumps: &Dumps) -> Verdict {
    let pipeline = passes::standard();
    prove_with(&mut task, &pipeline, config, dumps)
}

/// Proves tasks in order until one is refuted, as a search over growing
/// bounds does, and says which one decided. A sequence stops early once it
/// has spent the search budget, as undecided at the last task it finished.
pub fn prove_first(tasks: &[Task], config: &Config, dumps: &[Dumps]) -> (usize, Verdict) {
    let started = std::time::Instant::now();
    let mut unknown = None;
    for (index, task) in tasks.iter().enumerate() {
        if index > 0 && started.elapsed() > config.search_budget {
            let reason = format!("the search stopped after {index} steps, its time budget spent");
            return (index - 1, Verdict::Unknown(reason));
        }
        let dumps = dumps.get(index).cloned().unwrap_or_default();
        match prove(task.clone(), config, &dumps) {
            Verdict::Proved => {}
            Verdict::Unknown(reason) => {
                unknown.get_or_insert((index, reason));
            }
            decided => return (index, decided),
        }
    }
    match unknown {
        Some((index, reason)) => (index, Verdict::Unknown(reason)),
        None => (tasks.len().saturating_sub(1), Verdict::Proved),
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

fn counterexample(
    solver: &mut dyn Solver,
    task: &Task,
    timeout: Duration,
) -> Result<Refutation, String> {
    let primitives: Vec<Input<'_>> = task
        .inputs
        .iter()
        .filter_map(|input| {
            let (ty, domain) = input.domain.as_ref()?;
            Some(Input {
                term: format!("v{}", input.value.0),
                ty: *ty,
                domain,
            })
        })
        .collect();
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
