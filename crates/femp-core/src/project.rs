use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::ingot::{DependencySource, INGOT_FILE, IngotDependency, IngotManifest, ROOT_SOURCE};
use crate::manifest::{MANIFEST_FILE, Manifest};
use crate::platform::PlatformId;
use crate::workspace::Workspace;

pub const BUILD_DIR: &str = "build";
pub const DIST_DIR: &str = "dist";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleRole {
    Shared,
    Platform(PlatformId),
}

#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub dir: PathBuf,
    pub source_dir: PathBuf,
    pub ingot: IngotManifest,
    pub role: ModuleRole,
}

impl Module {
    pub fn is_shared(&self) -> bool {
        self.role == ModuleRole::Shared
    }

    pub fn platform(&self) -> Option<&PlatformId> {
        match &self.role {
            ModuleRole::Shared => None,
            ModuleRole::Platform(id) => Some(id),
        }
    }

    pub fn root_source(&self) -> PathBuf {
        self.dir.join(ROOT_SOURCE)
    }
    /// Point compiler locations at authored sources when a staged copy was compiled.
    pub fn diagnostics(&self, text: &str) -> String {
        if self.source_dir == self.dir {
            return text.to_string();
        }
        text.split_inclusive('\n')
            .map(|line| {
                let Some((prefix, location)) = line.split_once("─ ") else {
                    return line.to_string();
                };
                let ending = if line.ends_with('\n') { "\n" } else { "" };
                let mut parts = location.trim_end().rsplitn(3, ':');
                let (Some(column), Some(row), Some(path)) =
                    (parts.next(), parts.next(), parts.next())
                else {
                    return line.to_string();
                };
                let path = Path::new(path);
                let relative = path.strip_prefix(&self.dir).unwrap_or(path);
                if !relative.is_relative() || !relative.starts_with("src") {
                    return line.to_string();
                }
                let authored = self.source_dir.join(relative);
                if !authored.is_file() {
                    return line.to_string();
                }
                format!("{prefix}─ {}:{row}:{column}{ending}", authored.display())
            })
            .collect()
    }

