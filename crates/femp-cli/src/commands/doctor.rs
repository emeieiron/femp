use std::env;

use anyhow::Result;
use femp_core::Error;
use femp_core::environment::Environment;
use femp_core::layering;
use femp_core::platform::{Capability, PlatformId, PlatformRegistry, Stability};
use femp_core::project::Project;
use femp_host::probe;

use crate::console;
use crate::generation;
use crate::registry;

pub fn run() -> Result<bool> {
    let registry = registry::platforms();
    let environment = Environment {
        toolchain: femp_compiler::toolchain(),
        host: probe::host(),
    };
    print_environment(&environment);
    let unavailable = print_platforms(&registry, &environment);
    Ok(print_project(&registry, &unavailable))
}

fn print_environment(environment: &Environment) {
    let toolchain = &environment.toolchain;
    let host = &environment.host;

    console::line("Compiler");
    console::line(format!(
        "  fe         {} ({}), linked into femp",
        toolchain.version, toolchain.commit
    ));
    console::line(format!("  backends   {}", toolchain.backends.join(", ")));
    console::line("Host");
    console::line(format!("  system     {} {}", host.arch, host.os));
    console::line(format!(
        "  node       {}",
        match (&host.node, host.node_major) {
            (Some(path), Some(major)) => format!("v{major} ({})", path.display()),
            _ => "not found".to_string(),
        }
    ));
    console::line(format!(
        "  cc         {}",
        if host.has_c_compiler {
            "found"
        } else {
            "not found"
        }
    ));
    console::line(format!(
        "  z3         {}",
        match (&host.z3, &host.z3_version) {
            (Some(path), Some(version)) => format!("{version} ({})", path.display()),
            _ => "not found (needed by `femp prove`)".to_string(),
        }
    ));
}

fn print_platforms(registry: &PlatformRegistry, environment: &Environment) -> Vec<PlatformId> {
    console::line("Platforms");
    let mut unavailable = Vec::new();
    for platform in registry.iter() {
        let stability = match platform.stability() {
            Stability::Stable => "stable",
            Stability::Experimental => "experimental",
        };
        match platform.capability(environment) {
            Capability::Available => {
                console::line(format!("  {:<10} {stability:<13} available", platform.id()));
            }
            Capability::Unavailable { reason, fix } => {
                unavailable.push(platform.id().clone());
                console::line(format!(
                    "  {:<10} {stability:<13} unavailable: {reason}",
                    platform.id()
                ));
                if let Some(fix) = fix {
                    console::line(format!("  {:<10} {:<13} fix: {fix}", "", ""));
                }
            }
        }
    }
    unavailable
}

fn print_project(registry: &PlatformRegistry, unavailable: &[PlatformId]) -> bool {
    let project = match env::current_dir() {
        Ok(dir) => match Project::discover(&dir) {
            Ok(project) => project,
            Err(Error::ProjectNotFound { .. }) => return true,
            Err(error) => {
                console::error(&error.to_string());
                return false;
            }
        },
        Err(error) => {
            console::error(&error.to_string());
            return false;
        }
    };
    console::line("Project");
    console::line(format!(
        "  {} {} ({} modules)",
        project.name(),
        project.version(),
        project.modules.len()
    ));
    if let Err(error) = layering::validate(&project, registry) {
        console::error(&error.to_string());
        return false;
    }
    console::line("  layout     ok");
    if let Err(error) = femp_compiler::check_pin(&project.manifest.toolchain.fe) {
        console::error(&error.to_string());
        return false;
    }

    let blocked: Vec<String> = project
        .platform_ids()
        .into_iter()
        .filter(|id| unavailable.contains(id))
        .map(|id| id.to_string())
        .collect();
    if !blocked.is_empty() {
        console::error(&format!(
            "the project targets platforms that are unavailable here: {}",
            blocked.join(", ")
        ));
    }
    let mut generators_available = true;
    for module in &project.modules {
        for (name, generator) in &project.manifest.modules[&module.name].generate {
            let program = generation::executable(module, generator);
            if !generation::available(&program) {
                console::error(&format!(
                    "generator {}.{name} requires {}",
                    module.name,
                    program.display()
                ));
                generators_available = false;
            }
        }
    }
    blocked.is_empty() && generators_available
}
