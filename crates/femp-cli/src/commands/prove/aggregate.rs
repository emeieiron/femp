//! Combines the verdicts of a claim's tasks into its report: refuted if any
//! target's program is refuted, unknown if any is undecided, not supported
//! if any cannot be modeled, and proved only when every one is.

use std::path::{Path, PathBuf};

use femp_core::project::{Module, Project};
use femp_prove::report::{ClaimReport, ClaimStatus, Refuted};
use femp_prove::{Refutation, Verdict};
use femp_spec::{Arg, Counterexample};
use femp_vir::{Check, Span};

use super::plan::{ClaimPlan, Pending};

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

pub fn report(plan: &ClaimPlan, places: &Places) -> ClaimReport {
    let mut targets = Vec::new();
    let mut refuted: Option<(&Refutation, Vec<String>)> = None;
    let mut unknown: Option<String> = None;
    let mut unsupported: Option<(String, String)> = None;
    for pending in &plan.results {
        let Pending::Done {
            targets: on,
            verdict,
        } = pending
        else {
            continue;
        };
        for target in on {
            if !targets.contains(target) {
                targets.push(target.clone());
            }
        }
        match verdict {
            Verdict::Proved => {}
            Verdict::Refuted(refutation) => {
                if refuted.is_none() {
                    refuted = Some((refutation, on.clone()));
                }
            }
            Verdict::Unknown(reason) => {
                unknown.get_or_insert_with(|| reason.clone());
            }
            Verdict::NotSupported(error) => {
                unsupported
                    .get_or_insert_with(|| (places.show(&error.span), error.message.clone()));
            }
        }
    }
    let status = if let Some((refutation, on)) = refuted {
        ClaimStatus::Refuted(Box::new(Refuted {
            failure: failure(&refutation.check, &refutation.span, places),
            counterexample: Counterexample::Args {
                args: refutation
                    .shown
                    .iter()
                    .map(|(name, value)| Arg {
                        name: name.clone(),
                        value: value.clone(),
                    })
                    .collect(),
            },
            refuted_on: on,
            replay: Vec::new(),
            saved_to: None,
        }))
    } else if let Some(reason) = unknown {
        ClaimStatus::Unknown { reason }
    } else if let Some((location, reason)) = unsupported {
        ClaimStatus::NotSupported { location, reason }
    } else if plan.results.is_empty() {
        ClaimStatus::Skipped {
            reason: "the prover does not handle this kind of claim yet".to_string(),
        }
    } else {
        ClaimStatus::Proved
    };
    ClaimReport {
        module: plan.module.clone(),
        name: plan.name.clone(),
        kind: plan.kind,
        location: plan.location.clone(),
        targets,
        assumed: plan.assumed.clone(),
        notes: Vec::new(),
        status,
    }
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
