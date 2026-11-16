//! Runs each counterexample the prover found on the compiled targets, with
//! the harnesses `femp prop` uses, judges the outcomes as `femp prop` would,
//! and stores reproduced ones as regressions that `femp prop` keeps
//! replaying. A counterexample the targets do not reproduce means the proof
//! model and the compiled code disagree, which fails the run.

use std::time::Duration;

use anyhow::{Context as _, Result};
use femp_core::platform::Platform;
use femp_core::project::{BUILD_DIR, Module};
use femp_core::property::{Executor, HarnessRequest, Outcome, Suite};
use femp_prop::compare::Normalized;
use femp_prop::regress::{Recorded, Regression, Store};
use femp_prop::report::{FailureKindName, TargetOutcome};
use femp_prop::verdict::judge_property;
use femp_prove::Refutation;
use femp_prove::report::{ClaimReport, ClaimStatus, Replay};
use femp_spec::Claims;

use super::plan::{ClaimPlan, Entry};
use crate::console;
use crate::session::BuildInputs;

const REPLAY_DIR: &str = "replay";
/// How a regression found by proof names its origin where `femp prop`
/// writes a seed.
const PROOF_SEED: &str = "proof";

pub struct Replayer<'a> {
    pub inputs: &'a BuildInputs<'a>,
    pub platforms: &'a [&'a dyn Platform],
    pub timeout: Duration,
    pub fe_version: String,
}

/// One refuted claim waiting for replay.
pub struct Found<'a> {
    pub plan: &'a ClaimPlan,
    pub report: &'a mut ClaimReport,
    pub refutation: Refutation,
}

impl Replayer<'_> {
    pub fn replay(&self, module: &Module, claims: &Claims, found: Vec<Found<'_>>) -> Result<()> {
        if found.is_empty() {
            return Ok(());
        }
        let cx = self.inputs.context(Default::default());
        let project = cx.project;
        let mut store = Store::open(&project.root);
        let mut executors: Vec<(String, Box<dyn Executor>)> = Vec::new();
        for platform in self
            .platforms
            .iter()
            .filter(|platform| module.is_shared() || module.platform() == Some(platform.id()))
        {
            let label = platform.id().to_string();
            console::step("Building", &format!("{} harness [{label}]", module.name));
            let out_dir = project
                .root
                .join(BUILD_DIR)
                .join("prove")
                .join(REPLAY_DIR)
                .join(&label)
                .join(&module.name);
            let executor = platform
                .property_harness(
                    &cx,
                    &HarnessRequest {
                        subject: module,
                        suite: &claims.suite,
                        out_dir: &out_dir,
                        timeout: self.timeout,
                    },
                )
                .map_err(|error| module.compiler_error(error))
                .with_context(|| {
                    format!(
                        "cannot build the {label} harness to replay counterexamples of `{}`",
                        module.name
                    )
                })?;
            executors.push((label, executor));
        }
        for item in found {
            self.replay_one(&claims.suite, &mut executors, &mut store, item)?;
        }
        Ok(())
    }

    fn replay_one(
        &self,
        suite: &Suite,
        executors: &mut [(String, Box<dyn Executor>)],
        store: &mut Store,
        found: Found<'_>,
    ) -> Result<()> {
        let Entry::Property(index) = found.plan.entry else {
            return Ok(());
        };
        let property = &suite.properties[index];
        let mut outcomes: Vec<(String, Outcome)> = Vec::new();
        for (label, executor) in executors.iter_mut() {
            let support = executor.supports_property(index);
            if let femp_core::property::Support::No(reason) = support {
                found
                    .report
                    .notes
                    .push(format!("not replayed on {label}: {reason}"));
                continue;
            }
            let mut results = executor
                .call(index, std::slice::from_ref(&found.refutation.values))
                .with_context(|| format!("cannot replay `{}` on {label}", property.name))?;
            outcomes.push((label.clone(), results.remove(0)));
        }
        let plain: Vec<Outcome> = outcomes
            .iter()
            .map(|(_, outcome)| outcome.clone())
            .collect();
        let judged = judge_property(property.kind, &plain, &Normalized);
        let ClaimStatus::Refuted(refuted) = &mut found.report.status else {
            return Ok(());
        };
        refuted.replay = outcomes
            .iter()
            .map(|(target, outcome)| Replay {
                target: target.clone(),
                outcome: outcome.to_string(),
            })
            .collect();
        let Some(failure) = judged else {
            found.report.notes.push(
                "the compiled targets do not reproduce this counterexample: the proof model and the compiled code disagree"
                    .to_string(),
            );
            return Ok(());
        };
        let regression = Regression {
            failure: FailureKindName::from(failure.kind()),
            seed: PROOF_SEED.to_string(),
            case: None,
            choices: Vec::new(),
            counterexample: refuted.counterexample.clone(),
            outcomes: outcomes
                .iter()
                .map(|(target, outcome)| TargetOutcome {
                    target: target.clone(),
                    outcome: outcome.to_string(),
                })
                .collect(),
            recorded: Recorded::now(&self.fe_version, vec!["1".to_string()]),
        };
        match store.save(&suite.module, &property.name, regression) {
            Ok(path) => refuted.saved_to = Some(path.display().to_string()),
            Err(error) => found.report.notes.push(error.to_string()),
        }
        Ok(())
    }
}
