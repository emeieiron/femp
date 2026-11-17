//! Runs each counterexample the prover found on the compiled targets, with
//! the harnesses `femp prop` uses, judges the outcomes as `femp prop` would,
//! and stores reproduced ones as regressions that `femp prop` keeps
//! replaying. A counterexample the targets do not reproduce means the proof
//! model and the compiled code disagree, which fails the run.

use std::time::Duration;

use anyhow::{Context as _, Result};
use femp_compiler::Word;
use femp_core::platform::Platform;
use femp_core::project::{BUILD_DIR, Module};
use femp_core::property::{
    Delivery, Executor, HarnessRequest, Outcome, Sequence, Suite, Support, Trace, Value,
};
use femp_prop::compare::Normalized;
use femp_prop::describe::{describe, first_difference};
use femp_prop::regress::{Recorded, Regression, Store};
use femp_prop::report::{FailureKindName, TargetOutcome};
use femp_prop::verdict::{judge_machine, judge_property};
use femp_prove::report::{ClaimReport, ClaimStatus, Replay};
use femp_spec::Claims;
use femp_vir::eval::Val;

use super::aggregate::ReplayCase;
use super::plan::{ClaimPlan, Entry, MessageSide};
use crate::console;
use crate::contracts::{ContractSide, Reads, transactions};
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
            Entry::Machine(_) | Entry::Storage(_) | Entry::Message(..) => None,
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

    /// Sends each sequence that breaks a contract's claim to the compiled
    /// contract, then evaluates the claim on the storage the contract
    /// holds. The proof found the sequence among transactions that go
    /// through but, for a message that must revert, the last; a contract
    /// that treats them otherwise, or whose storage satisfies the claim,
    /// disagrees with the model.
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
            let (
                Some(machine),
                ReplayCase::Transactions {
                    sequence,
                    last_goes_through,
                    extra,
                    holds,
                },
            ) = (&item.plan.machine, &item.case)
            else {
                continue;
            };
            let last_goes_through = *last_goes_through;
            // What a callee answers is the search's choice, which a replay
            // against accounts without code cannot give.
            let calling = item.plan.contract.as_ref().and_then(|side| {
                std::iter::once(&machine.init.name)
                    .chain(
                        sequence
                            .steps
                            .iter()
                            .map(|step| &machine.commands[step.command].method.name),
                    )
                    .find(|name| side.calls_out.contains(name))
            });
            if let Some(name) = calling {
                item.report.notes.push(format!(
                    "not replayed on {label}: `{name}` calls another contract, whose answers the search chose"
                ));
                continue;
            }
            let storage = item.plan.contract.as_ref().map(|side| {
                let named = sequence
                    .init
                    .iter()
                    .chain(sequence.steps.iter().flat_map(|step| &step.args))
                    .chain(extra);
                (side, side.reads(named))
            });
            let mut transactions = transactions(machine, &item.plan.selectors, sequence);
            if let Some((_, reads)) = &storage {
                transactions.reads = reads.slots.clone();
            }
            console::step("Replaying", &format!("{} [{label}]", machine.name));
            let run = match platform.run_contract(&cx, module, &transactions, &out_dir) {
                Ok(run) => run,
                Err(error) => {
                    let error = module.compiler_error(error);
                    item.report
                        .notes
                        .push(format!("not replayed on {label}: {error}"));
                    continue;
                }
            };
            let deliveries = &run.deliveries;
            // Every transaction goes through but, when the model says it
            // reverts, the last.
            let last = deliveries.len().saturating_sub(1);
            let unexpected = deliveries.iter().enumerate().find_map(|(index, delivery)| {
                let goes_through = index < last || last_goes_through;
                match (delivery, goes_through) {
                    (Delivery::Rejected(outcome), true) => Some((index, Some(outcome.clone()))),
                    (Delivery::Accepted, false) => Some((index, None)),
                    _ => None,
                }
            });
            let ClaimStatus::Refuted(refuted) = &mut item.report.status else {
                continue;
            };
            let name = |step: usize| match step {
                0 => "the deployment".to_string(),
                step => format!("step {step} (`{}`)", transactions.calls[step - 1].name),
            };
            let mut outcome = match &unexpected {
                None if last_goes_through => "accepts every transaction".to_string(),
                None => match deliveries.last() {
                    Some(Delivery::Rejected(outcome)) => {
                        format!("rejects {} as the model says: {outcome}", name(last))
                    }
                    _ => format!("rejects {} as the model says", name(last)),
                },
                Some((step, Some(outcome))) => format!("rejects {}: {outcome}", name(*step)),
                Some((step, None)) => {
                    format!("accepts {}, which the model says reverts", name(*step))
                }
            };
            let mut agrees = unexpected.is_none();
            if agrees && let Some((side, reads)) = &storage {
                let judged = judge(
                    side,
                    item.plan.message.as_ref(),
                    sequence,
                    extra,
                    reads,
                    &run.storage,
                );
                match judged {
                    Ok(broken) if broken.is_none() == *holds => {
                        outcome.push_str(&storage_agrees(broken, last));
                    }
                    Ok(_) => {
                        outcome.push_str(", but its storage satisfies the claim");
                        agrees = false;
                    }
                    Err(reason) => item
                        .report
                        .notes
                        .push(format!("its storage on {label} is not checked: {reason}")),
                }
            }
            refuted.replay = vec![Replay {
                target: label.clone(),
                outcome,
            }];
            if !agrees {
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

/// What the replay adds once the contract's storage gives the claim what
/// the model says: `broken` names the invariant the storage breaks.
fn storage_agrees(broken: Option<Broken>, last: usize) -> String {
    match broken {
        Some(Broken::Invariant(name)) => format!(", and its storage then breaks `{name}`"),
        Some(Broken::Message) => format!(", and its storage around step {last} breaks the claim"),
        None => format!(", and the claim holds on its storage before step {last}"),
    }
}

/// What part of a claim the contract's storage breaks.
enum Broken {
    Invariant(String),
    Message,
}

/// Evaluates the broken claim on the storage the contract holds after
/// the sequence, or before and after its last message: each invariant, or
/// the message claim. Gives what the storage breaks, if anything.
fn judge(
    side: &ContractSide,
    message: Option<&MessageSide>,
    sequence: &Sequence,
    extra: &[Value],
    reads: &Reads,
    snapshots: &[Vec<Word>],
) -> Result<Option<Broken>, String> {
    let state = |index: usize| -> Result<Val, String> {
        let words = snapshots
            .get(index)
            .ok_or_else(|| "the contract's storage was not read".to_string())?;
        side.decode(reads, words)
    };
    let extra: Vec<Val> = extra.iter().map(Val::of).collect();
    let last = sequence.steps.len();
    let Some(message) = message else {
        let broken = side
            .broken(&state(last)?, &extra)
            .map_err(|stop| stop.to_string())?;
        return Ok(broken.map(|name| Broken::Invariant(name.to_string())));
    };
    let step = sequence
        .steps
        .last()
        .ok_or_else(|| "the sequence sends no message".to_string())?;
    let mut args = vec![state(last - 1)?];
    if message.after {
        args.push(state(last)?);
    }
    args.extend(step.args.iter().map(Val::of));
    args.extend(extra);
    let holds = side
        .holds(message.func, args)
        .map_err(|stop| stop.to_string())?;
    Ok((!holds).then_some(Broken::Message))
}
