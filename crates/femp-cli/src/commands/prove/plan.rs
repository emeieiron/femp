//! Turns a module's claims into tasks, one per target. Targets whose
//! programs for a task print the same share it, so code without
//! `#[cfg(target)]` differences is proved once. Differentials compare the
//! distinct programs once every target is lowered; a machine's search runs
//! only after its induction fails.

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use femp_core::project::Module;
use femp_core::property::{Machine, PropertyKind};
use femp_frontend::{Loaded, Root, Unsupported};
use femp_prove::obligation::differential::{self, Side};
use femp_prove::obligation::machine::{self, MachineFns};
use femp_prove::obligation::property;
use femp_prove::passes;
use femp_prove::report::ClaimKind;
use femp_prove::{Dumps, NotSupported, Task, Verdict};
use femp_spec::Claims;
use femp_vir::Program;

/// The declaration a claim comes from, by its index in the module's claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Property(usize),
    Machine(usize),
    Contract(usize),
}

/// What a task establishes for its claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Main,
    Init,
    Step(usize),
    Search,
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
    /// Each distinct program of a differential, with its targets.
    sides: Vec<(Vec<String>, Side)>,
    /// Each distinct program of a machine, for the search after induction.
    machines: Vec<MachineSide>,
}

pub struct MachineSide {
    pub targets: Vec<String>,
    pub program: Program,
    pub fns: MachineFns,
}

pub enum Pending {
    Job(Box<Job>),
    Done(Done),
}

pub struct Done {
    pub role: Role,
    pub targets: Vec<String>,
    /// Which of the job's tasks decided, such as the number of steps a
    /// search took, less one.
    pub index: usize,
    pub verdict: Verdict,
}

pub struct Job {
    pub role: Role,
    pub targets: Vec<String>,
    /// Proved in order until one is refuted.
    pub tasks: Vec<Task>,
    /// The tasks' programs as text, which decides whether targets share them.
    pub fingerprint: String,
    pub dumps: Vec<Dumps>,
}

pub struct Options {
    pub out_dir: PathBuf,
    pub dump_vir: bool,
    pub keep_smt: bool,
    pub steps: usize,
}

fn skeleton(
    module: &Module,
    name: &str,
    kind: ClaimKind,
    location: String,
    entry: Entry,
) -> ClaimPlan {
    ClaimPlan {
        module: module.name.clone(),
        name: name.to_string(),
        kind,
        location,
        entry,
        results: Vec::new(),
        assumed: Vec::new(),
        sides: Vec::new(),
        machines: Vec::new(),
    }
}

pub fn skeletons(module: &Module, claims: &Claims) -> Vec<ClaimPlan> {
    let mut plans = Vec::new();
    for (index, property) in claims.suite.properties.iter().enumerate() {
        let kind = match property.kind {
            PropertyKind::Holds => ClaimKind::Property,
            PropertyKind::Agrees => ClaimKind::Differential,
        };
        plans.push(skeleton(
            module,
            &property.name,
            kind,
            property.location.to_string(),
            Entry::Property(index),
        ));
    }
    for (index, machine) in claims.suite.machines.iter().enumerate() {
        let mut plan = skeleton(
            module,
            &machine.name,
            ClaimKind::Machine,
            machine.location.to_string(),
            Entry::Machine(index),
        );
        if machine.uses_effects() {
            plan.results.push(Pending::Done(Done {
                role: Role::Main,
                targets: Vec::new(),
                index: 0,
                verdict: Verdict::NotSupported(NotSupported::new(
                    femp_vir::Span::new(
                        machine.location.file.display().to_string(),
                        machine.location.line as u32,
                    ),
                    "machines whose methods use effects are not supported yet",
                )),
            }));
        }
        plans.push(plan);
    }
    for (index, contract) in claims.contracts.iter().enumerate() {
        plans.push(skeleton(
            module,
            &contract.name,
            ClaimKind::Spec,
            contract.location.to_string(),
            Entry::Contract(index),
        ));
    }
    plans
}

fn not_supported(error: &Unsupported) -> Verdict {
    Verdict::NotSupported(NotSupported::new(error.span.clone(), error.message.clone()))
}

/// The functions each claim needs, in one list for the front end, and
/// where each claim's functions start in it.
struct Roots {
    roots: Vec<Root>,
    properties: Vec<usize>,
    machines: Vec<Option<usize>>,
}

