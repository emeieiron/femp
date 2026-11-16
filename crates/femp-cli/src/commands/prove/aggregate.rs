//! Combines the verdicts of a claim's tasks into its report. A property is
//! refuted if any target's program is, unknown if any is undecided, not
//! supported if any cannot be modeled, and proved only when every one is.
//! A machine is refuted by its initializer or by a search that found a
//! sequence; an induction step that fails without such a sequence leaves
//! it unknown, with the state that breaks the step.

use std::path::{Path, PathBuf};

use femp_core::project::{Module, Project};
use femp_core::property::{Machine, Param, Sequence, Value};
use femp_prove::obligation::machine;
use femp_prove::report::{ClaimReport, ClaimStatus, Refuted};
use femp_prove::{Refutation, Verdict};
use femp_spec::{Arg, Claims, Counterexample, StepReport};
use femp_vir::{Check, Span};

use super::plan::{ClaimPlan, Done, Entry, Pending, Role};

/// Shows source positions relative to the project, as the user wrote them.
pub struct Places {
    root: PathBuf,
    modules: Vec<(PathBuf, PathBuf)>,
}

impl Places {
    pub fn new(project: &Project) -> Self {
        let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        Self {
            root: canonical(&project.root),
            modules: project
                .modules
                .iter()
                .map(|module: &Module| (canonical(&module.dir), module.source_dir.clone()))
                .collect(),
        }
    }

    /// `text` with the absolute source paths in it shown as `show` would.
    pub fn relativize(&self, text: &str) -> String {
        let mut text = text.to_string();
        for (dir, source) in &self.modules {
            let source = source.canonicalize().unwrap_or_else(|_| source.clone());
            let shown = source.strip_prefix(&self.root).unwrap_or(&source);
            text = text.replace(
                &format!("{}/", dir.display()),
                &format!("{}/", shown.display()),
            );
        }
        text.replace(&format!("{}/", self.root.display()), "")
    }

    pub fn show(&self, span: &Span) -> String {
        if !span.is_known() {
            return String::new();
        }
        let path = PathBuf::from(&span.file);
        let authored = self
            .modules
            .iter()
            .find_map(|(dir, source)| {
                let relative = path.strip_prefix(dir).ok()?;
                let source = source.canonicalize().unwrap_or_else(|_| source.clone());
                Some(source.join(relative))
            })
            .unwrap_or(path);
        let shown = authored
            .strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .unwrap_or(authored);
        format!("{}:{}", shown.display(), span.line)
    }
}

/// What to run on the compiled targets to confirm a refutation.
#[derive(Debug, Clone)]
pub enum ReplayCase {
    Args(Vec<Value>),
    Sequence(Sequence),
}

fn done(plan: &ClaimPlan) -> impl Iterator<Item = &Done> {
    plan.results.iter().filter_map(|pending| match pending {
        Pending::Done(done) => Some(done),
        Pending::Job(_) => None,
    })
}

fn refuted(plan: &ClaimPlan, wanted: impl Fn(Role) -> bool) -> Option<(&Done, &Refutation)> {
    done(plan)
        .filter(|done| wanted(done.role))
        .find_map(|done| match &done.verdict {
            Verdict::Refuted(refutation) => Some((done, refutation)),
            _ => None,
        })
}

pub fn report(
    plan: &ClaimPlan,
    claims: &Claims,
    places: &Places,
) -> (ClaimReport, Option<ReplayCase>) {
    let mut targets: Vec<String> = Vec::new();
    for done in done(plan) {
        for target in &done.targets {
            if !targets.contains(target) {
                targets.push(target.clone());
            }
        }
    }
    let unknown = done(plan).find_map(|done| match &done.verdict {
        Verdict::Unknown(reason) => Some(places.relativize(reason)),
        _ => None,
    });
    let unsupported = done(plan).find_map(|done| match &done.verdict {
        Verdict::NotSupported(error) => Some((places.show(&error.span), error.message.clone())),
        _ => None,
    });
    let (status, replay) = match plan.entry {
        Entry::Machine(index) => machine_status(plan, &claims.suite.machines[index], places),
        Entry::Property(index) => {
            let params = &claims.suite.properties[index].params;
            match refuted(plan, |_| true) {
                Some((done, refutation)) => (
                    Some(refuted_status(
                        failure(&refutation.check, &refutation.span, places),
                        args_counterexample(refutation),
                        done.targets.clone(),
                    )),
                    Some(ReplayCase::Args(primitive_args(params, refutation))),
                ),
                None => (None, None),
            }
        }
        Entry::Contract(_) => (None, None),
    };
    let status = status.unwrap_or_else(|| {
        if let Some(reason) = unknown {
            ClaimStatus::Unknown { reason }
        } else if let Some((location, reason)) = unsupported {
            ClaimStatus::NotSupported { location, reason }
        } else if plan.results.is_empty() {
            ClaimStatus::Skipped {
                reason: "the prover does not handle this kind of claim yet".to_string(),
            }
        } else {
            ClaimStatus::Proved
        }
    });
    let report = ClaimReport {
        module: plan.module.clone(),
        name: plan.name.clone(),
        kind: plan.kind,
        location: plan.location.clone(),
        targets,
        assumed: plan.assumed.clone(),
        notes: Vec::new(),
        status,
    };
    (report, replay)
}

