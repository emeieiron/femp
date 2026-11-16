//! Runs each counterexample the prover found on the compiled targets, with
//! the harnesses `femp prop` uses, judges the outcomes as `femp prop` would,
//! and stores reproduced ones as regressions that `femp prop` keeps
//! replaying. A counterexample the targets do not reproduce means the proof
//! model and the compiled code disagree, which fails the run.

use std::time::Duration;

use anyhow::{Context as _, Result};
use femp_core::platform::Platform;
use femp_core::project::{BUILD_DIR, Module};
use femp_core::property::{
    Delivery, Executor, HarnessRequest, Machine, Method, Outcome, Sequence, Suite, Support, Trace,
    Transaction, Transactions, Value,
};
use femp_prop::compare::Normalized;
use femp_prop::describe::{describe, first_difference};
use femp_prop::regress::{Recorded, Regression, Store};
use femp_prop::report::{FailureKindName, TargetOutcome};
use femp_prop::verdict::{judge_machine, judge_property};
use femp_prove::report::{ClaimReport, ClaimStatus, Replay};
use femp_spec::Claims;

use super::aggregate::ReplayCase;
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
    pub case: ReplayCase,
    /// Whether the proof used specs in place of their targets, which leave
    /// results open that the code may never produce.
    pub relied: bool,
}

