use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use femp_core::artifact::write_file;
use femp_core::environment::OptLevel;
use femp_core::platform::Platform;
use femp_core::project::{BUILD_DIR, Module, Project};
use femp_core::property::{Executor, HarnessRequest, Suite};
use femp_prop::compare;
use femp_prop::engine::{Config, Engine, Target, format_seed};
use femp_prop::generators::Registry;
use femp_prop::regress::Store;
use femp_prop::report::{self, RunReport};
use femp_prop::rng::derive;
use femp_spec::discover;

use crate::cli::PropArgs;
use crate::console;
use crate::session::Session;

const PROP_DIR: &str = "prop";
const REPORT_FILE: &str = "report.json";

pub fn run(args: &PropArgs) -> Result<bool> {
    let comparator = compare::by_name(&args.compare).ok_or_else(|| {
        anyhow!(
            "unknown comparison `{}`; use one of {}",
            args.compare,
            compare::NAMES.join(", ")
        )
    })?;
    let session = Session::open()?;
    let platforms = session.select(&args.target.targets)?;
    let inputs = session.prepare(&platforms, true)?;
    let levels = &args.optimize;
    let project = inputs.context(levels[0]).project;

    let suites = discover_all(project, &platforms, args.filter.as_deref())?;
    if suites.is_empty() {
        console::warning("no `#[property]`, `#[differential]` or `#[machine]` entry points found");
        return Ok(true);
    }
    if args.list {
        list(&suites);
        return Ok(true);
    }

    let seed = match &args.seed {
        Some(text) => parse_seed(text)?,
        None => fresh_seed(),
    };
    let config = Config {
        seed,
        cases: args.cases,
        exhaustive: args.exhaustive,
        shrink: !args.no_shrink,
        max_shrinks: args.max_shrinks,
        regressions: !args.no_regressions,
        fe_version: session.environment.toolchain.version.clone(),
        optimize: levels
            .iter()
            .map(|level| level.as_arg().to_string())
            .collect(),
    };
    let registry = Registry::standard();
    let engine = Engine {
        config: &config,
        registry: &registry,
        comparator: comparator.as_ref(),
    };
    let mut store = Store::open(&project.root);
    let label = |platform: &dyn Platform, level: OptLevel| {
        if levels.len() > 1 {
            format!("{}-O{}", platform.id(), level.as_arg())
        } else {
            platform.id().to_string()
        }
    };

    console::step("Seed", &format_seed(seed));
    let mut entries = Vec::new();
    let mut labels = Vec::new();
    for (module, suite) in &suites {
        let mut executors: Vec<(String, Box<dyn Executor>)> = Vec::new();
        for platform in platforms
            .iter()
            .filter(|platform| module.is_shared() || module.platform() == Some(platform.id()))
        {
            for level in levels {
                let target = label(*platform, *level);
                console::step("Building", &format!("{} harness [{target}]", module.name));
                let cx = inputs.context(*level);
                let out_dir = project
                    .root
                    .join(BUILD_DIR)
                    .join(PROP_DIR)
                    .join(&target)
                    .join(&module.name);
                let executor = platform
                    .property_harness(
                        &cx,
                        &HarnessRequest {
                            subject: module,
                            suite,
                            out_dir: &out_dir,
                            timeout: Duration::from_secs(args.timeout),
                        },
                    )
                    .map_err(|error| module.compiler_error(error))
                    .with_context(|| {
                        format!(
                            "cannot build the {target} property harness of `{}`",
                            module.name
                        )
                    })?;
                if !labels.contains(&target) {
                    labels.push(target.clone());
                }
                executors.push((target, executor));
            }
        }
        let mut targets: Vec<Target<'_>> = executors
            .iter_mut()
            .map(|(label, executor)| Target {
                label: label.clone(),
                executor: executor.as_mut(),
            })
            .collect();
        entries.extend(
            engine.run_suite(suite, &mut targets, &mut store, &mut |entry| {
                console::step("Checked", &entry.label());
            }),
        );
    }

    let report = RunReport {
        seed: format_seed(seed),
        targets: labels,
        comparison: comparator.name().to_string(),
        entries,
    };
    console::line("");
    console::line(report::render(&report).trim_end());
    if args.json {
        let path = project
            .root
            .join(BUILD_DIR)
            .join(PROP_DIR)
            .join(REPORT_FILE);
        let json = serde_json::to_string_pretty(&report).context("cannot encode the report")?;
        write_file(&path, format!("{json}\n"))?;
        console::step("Wrote", &console::relative(&path, &project.root));
    }
    Ok(report.succeeded())
}

// Shared modules run on every selected platform, platform modules on their
// own; a module without entry points needs no harness.
fn discover_all<'a>(
    project: &'a Project,
    platforms: &[&dyn Platform],
    filter: Option<&str>,
) -> Result<Vec<(&'a Module, Suite)>> {
    let mut suites = Vec::new();
    let mut problems = Vec::new();
    for module in project.modules.iter().filter(|module| {
        module.is_shared()
            || platforms
                .iter()
                .any(|platform| module.platform() == Some(platform.id()))
    }) {
        match discover(module).map_err(|errors| {
            errors
                .into_iter()
                .map(|mut error| {
                    error.location.file = relative(&error.location.file, &project.root);
                    error
                })
                .collect::<Vec<_>>()
        }) {
            Ok(claims) => {
                let mut suite = claims.suite;
                relativize(&mut suite, &project.root);
                if let Some(pattern) = filter {
                    suite
                        .properties
                        .retain(|property| property.name.contains(pattern));
                    suite
                        .machines
                        .retain(|machine| machine.name.contains(pattern));
                }
                if !suite.is_empty() {
                    suites.push((module, suite));
                }
            }
            Err(errors) => problems.extend(errors),
        }
    }
    if !problems.is_empty() {
        for problem in &problems {
            console::error(&problem.to_string());
        }
        bail!("{} invalid property declaration(s)", problems.len());
    }
    Ok(suites)
}

fn relative(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

// Messages and regression files name sources relative to the project.
fn relativize(suite: &mut Suite, root: &Path) {
    for property in &mut suite.properties {
        property.location.file = relative(&property.location.file, root);
    }
    for machine in &mut suite.machines {
        machine.location.file = relative(&machine.location.file, root);
    }
}

fn list(suites: &[(&Module, Suite)]) {
    for (module, suite) in suites {
        for property in &suite.properties {
            let kind = match property.kind {
                femp_core::property::PropertyKind::Holds => "property",
                femp_core::property::PropertyKind::Agrees => "differential",
            };
            console::line(format!(
                "{}::{}  {kind}  {}",
                module.name, property.name, property.location
            ));
        }
        for machine in &suite.machines {
            console::line(format!(
                "{}::{}  machine  {}",
                module.name, machine.name, machine.location
            ));
        }
    }
}

fn parse_seed(text: &str) -> Result<u64> {
    let text = text.trim().replace('_', "");
    let parsed = match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.map_err(|_| anyhow!("`{text}` is not a seed; use a decimal or 0x hex number"))
}

fn fresh_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);
    derive(nanos, process::id() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimal_and_hex_seeds() {
        assert_eq!(parse_seed("42").unwrap(), 42);
        assert_eq!(parse_seed("0x00ff").unwrap(), 255);
        assert_eq!(parse_seed("0x3c1f_9a2b").unwrap(), 0x3c1f_9a2b);
        assert!(parse_seed("seed").is_err());
    }
}
