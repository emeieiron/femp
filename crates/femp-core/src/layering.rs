use crate::error::{Error, Result};
use crate::platform::PlatformRegistry;
use crate::project::{Module, ModuleRole, Project};

pub fn validate(project: &Project, registry: &PlatformRegistry) -> Result<()> {
    let mut violations = Vec::new();
    for module in &project.modules {
        check_target(module, registry, &mut violations);
        check_dependencies(project, module, &mut violations);
    }
    check_platform_sections(project, registry, &mut violations);

    if violations.is_empty() {
        Ok(())
    } else {
        Err(Error::Layout(violations))
    }
}

fn check_target(module: &Module, registry: &PlatformRegistry, violations: &mut Vec<String>) {
    let declared = module.ingot.target.as_deref();
    match &module.role {
        ModuleRole::Shared => {
            if let Some(target) = declared {
                violations.push(format!(
                    "shared module `{}` sets `target = \"{target}\"` in its fe.toml; \
                     shared modules must compile for every platform",
                    module.name
                ));
            }
        }
        ModuleRole::Platform(id) => match registry.find(id.as_str()) {
            None => violations.push(format!(
                "module `{}` targets unknown platform `{id}` (known platforms: {})",
                module.name,
                registry.known_ids()
            )),
            Some(platform) if declared != Some(platform.fe_target()) => violations.push(format!(
                "module `{}` targets platform `{id}` but its fe.toml {}; expected `target = \"{}\"`",
                module.name,
                match declared {
                    Some(target) => format!("sets `target = \"{target}\"`"),
                    None => "sets no target".to_string(),
                },
                platform.fe_target()
            )),
            Some(_) => {}
        },
    }
}

fn check_dependencies(project: &Project, module: &Module, violations: &mut Vec<String>) {
    for dependency in project.dependencies_of(module) {
        let allowed = match (&module.role, &dependency.role) {
            (_, ModuleRole::Shared) => true,
            (ModuleRole::Platform(own), ModuleRole::Platform(other)) => own == other,
            (ModuleRole::Shared, ModuleRole::Platform(_)) => false,
        };
        if !allowed {
            violations.push(format!(
                "{} `{}` must not depend on {} `{}`",
                describe(module),
                module.name,
                describe(dependency),
                dependency.name
            ));
        }
    }
}

fn check_platform_sections(
    project: &Project,
    registry: &PlatformRegistry,
    violations: &mut Vec<String>,
) {
    for (name, section) in &project.manifest.platforms {
        let Some(platform) = registry.find(name) else {
            violations.push(format!(
                "`[platform.{name}]` configures an unknown platform (known platforms: {})",
                registry.known_ids()
            ));
            continue;
        };
        for kind in section.bindings.iter().flatten() {
            if !platform.binding_kinds().contains(&kind.as_str()) {
                violations.push(format!(
                    "platform `{name}` cannot generate `{kind}` bindings (available: {})",
                    platform.binding_kinds().join(", ")
                ));
            }
        }
    }
}

fn describe(module: &Module) -> String {
    match &module.role {
        ModuleRole::Shared => "shared module".to_string(),
        ModuleRole::Platform(id) => format!("{id} module"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{registry, write_project};

    const MANIFEST: &str = r#"
        [project]
        name = "demo"
        version = "0.1.0"

        [modules.core]
        path = "shared"

        [modules.contracts]
        path = "evm"
        platform = "evm"

        [modules.web]
        path = "wasm"
        platform = "wasm"
    "#;
    const SHARED: &str = "[ingot]\nname = \"core\"\n";
    const EVM: &str =
        "[ingot]\nname = \"contracts\"\ntarget = \"evm\"\n\n[dependencies]\ncore = \"../shared\"\n";
    const WASM: &str =
        "[ingot]\nname = \"web\"\ntarget = \"wasm\"\n\n[dependencies]\ncore = \"../shared\"\n";

    fn violations(manifest: &str, ingots: &[(&str, &str)]) -> Vec<String> {
        let dir = tempfile::tempdir().unwrap();
        write_project(dir.path(), manifest, ingots);
        let project = Project::load(dir.path()).unwrap();
        match validate(&project, &registry(&["evm", "wasm"])) {
            Ok(()) => Vec::new(),
            Err(Error::Layout(violations)) => violations,
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn accepts_a_well_layered_project() {
        let found = violations(
            MANIFEST,
            &[("shared", SHARED), ("evm", EVM), ("wasm", WASM)],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn shared_modules_must_not_set_a_target() {
        let shared = "[ingot]\nname = \"core\"\ntarget = \"evm\"\n";
        let found = violations(
            MANIFEST,
            &[("shared", shared), ("evm", EVM), ("wasm", WASM)],
        );
        assert_eq!(found.len(), 1);
        assert!(
            found[0].contains("shared module `core` sets `target = \"evm\"`"),
            "{found:?}"
        );
    }

    #[test]
    fn platform_modules_must_declare_their_target() {
        let evm = "[ingot]\nname = \"contracts\"\ntarget = \"wasm\"\n";
        let wasm = "[ingot]\nname = \"web\"\n";
        let found = violations(
            MANIFEST,
            &[("shared", SHARED), ("evm", evm), ("wasm", wasm)],
        );
        assert_eq!(found.len(), 2);
        assert!(
            found[0].contains("sets `target = \"wasm\"`; expected `target = \"evm\"`"),
            "{found:?}"
        );
        assert!(
            found[1].contains("sets no target; expected `target = \"wasm\"`"),
            "{found:?}"
        );
    }

    #[test]
    fn shared_modules_must_not_depend_on_platform_modules() {
        let shared = "[ingot]\nname = \"core\"\n\n[dependencies]\ncontracts = \"../evm\"\n";
        let found = violations(
            MANIFEST,
            &[("shared", shared), ("evm", EVM), ("wasm", WASM)],
        );
        assert_eq!(
            found,
            ["shared module `core` must not depend on evm module `contracts`"]
        );
    }

    #[test]
    fn platform_modules_must_not_cross_platforms() {
        let wasm = "[ingot]\nname = \"web\"\ntarget = \"wasm\"\n\n[dependencies]\ncontracts = \"../evm\"\n";
        let found = violations(
            MANIFEST,
            &[("shared", SHARED), ("evm", EVM), ("wasm", wasm)],
        );
        assert_eq!(
            found,
            ["wasm module `web` must not depend on evm module `contracts`"]
        );
    }

    #[test]
    fn unknown_platforms_and_bindings_are_reported() {
        let manifest = format!(
            "{MANIFEST}\n[modules.app]\npath = \"app\"\nplatform = \"console\"\n\n\
             [platform.evm]\nbindings = [\"python\"]\n\n[platform.desktop]\n"
        );
        let app = "[ingot]\nname = \"app\"\n";
        let found = violations(
            &manifest,
            &[
                ("shared", SHARED),
                ("evm", EVM),
                ("wasm", WASM),
                ("app", app),
            ],
        );
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(
            found
                .iter()
                .any(|v| v.contains("unknown platform `console`"))
        );
        assert!(
            found
                .iter()
                .any(|v| v.contains("cannot generate `python` bindings"))
        );
        assert!(
            found
                .iter()
                .any(|v| v.contains("`[platform.desktop]` configures an unknown platform"))
        );
    }
}