fn refuted_status(failure: String, counterexample: Counterexample, on: Vec<String>) -> ClaimStatus {
    ClaimStatus::Refuted(Box::new(Refuted {
        failure,
        counterexample,
        refuted_on: on,
        replay: Vec::new(),
        saved_to: None,
    }))
}

fn args_counterexample(refutation: &Refutation) -> Counterexample {
    Counterexample::Args {
        args: refutation
            .shown
            .iter()
            .map(|(name, value)| Arg {
                name: name.clone(),
                value: value.clone(),
            })
            .collect(),
    }
}

fn primitive_args(params: &[Param], refutation: &Refutation) -> Vec<Value> {
    refutation
        .values
        .iter()
        .take(params.len())
        .copied()
        .collect()
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

fn sequence_counterexample(machine: &Machine, sequence: &Sequence) -> Counterexample {
    Counterexample::Sequence {
        init: machine.init.name.clone(),
        init_args: args(&machine.init.params, &sequence.init),
        steps: sequence
            .steps
            .iter()
            .map(|step| {
                let method = &machine.commands[step.command].method;
                StepReport {
                    command: method.name.clone(),
                    args: args(&method.params, &step.args),
                    advance: step.advance,
                }
            })
            .collect(),
    }
}

fn machine_status(
    plan: &ClaimPlan,
    machine: &Machine,
    places: &Places,
) -> (Option<ClaimStatus>, Option<ReplayCase>) {
    let found = refuted(plan, |role| role == Role::Init)
        .map(|(done, refutation)| (done, refutation, 0))
        .or_else(|| {
            refuted(plan, |role| role == Role::Search)
                .map(|(done, refutation)| (done, refutation, done.index + 1))
        });
    if let Some((done, refutation, steps)) = found
        && let Some(sequence) = machine::decode(machine, steps, &refutation.values)
    {
        let status = refuted_status(
            failure(&refutation.check, &refutation.span, places),
            sequence_counterexample(machine, &sequence),
            done.targets.clone(),
        );
        return (Some(status), Some(ReplayCase::Sequence(sequence)));
    }
    let step = refuted(plan, |role| matches!(role, Role::Step(_)));
    let Some((done, refutation)) = step else {
        return (None, None);
    };
    let Role::Step(command) = done.role else {
        return (None, None);
    };
    let name = &machine.commands[command].method.name;
    let what = match &refutation.check {
        Check::Panic(kind) => {
            let at = places.show(&refutation.span);
            format!("`{name}` panics ({kind}) at {at}")
        }
        other => other.to_string(),
    };
    let mut shown = refutation.shown.iter();
    let state = shown
        .next()
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let arguments: Vec<String> = shown
        .map(|(param, value)| format!("{param} = {value}"))
        .collect();
    let mut from = format!("from {state}");
    if !arguments.is_empty() {
        from.push_str(&format!(", {}", arguments.join(", ")));
    }
    let mut reason = format!("{what} {from}");
    if let Some(steps) = searched(plan) {
        reason.push_str(&format!(
            "\nno sequence of up to {steps} commands from `{}` breaks it, so the invariants are not inductive",
            machine.init.name
        ));
    }
    (Some(ClaimStatus::Unknown { reason }), None)
}

/// The number of steps the machine's searches covered, if they ran.
fn searched(plan: &ClaimPlan) -> Option<usize> {
    done(plan)
        .filter(|done| done.role == Role::Search)
        .map(|done| done.index + 1)
        .max()
}

pub fn failure(check: &Check, span: &Span, places: &Places) -> String {
    let at = places.show(span);
    let at = if at.is_empty() {
        String::new()
    } else {
        format!(" at {at}")
    };
    match check {
        Check::Claim(text) => text.clone(),
        Check::Panic(kind) => format!("panics ({kind}){at}"),
        Check::Unwind => format!("a loop runs longer than it was unrolled{at}"),
    }
}
