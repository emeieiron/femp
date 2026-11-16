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
}

pub fn list(found: &[(&Module, Claims)], specs: bool) {
    for (module, claims) in found {
        for property in &claims.suite.properties {
            let kind = match property.kind {
                PropertyKind::Holds => "property",
                PropertyKind::Agrees => "differential",
            };
            console::line(format!(
                "{}::{}  {kind}  {}",
                module.name, property.name, property.location
            ));
        }
        for machine in &claims.suite.machines {
            console::line(format!(
                "{}::{}  machine  {}",
                module.name, machine.name, machine.location
            ));
        }
        if specs {
            for contract in &claims.contracts {
                console::line(format!(
                    "{}::{}  spec  {}",
                    module.name, contract.name, contract.location
                ));
            }
        }
    }
}