fn roots(claims: &Claims) -> Roots {
    let mut roots = Vec::new();
    let mut properties = Vec::new();
    for property in &claims.suite.properties {
        properties.push(roots.len());
        roots.push(Root::function(&property.path, &property.name));
    }
    let mut machines = Vec::new();
    for machine in &claims.suite.machines {
        if machine.uses_effects() {
            machines.push(None);
            continue;
        }
        machines.push(Some(roots.len()));
        for method in machine.methods() {
            roots.push(Root::method(&machine.path, &machine.name, &method.name));
        }
    }
    Roots {
        roots,
        properties,
        machines,
    }
}

fn machine_fns(
    machine: &Machine,
    start: usize,
    found: &[Result<femp_vir::FuncId, Unsupported>],
) -> Result<MachineFns, Unsupported> {
    let count = machine.methods().count();
    let mut ids = found[start..start + count].iter().cloned();
    let mut take = |count: usize| -> Result<Vec<femp_vir::FuncId>, Unsupported> {
        ids.by_ref().take(count).collect()
    };
    let init = take(1)?[0];
    Ok(MachineFns {
        init,
        commands: take(machine.commands.len())?,
        invariants: take(machine.invariants.len())?,
        observations: take(machine.observations.len())?,
    })
}

/// Lowers the module for one target and adds each claim's tasks for it.
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
    let wanted = roots(claims);
    let lowered = loaded.lower(&wanted.roots);
    let program = &lowered.program;
    for plan in plans.iter_mut() {
        match plan.entry {
            Entry::Property(index) => {
                let declared = &claims.suite.properties[index];
                let func = match &lowered.roots[wanted.properties[index]] {
                    Ok(func) => *func,
                    Err(error) => {
                        record(plan, Role::Main, label, not_supported(error));
                        continue;
                    }
                };
                match declared.kind {
                    PropertyKind::Holds => {
                        let built = property::task(program, func, declared);
                        add_built(plan, Role::Main, built, label, options);
                    }
                    PropertyKind::Agrees => match differential::side(program, func) {
                        Ok(side) => add_side(plan, side, label),
                        Err(error) => record(plan, Role::Main, label, Verdict::NotSupported(error)),
                    },
                }
            }
            Entry::Machine(index) => {
                let declared = &claims.suite.machines[index];
                let Some(start) = wanted.machines[index] else {
                    continue;
                };
                let fns = match machine_fns(declared, start, &lowered.roots) {
                    Ok(fns) => fns,
                    Err(error) => {
                        record(plan, Role::Main, label, not_supported(&error));
                        continue;
                    }
                };
                let init = machine::init_task(program, &fns, declared);
                add_built(plan, Role::Init, init, label, options);
                for command in 0..declared.commands.len() {
                    let step = machine::step_task(program, &fns, declared, command);
                    add_built(plan, Role::Step(command), step, label, options);
                }
                plan.machines.push(MachineSide {
                    targets: vec![label.to_string()],
                    program: program.clone(),
                    fns,
                });
            }
            Entry::Contract(_) => {}
        }
    }
    Ok(())
}

fn record(plan: &mut ClaimPlan, role: Role, label: &str, verdict: Verdict) {
    plan.results.push(Pending::Done(Done {
        role,
        targets: vec![label.to_string()],
        index: 0,
        verdict,
    }));
}

fn add_built(
    plan: &mut ClaimPlan,
    role: Role,
    built: Result<Task, String>,
    label: &str,
    options: &Options,
) {
    match built {
        Ok(task) => add_tasks(plan, role, vec![task], label, options),
        Err(message) => record(
            plan,
            role,
            label,
            Verdict::NotSupported(NotSupported::new(Default::default(), message)),
        ),
    }
}

