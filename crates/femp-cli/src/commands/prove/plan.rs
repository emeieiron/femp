//! Turns a module's claims into tasks, one per target. Targets whose
//! programs for a claim print the same share one task, so code without
//! `#[cfg(target)]` differences is proved once.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use femp_core::project::Module;
use femp_core::property::PropertyKind;
use femp_frontend::{Loaded, Root, Unsupported};
use femp_prove::obligation::property;
use femp_prove::passes;
use femp_prove::report::ClaimKind;
use femp_prove::{Dumps, NotSupported, Task, Verdict};
use femp_spec::Claims;

/// The declaration a claim comes from, by its index in the module's claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Property(usize),
    Machine(usize),
    Contract(usize),
}

pub struct ClaimPlan {
    pub module: String,
    pub name: String,
    pub kind: ClaimKind,
    pub location: String,
    pub entry: Entry,
    pub results: Vec<Pending>,
    /// What the claim's tasks take on trust.
    pub assumed: Vec<String>,
}

pub enum Pending {
    Job(Box<Job>),
    Done {
        targets: Vec<String>,
        verdict: Verdict,
    },
}

pub struct Job {
    pub targets: Vec<String>,
    pub task: Task,
    /// The task's program as text, which decides whether targets share it.
    pub fingerprint: String,
    pub dumps: Dumps,
}

pub struct Options {
    pub out_dir: PathBuf,
    pub dump_vir: bool,
    pub keep_smt: bool,
}

pub fn skeletons(module: &Module, claims: &Claims) -> Vec<ClaimPlan> {
    let mut plans = Vec::new();
    for (index, property) in claims.suite.properties.iter().enumerate() {
        plans.push(ClaimPlan {
            module: module.name.clone(),
            name: property.name.clone(),
            kind: match property.kind {
                PropertyKind::Holds => ClaimKind::Property,
                PropertyKind::Agrees => ClaimKind::Differential,
            },
            location: property.location.to_string(),
            entry: Entry::Property(index),
            results: Vec::new(),
            assumed: Vec::new(),
        });
    }
    for (index, machine) in claims.suite.machines.iter().enumerate() {
        plans.push(ClaimPlan {
            module: module.name.clone(),
            name: machine.name.clone(),
            kind: ClaimKind::Machine,
            location: machine.location.to_string(),
            entry: Entry::Machine(index),
            results: Vec::new(),
            assumed: Vec::new(),
        });
    }
    for (index, contract) in claims.contracts.iter().enumerate() {
        plans.push(ClaimPlan {
            module: module.name.clone(),
            name: contract.name.clone(),
            kind: ClaimKind::Spec,
            location: contract.location.to_string(),
            entry: Entry::Contract(index),
            results: Vec::new(),
            assumed: Vec::new(),
        });
    }
    plans
}

fn not_supported(error: &Unsupported) -> Verdict {
    Verdict::NotSupported(NotSupported::new(error.span.clone(), error.message.clone()))
}

/// Lowers the module for one target and adds each claim's task for it.
pub fn add_target(
    plans: &mut [ClaimPlan],
    module: &Module,
    claims: &Claims,
    fe_target: &str,
    label: &str,
    options: &Options,
) -> Result<()> {
    let loaded = Loaded::open(&module.dir, fe_target)
        .map_err(|error| anyhow!("cannot load `{}` for {label}: {error}", module.name))?;
    let roots: Vec<Root> = claims
        .suite
        .properties
        .iter()
        .map(|property| Root::function(&property.path, &property.name))
        .collect();
    let lowered = loaded.lower(&roots);
    for plan in plans.iter_mut() {
        let built = match plan.entry {
            Entry::Property(index) => {
                let declared = &claims.suite.properties[index];
                if declared.kind == PropertyKind::Agrees {
                    continue;
                }
                match &lowered.roots[index] {
                    Ok(func) => property::task(&lowered.program, *func, declared),
                    Err(error) => {
                        record(plan, label, not_supported(error));
                        continue;
                    }
                }
            }
            Entry::Machine(_) | Entry::Contract(_) => continue,
        };
        match built {
            Ok(task) => add_task(plan, task, label, options),
            Err(message) => record(
                plan,
                label,
                Verdict::NotSupported(NotSupported::new(Default::default(), message)),
            ),
        }
    }
    Ok(())
}

fn record(plan: &mut ClaimPlan, label: &str, verdict: Verdict) {
    plan.results.push(Pending::Done {
        targets: vec![label.to_string()],
        verdict,
    });
}

fn add_task(plan: &mut ClaimPlan, mut task: Task, label: &str, options: &Options) {
    for assumption in &task.assumed {
        if !plan.assumed.contains(assumption) {
            plan.assumed.push(assumption.clone());
        }
    }
    if let Err(error) = passes::scope().run(&mut task) {
        record(plan, label, Verdict::NotSupported(error));
        return;
    }
    let fingerprint = femp_vir::print::body(&task.program);
    for pending in &mut plan.results {
        if let Pending::Job(job) = pending
            && job.fingerprint == fingerprint
        {
            job.targets.push(label.to_string());
            return;
        }
    }
    let file = |dir: &str, extension: &str| {
        options
            .out_dir
            .join(dir)
            .join(label)
            .join(&plan.module)
            .join(format!("{}.{extension}", plan.name))
    };
    let dumps = Dumps {
        vir: options.dump_vir.then(|| file("vir", "vir")),
        smt: options.keep_smt.then(|| file("smt", "smt2")),
    };
    plan.results.push(Pending::Job(Box::new(Job {
        targets: vec![label.to_string()],
        task,
        fingerprint,
        dumps,
    })));
}
