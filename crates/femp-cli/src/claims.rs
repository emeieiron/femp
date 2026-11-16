//! Finds the claims of a project's modules for the commands that check
//! them, with locations relative to the project and the same `-f` filter
//! and listing everywhere.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use femp_core::platform::Platform;
use femp_core::project::{Module, Project};
use femp_core::property::PropertyKind;
use femp_spec::{Claims, discover};

use crate::console;

/// Shared modules count on every selected platform, platform modules on
/// their own. Modules without claims are left out.
pub fn discover_all<'a>(
    project: &'a Project,
    platforms: &[&dyn Platform],
    filter: Option<&str>,
) -> Result<Vec<(&'a Module, Claims)>> {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    for module in project.modules.iter().filter(|module| {
        module.is_shared()
            || platforms
                .iter()
                .any(|platform| module.platform() == Some(platform.id()))
    }) {
        match discover(module) {
            Ok(mut claims) => {
                relativize(&mut claims, &project.root);
                if let Some(pattern) = filter {
                    claims.retain_matching(pattern);
                }
                if !claims.is_empty() {
                    found.push((module, claims));
                }
            }
            Err(errors) => problems.extend(errors.into_iter().map(|mut error| {
                error.location.file = relative(&error.location.file, &project.root);
                error
            })),
        }
    }
    if !problems.is_empty() {
        for problem in &problems {
            console::error(&problem.to_string());
        }
        bail!("{} invalid property declaration(s)", problems.len());
    }
    Ok(found)
}

pub fn relative(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

// Messages and regression files name sources relative to the project.
fn relativize(claims: &mut Claims, root: &Path) {
    for property in &mut claims.suite.properties {
        property.location.file = relative(&property.location.file, root);
    }
    for machine in &mut claims.suite.machines {
        machine.location.file = relative(&machine.location.file, root);
    }
    for contract in &mut claims.contracts {
        contract.location.file = relative(&contract.location.file, root);
        contract.target.location.file = relative(&contract.target.location.file, root);
    }
    for claim in &mut claims.storage {
        claim.location.file = relative(&claim.location.file, root);
        for invariant in &mut claim.invariants {
            invariant.location.file = relative(&invariant.location.file, root);
        }
        for message in &mut claim.messages {
            message.location.file = relative(&message.location.file, root);
        }
    }
}

/// Lists every claim: properties and differentials, machines, specs, then
/// contracts. Property testing lists the specs it can run; proving lists
/// them all, and the contracts it proves.
pub fn list(found: &[(&Module, Claims)], all_specs: bool) {
    for (module, claims) in found {
        let line = |name: &str, kind: &str, location: &dyn std::fmt::Display| {
            console::line(format!("{}::{name}  {kind}  {location}", module.name));
        };
        for property in &claims.suite.properties {
            let kind = match property.kind {
                PropertyKind::Holds => "property",
                PropertyKind::Agrees => "differential",
                PropertyKind::Spec => continue,
            };
            line(&property.name, kind, &property.location);
        }
        for machine in &claims.suite.machines {
            line(&machine.name, "machine", &machine.location);
        }
        for contract in &claims.contracts {
            if all_specs || contract.runnable().is_some() {
                line(&contract.name, "spec", &contract.location);
            }
        }
        if all_specs {
            for claim in &claims.storage {
                if claim.claims_invariants && !claim.invariants.is_empty() {
                    line(&claim.name, "contract", &claim.location);
                }
                for message in &claim.messages {
                    line(&message.name, "message", &message.location);
                }
            }
        }
    }
}