    pub fn compiler_error(&self, error: Error) -> Error {
        match error {
            Error::Command { command, output } => Error::Command {
                command,
                output: self.diagnostics(&output),
            },
            Error::Rejected { output } => Error::Rejected {
                output: self.diagnostics(&output),
            },
            other => other,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub modules: Vec<Module>,
}

impl Project {
    pub fn discover(start: &Path) -> Result<Self> {
        let root = start
            .ancestors()
            .find(|dir| dir.join(MANIFEST_FILE).is_file())
            .ok_or_else(|| Error::ProjectNotFound {
                file: MANIFEST_FILE,
                start: start.to_path_buf(),
            })?;
        Self::load(root)
    }

    pub fn load(root: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .map_err(|source| Error::io("resolve", root, source))?;
        let manifest_path = root.join(MANIFEST_FILE);
        let manifest = Manifest::read(&manifest_path)?;

        let modules = manifest
            .modules
            .iter()
            .map(|(name, section)| {
                let dir = root.join(&section.path);
                if !dir.join(INGOT_FILE).is_file() {
                    return Err(Error::invalid_manifest(
                        &manifest_path,
                        format!(
                            "module `{name}` points at `{}`, which has no `{INGOT_FILE}`",
                            section.path.display()
                        ),
                    ));
                }
                let dir = dir
                    .canonicalize()
                    .map_err(|source| Error::io("resolve", &dir, source))?;
                Ok(Module {
                    name: name.clone(),
                    ingot: IngotManifest::read(&dir)?,
                    role: match &section.platform {
                        Some(id) => ModuleRole::Platform(PlatformId::new(id)),
                        None => ModuleRole::Shared,
                    },
                    source_dir: dir.clone(),
                    dir,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            root,
            manifest,
            modules,
        })
    }

    pub fn name(&self) -> &str {
        &self.manifest.project.name
    }

    pub fn version(&self) -> &str {
        &self.manifest.project.version
    }

    pub fn module(&self, name: &str) -> Option<&Module> {
        self.modules.iter().find(|module| module.name == name)
    }

    pub fn shared_modules(&self) -> impl Iterator<Item = &Module> {
        self.modules.iter().filter(|module| module.is_shared())
    }

    pub fn modules_for<'a>(&'a self, platform: &'a PlatformId) -> impl Iterator<Item = &'a Module> {
        self.modules
            .iter()
            .filter(move |module| module.platform() == Some(platform))
    }

    pub fn platform_ids(&self) -> Vec<PlatformId> {
        self.modules
            .iter()
            .filter_map(Module::platform)
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn dependencies_of(&self, module: &Module) -> Vec<&Module> {
        module
            .ingot
            .dependencies
            .iter()
            .filter_map(|dependency| self.resolve_dependency(module, dependency))
            .collect()
    }

    pub fn resolve_dependency(
        &self,
        module: &Module,
        dependency: &IngotDependency,
    ) -> Option<&Module> {
        match &dependency.source {
            DependencySource::Path(relative) => {
                let target = module.dir.join(relative).canonicalize().ok()?;
                self.modules.iter().find(|other| other.dir == target)
            }
            DependencySource::Name(name) => {
                if let Ok(Some(workspace)) = Workspace::discover(&module.dir) {
                    if let Some(inherited) = workspace
                        .dependencies
                        .iter()
                        .find(|item| item.alias == dependency.alias)
                    {
                        // Workspace definitions take precedence over member names.
                        if let DependencySource::Path(path) = &inherited.source {
                            let target = workspace.root.join(path).canonicalize().ok()?;
                            return self.modules.iter().find(|other| other.dir == target);
                        }
                        return None;
                    }
                    if let Some(target) = workspace.members.get(name) {
                        return self.modules.iter().find(|other| &other.dir == target);
                    }
                }
                self.modules.iter().find(|other| &other.ingot.name == name)
            }
            DependencySource::Remote(_) => None,
        }
    }

    pub fn requested_bindings(&self, platform: &PlatformId) -> Option<&[String]> {
        self.manifest
            .platforms
            .get(platform.as_str())
            .and_then(|section| section.bindings.as_deref())
    }

    pub fn build_dir(&self, platform: &PlatformId, module: &Module) -> PathBuf {
        self.root
            .join(BUILD_DIR)
            .join(platform.as_str())
            .join(&module.name)
    }

    pub fn dist_dir(&self, platform: &PlatformId) -> PathBuf {
        self.root.join(DIST_DIR).join(platform.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::write_project;

    const MANIFEST: &str = r#"
        [project]
        name = "demo"
        version = "0.1.0"

        [modules.core]
        path = "shared"

        [modules.web]
        path = "wasm"
        platform = "wasm"

        [platform.wasm]
        bindings = []
    "#;
    const SHARED: &str = "[ingot]\nname = \"demo_core\"\n";
    const WASM: &str = "[ingot]\nname = \"demo_web\"\ntarget = \"wasm\"\n\n[dependencies]\nlogic = \"../shared\"\n";

    fn project(dir: &Path) -> Project {
        write_project(dir, MANIFEST, &[("shared", SHARED), ("wasm", WASM)]);
        Project::load(dir).unwrap()
    }

    #[test]
    fn discovers_the_project_from_a_nested_directory() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = project(dir.path());
        let nested = loaded.root.join("wasm");

        assert_eq!(Project::discover(&nested).unwrap().root, loaded.root);
    }

    #[test]
    fn reports_a_missing_project() {
        let dir = tempfile::tempdir().unwrap();
        let error = Project::discover(dir.path()).unwrap_err();
        assert!(matches!(error, Error::ProjectNotFound { .. }));
    }

    #[test]
    fn classifies_modules_by_role() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let wasm = PlatformId::new("wasm");

        let shared: Vec<&str> = project.shared_modules().map(|m| m.name.as_str()).collect();
        let web: Vec<&str> = project
            .modules_for(&wasm)
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(shared, ["core"]);
        assert_eq!(web, ["web"]);
        assert_eq!(project.platform_ids(), [wasm]);
    }

    #[test]
    fn resolves_dependencies_to_project_modules() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());
        let web = project.module("web").unwrap();

        let dependencies: Vec<&str> = project
            .dependencies_of(web)
            .iter()
            .map(|module| module.name.as_str())
            .collect();
        assert_eq!(dependencies, ["core"]);
    }

    #[test]
    fn distinguishes_no_bindings_from_default_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let project = project(dir.path());

        assert_eq!(
            project.requested_bindings(&PlatformId::new("wasm")),
            Some(&[][..])
        );
        assert_eq!(project.requested_bindings(&PlatformId::new("evm")), None);
    }

    #[test]
    fn rejects_a_module_without_an_ingot() {
        let dir = tempfile::tempdir().unwrap();
        write_project(dir.path(), MANIFEST, &[("shared", SHARED)]);

        let error = Project::load(dir.path()).unwrap_err();
        assert!(
            error.to_string().contains("module `web` points at `wasm`"),
            "{error}"
        );
    }
}