fn add_tasks(plan: &mut ClaimPlan, role: Role, tasks: Vec<Task>, label: &str, options: &Options) {
    let mut scoped = Vec::new();
    for mut task in tasks {
        for assumption in &task.assumed {
            if !plan.assumed.contains(assumption) {
                plan.assumed.push(assumption.clone());
            }
        }
        if let Err(error) = passes::scope().run(&mut task) {
            record(plan, role, label, Verdict::NotSupported(error));
            return;
        }
        scoped.push(task);
    }
    let fingerprint: String = scoped
        .iter()
        .map(|task| femp_vir::print::fingerprint(&task.program))
        .collect();
    for pending in &mut plan.results {
        if let Pending::Job(job) = pending
            && job.role == role
            && job.fingerprint == fingerprint
        {
            job.targets.push(label.to_string());
            return;
        }
    }
    let suffix = match role {
        Role::Main => String::new(),
        Role::Init => ".init".to_string(),
        Role::Step(command) => format!(".step{command}"),
        Role::Search => ".search".to_string(),
    };
    let count = scoped.len();
    let dumps = (0..count)
        .map(|index| {
            let number = if count > 1 {
                format!(".{}", index + 1)
            } else {
                String::new()
            };
            let file = |dir: &str, extension: &str| {
                options
                    .out_dir
                    .join(dir)
                    .join(label)
                    .join(&plan.module)
                    .join(format!("{}{suffix}{number}.{extension}", plan.name))
            };
            Dumps {
                vir: options.dump_vir.then(|| file("vir", "vir")),
                smt: options.keep_smt.then(|| file("smt", "smt2")),
            }
        })
        .collect();
    plan.results.push(Pending::Job(Box::new(Job {
        role,
        targets: vec![label.to_string()],
        tasks: scoped,
        fingerprint,
        dumps,
    })));
}

fn add_side(plan: &mut ClaimPlan, side: Side, label: &str) {
    let text = femp_vir::print::fingerprint(&side.program);
    for (targets, existing) in &mut plan.sides {
        if femp_vir::print::fingerprint(&existing.program) == text {
            targets.push(label.to_string());
            return;
        }
    }
    plan.sides.push((vec![label.to_string()], side));
}

/// Turns the distinct programs of each differential into comparisons, once
/// every target is lowered.
pub fn compare_sides(plans: &mut [ClaimPlan], claims: &Claims, options: &Options) {
    for plan in plans.iter_mut() {
        let Entry::Property(index) = plan.entry else {
            continue;
        };
        let sides = std::mem::take(&mut plan.sides);
        if sides.is_empty() {
            continue;
        }
        let declared = &claims.suite.properties[index];
        if sides.len() == 1 {
            plan.results.push(Pending::Done(Done {
                role: Role::Main,
                targets: sides[0].0.clone(),
                index: 0,
                verdict: Verdict::Proved,
            }));
            continue;
        }
        let (first_targets, first) = &sides[0];
        for (targets, other) in &sides[1..] {
            let claim = format!(
                "diverges: {} and {} compute different outcomes",
                first_targets.join("+"),
                targets.join("+")
            );
            let label = format!("{}+{}", first_targets.join("+"), targets.join("+"));
            let built = differential::task(first, other, declared, &claim);
            add_built(plan, Role::Main, built, &label, options);
            if let Some(Pending::Job(job)) = plan.results.last_mut() {
                job.targets = first_targets.iter().chain(targets).cloned().collect();
            }
        }
    }
}

/// Adds a bounded search from `#[init]` for each machine program whose
/// induction failed.
pub fn add_searches(plans: &mut [ClaimPlan], claims: &Claims, options: &Options) {
    for plan in plans.iter_mut() {
        let Entry::Machine(index) = plan.entry else {
            continue;
        };
        let declared = &claims.suite.machines[index];
        let sides = std::mem::take(&mut plan.machines);
        for side in sides {
            let failed = plan.results.iter().any(|pending| {
                matches!(pending, Pending::Done(done)
                    if matches!(done.role, Role::Step(_))
                        && done.targets.iter().any(|t| side.targets.contains(t))
                        && !done.verdict.is_proved())
            });
            if !failed {
                continue;
            }
            let tasks: Result<Vec<Task>, String> = (1..=options.steps)
                .map(|steps| machine::search_task(&side.program, &side.fns, declared, steps))
                .collect();
            for target in &side.targets {
                match &tasks {
                    Ok(tasks) => add_tasks(plan, Role::Search, tasks.clone(), target, options),
                    Err(message) => record(
                        plan,
                        Role::Search,
                        target,
                        Verdict::NotSupported(NotSupported::new(
                            Default::default(),
                            message.clone(),
                        )),
                    ),
                }
            }
        }
    }
}
