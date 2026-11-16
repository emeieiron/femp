//! `femp prove`: the claims `femp prop` tests, proved for all inputs.

mod aggregate;
mod plan;
mod replay;
mod schedule;

use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use femp_core::artifact::write_file;
use femp_core::environment::OptLevel;
use femp_core::project::BUILD_DIR;
use femp_frontend::LINKED_FE_VERSION;
use femp_prove::Config;
use femp_prove::report::{self, ProveReport};

use crate::claims;
use crate::cli::ProveArgs;
use crate::console;
use crate::session::Session;

const PROVE_DIR: &str = "prove";
const REPORT_FILE: &str = "report.json";
/// Seconds one replayed counterexample may run on a target.
const REPLAY_TIMEOUT: u64 = 10;

pub fn run(args: &ProveArgs) -> Result<bool> {
    let session = Session::open()?;
    let platforms = session.select(&args.target.targets)?;
    let inputs = session.prepare(&platforms, true)?;
    let project = inputs.context(OptLevel::default()).project;

    let found = claims::discover_all(project, &platforms, args.filter.as_deref())?;
    if found.is_empty() {
        console::warning(
            "no `#[property]`, `#[differential]`, `#[machine]` or `#[spec]` claims found",
        );
        return Ok(true);
    }
    if args.list {
        claims::list(&found, true);
        return Ok(true);
    }
    let solver = session.environment.host.z3.clone().ok_or_else(|| {
        anyhow!(
            "`femp prove` needs the Z3 solver: install `z3`, or point {} at it",
            femp_toolchain::solver::Z3_ENV
        )
    })?;
    let pinned = &session.project.manifest.toolchain.fe;
    if pinned != LINKED_FE_VERSION {
        console::warning(&format!(
            "the project pins Fe {pinned}, but femp proves with the Fe {LINKED_FE_VERSION} front end"
        ));
    }

    let out_dir = project.root.join(BUILD_DIR).join(PROVE_DIR);
    let options = plan::Options {
        out_dir: out_dir.clone(),
        dump_vir: args.dump_vir,
        keep_smt: args.keep_smt,
    };
    let mut plans = Vec::new();
    for (module, claims) in &found {
        let mut module_plans = plan::skeletons(module, claims);
        for platform in platforms
            .iter()
            .filter(|platform| module.is_shared() || module.platform() == Some(platform.id()))
        {
            console::step("Lowering", &format!("{} [{}]", module.name, platform.id()));
            plan::add_target(
                &mut module_plans,
                module,
                claims,
                platform.fe_target(),
                platform.id().as_str(),
                &options,
            )?;
        }
        plans.extend(module_plans);
    }

    let config = Config {
        solver,
        timeout: Duration::from_secs(args.timeout.max(1)),
    };
    let jobs = args
        .jobs
        .unwrap_or_else(|| thread::available_parallelism().map_or(1, |count| count.get()));
    schedule::run(&mut plans, &config, jobs, &|label| {
        console::step("Checked", label);
    });

    let places = aggregate::Places::new(project);
    let (mut claims, refutations): (Vec<_>, Vec<_>) = plans
        .iter()
        .map(|plan| aggregate::report(plan, &places))
        .unzip();
    if !args.no_replay {
        let replayer = replay::Replayer {
            inputs: &inputs,
            platforms: &platforms,
            timeout: Duration::from_secs(REPLAY_TIMEOUT),
            fe_version: session.environment.toolchain.version.clone(),
        };
        for (module, module_claims) in &found {
            let pending: Vec<replay::Found<'_>> = claims
                .iter_mut()
                .zip(&plans)
                .zip(&refutations)
                .filter(|((_, plan), _)| plan.module == module.name)
                .filter_map(|((report, plan), refutation)| {
                    Some(replay::Found {
                        plan,
                        report,
                        refutation: refutation.clone()?,
                    })
                })
                .collect();
            replayer.replay(module, module_claims, pending)?;
        }
    }
    let report = ProveReport {
        targets: platforms
            .iter()
            .map(|platform| platform.id().to_string())
            .collect(),
        claims,
    };
    console::line("");
    console::line(report::render(&report).trim_end());
    if args.json {
        let path = out_dir.join(REPORT_FILE);
        let json = serde_json::to_string_pretty(&report).context("cannot encode the report")?;
        write_file(&path, format!("{json}\n"))?;
        console::step("Wrote", &console::relative(&path, &project.root));
    }
    Ok(report.succeeded(args.strict))
}