impl Replayer<'_> {
    pub fn replay(&self, module: &Module, claims: &Claims, found: Vec<Found<'_>>) -> Result<()> {
        if found.is_empty() {
            return Ok(());
        }
        let cx = self.inputs.context(Default::default());
        let suite = claims.testable();
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
                        suite: &suite,
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
            self.replay_one(claims, &suite, &mut executors, &mut store, item)?;
        }
        Ok(())
    }

    fn replay_one(
        &self,
        claims: &Claims,
        suite: &Suite,
        executors: &mut [(String, Box<dyn Executor>)],
        store: &mut Store,
        found: Found<'_>,
    ) -> Result<()> {
        // Runnable specs follow the suite's own properties.
        let property_index = match found.plan.entry {
            Entry::Property(index) => Some(index),
            Entry::Contract(index) => claims.contracts[index].runnable().map(|_| {
                claims.suite.properties.len()
                    + claims.contracts[..index]
                        .iter()
                        .filter(|contract| contract.runnable().is_some())
                        .count()
            }),
            Entry::Machine(_) | Entry::Storage(_) => None,
        };
        let (entry_name, judged, outcomes) = match (found.plan.entry, &found.case) {
            (Entry::Property(_) | Entry::Contract(_), ReplayCase::Args(values))
                if property_index.is_some() =>
            {
                let index = property_index.unwrap_or_default();
                let property = &suite.properties[index];
                let mut outcomes: Vec<(String, Outcome)> = Vec::new();
                for (label, executor) in executors.iter_mut() {
                    if let Support::No(reason) = executor.supports_property(index) {
                        found
                            .report
                            .notes
                            .push(format!("not replayed on {label}: {reason}"));
                        continue;
                    }
                    let mut results = executor
                        .call(index, std::slice::from_ref(values))
                        .with_context(|| format!("cannot replay `{}` on {label}", property.name))?;
                    outcomes.push((label.clone(), results.remove(0)));
                }
                let plain: Vec<Outcome> = outcomes
                    .iter()
                    .map(|(_, outcome)| outcome.clone())
                    .collect();
                let judged = judge_property(property.kind, &plain, &Normalized);
                let shown: Vec<(String, String)> = outcomes
                    .into_iter()
                    .map(|(target, outcome)| (target, outcome.to_string()))
                    .collect();
                (property.name.clone(), judged, shown)
            }
            (Entry::Machine(index), ReplayCase::Sequence(sequence)) => {
                let machine = &suite.machines[index];
                let mut traces: Vec<(String, Trace)> = Vec::new();
                for (label, executor) in executors.iter_mut() {
                    if let Support::No(reason) = executor.supports_machine(index) {
                        found
                            .report
                            .notes
                            .push(format!("not replayed on {label}: {reason}"));
                        continue;
                    }
                    let trace = executor
                        .run(index, sequence)
                        .with_context(|| format!("cannot replay `{}` on {label}", machine.name))?;
                    traces.push((label.clone(), trace));
                }
                let plain: Vec<Trace> = traces.iter().map(|(_, trace)| trace.clone()).collect();
                let judged = judge_machine(&plain, &Normalized);
                let focus = first_difference(&plain);
                let shown: Vec<(String, String)> = traces
                    .iter()
                    .map(|(target, trace)| (target.clone(), describe(machine, trace, focus)))
                    .collect();
                (machine.name.clone(), judged, shown)
            }
            _ => return Ok(()),
        };
        let ClaimStatus::Refuted(refuted) = &mut found.report.status else {
            return Ok(());
        };
        refuted.replay = outcomes
            .iter()
            .map(|(target, outcome)| Replay {
                target: target.clone(),
                outcome: outcome.clone(),
            })
            .collect();
        let Some(failure) = judged else {
            if found.relied {
                let shown = refuted.failure.clone();
                found.report.status = ClaimStatus::Unknown {
                    reason: format!(
                        "{shown} with a result a spec allows, but the compiled targets do not fail on this counterexample; a stronger spec may prove it"
                    ),
                };
                return Ok(());
            }
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
                    outcome: outcome.clone(),
                })
                .collect(),
            recorded: Recorded::now(&self.fe_version, vec!["1".to_string()]),
        };
        match store.save(&suite.module, &entry_name, regression) {
            Ok(path) => refuted.saved_to = Some(path.display().to_string()),
            Err(error) => found.report.notes.push(error.to_string()),
        }
        Ok(())
    }

    /// Sends each sequence that breaks a contract's invariants to the
    /// compiled contract. The proof found it among transactions that are
    /// all accepted, so a rejected one means the model and the contract
    /// disagree. The invariants read storage the EVM does not expose, so
    /// the replay confirms the transactions, not the broken invariant.
    pub fn replay_contracts(&self, module: &Module, found: Vec<Found<'_>>) -> Result<()> {
        let cx = self.inputs.context(Default::default());
        let Some(platform) = self
            .platforms
            .iter()
            .find(|platform| module.platform() == Some(platform.id()))
        else {
            return Ok(());
        };
        let label = platform.id().to_string();
        let out_dir = cx
            .project
            .root
            .join(BUILD_DIR)
            .join("prove")
            .join(REPLAY_DIR)
            .join(&label)
            .join(&module.name);
        for item in found {
            let (Some(machine), ReplayCase::Sequence(sequence)) = (&item.plan.machine, &item.case)
            else {
                continue;
            };
            let transactions = transactions(machine, &item.plan.selectors, sequence);
            console::step("Replaying", &format!("{} [{label}]", machine.name));
            let deliveries = match platform.run_contract(&cx, module, &transactions, &out_dir) {
                Ok(deliveries) => deliveries,
                Err(error) => {
                    let error = module.compiler_error(error);
                    item.report
                        .notes
                        .push(format!("not replayed on {label}: {error}"));
                    continue;
                }
            };
            let rejected =
                deliveries
                    .iter()
                    .enumerate()
                    .find_map(|(index, delivery)| match delivery {
                        Delivery::Rejected(outcome) => Some((index, outcome.clone())),
                        Delivery::Accepted => None,
                    });
            let ClaimStatus::Refuted(refuted) = &mut item.report.status else {
                continue;
            };
            let outcome = match &rejected {
                None => "accepts every transaction".to_string(),
                Some((0, outcome)) => format!("rejects the deployment: {outcome}"),
                Some((step, outcome)) => format!(
                    "rejects step {step} (`{}`): {outcome}",
                    transactions.calls[step - 1].name
                ),
            };
            refuted.replay = vec![Replay {
                target: label.clone(),
                outcome,
            }];
            if rejected.is_some() {
                item.report.notes.push(
                    "the compiled targets do not reproduce this counterexample: the proof model and the compiled code disagree"
                        .to_string(),
                );
            }
        }
        Ok(())
    }

    /// Whether the replay disagreed with the proof.
    pub fn disagrees(report: &ClaimReport) -> bool {
        report
            .notes
            .iter()
            .any(|note| note.contains("do not reproduce"))
    }
}

/// A contract's refuting sequence as transactions: each handler takes its
/// context fields first, which the transaction runs in.
fn transactions(machine: &Machine, selectors: &[Option<u32>], sequence: &Sequence) -> Transactions {
    let transaction = |method: &Method, selector: Option<u32>, values: &[Value]| {
        let mut transaction = Transaction {
            selector,
            name: method.name.clone(),
            args: Vec::new(),
            context: Vec::new(),
        };
        for (param, value) in method.params.iter().zip(values) {
            if param.name.starts_with("ctx.") {
                transaction.context.push((param.name.clone(), *value));
            } else {
                transaction.args.push(*value);
            }
        }
        transaction
    };
    Transactions {
        contract: machine.name.clone(),
        deploy: transaction(&machine.init, None, &sequence.init),
        calls: sequence
            .steps
            .iter()
            .map(|step| {
                transaction(
                    &machine.commands[step.command].method,
                    selectors.get(step.command).copied().flatten(),
                    &step.args,
                )
            })
            .collect(),
    }
}
