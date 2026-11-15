use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::ingot::SOURCE_DIR;
use crate::naming::is_valid_name;

pub const MANIFEST_FILE: &str = "femp.toml";
pub const DEFAULT_FE_VERSION: &str = "26.6.0";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub project: ProjectSection,
    #[serde(default)]
    pub toolchain: ToolchainSection,
    #[serde(default)]
    pub modules: BTreeMap<String, ModuleSection>,
    #[serde(default, rename = "platform")]
    pub platforms: BTreeMap<String, PlatformSection>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSection {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolchainSection {
    #[serde(default = "default_fe_version")]
    pub fe: String,
}

impl Default for ToolchainSection {
    fn default() -> Self {
        Self {
            fe: default_fe_version(),
        }
    }
}

fn default_fe_version() -> String {
    DEFAULT_FE_VERSION.to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleSection {
    pub path: PathBuf,
    pub platform: Option<String>,
    #[serde(default)]
    pub generate: BTreeMap<String, GeneratorSection>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorSection {
    /// The executable followed by its arguments. Standard output is Fe source.
    pub command: Vec<String>,
    /// Destination relative to the module root in the staged ingot.
    pub output: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformSection {
    /// Absent means "every binding kind the platform offers".
    pub bindings: Option<Vec<String>>,
}

impl Manifest {
    pub fn read(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|source| Error::io("read", path, source))?;
        Self::parse(&text, path)
    }

    pub fn parse(text: &str, path: &Path) -> Result<Self> {
        let manifest: Self =
            toml::from_str(text).map_err(|error| Error::invalid_manifest(path, error.message()))?;
        manifest.validate(path)?;
        Ok(manifest)
    }

    fn validate(&self, path: &Path) -> Result<()> {
        if !is_valid_name(&self.project.name) {
            return Err(Error::invalid_manifest(
                path,
                format!("project name `{}` is not a valid name", self.project.name),
            ));
        }
        if self.modules.is_empty() {
            return Err(Error::invalid_manifest(path, "no modules are declared"));
        }
        if let Some(name) = self.modules.keys().find(|name| !is_valid_name(name)) {
            return Err(Error::invalid_manifest(
                path,
                format!("module name `{name}` is not a valid name"),
            ));
        }
        for (module, section) in &self.modules {
            let mut outputs = BTreeSet::new();
            for (name, generator) in &section.generate {
                let invalid = |message: &str| {
                    Error::invalid_manifest(
                        path,
                        format!("modules.{module}.generate.{name}: {message}"),
                    )
                };
                if !is_valid_name(name) {
                    return Err(invalid("invalid generator name"));
                }
                if generator
                    .command
                    .first()
                    .is_none_or(|program| program.trim().is_empty())
                {
                    return Err(invalid("command must name an executable"));
                }
                let output = &generator.output;
                if !output.starts_with(SOURCE_DIR)
                    || output.extension().is_none_or(|ext| ext != "fe")
                    || output
                        .components()
                        .any(|component| !matches!(component, Component::Normal(_)))
                {
                    return Err(invalid("output must name a .fe file inside src"));
                }
                if !outputs.insert(output) {
                    return Err(invalid(
                        "output is already declared by another generator in this module",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "femp.toml";

    fn parse(text: &str) -> Result<Manifest> {
        Manifest::parse(text, Path::new(PATH))
    }

    #[test]
    fn parses_full_manifest() {
        let manifest = parse(
            r#"
            [project]
            name = "vesting"
            version = "0.1.0"

            [toolchain]
            fe = "26.6.0"

            [modules.core]
            path = "shared"

            [modules.web]
            path = "wasm"
            platform = "wasm"

            [platform.wasm]
            bindings = ["typescript"]
            "#,
        )
        .unwrap();

        assert_eq!(manifest.project.name, "vesting");
        assert_eq!(manifest.modules["core"].platform, None);
        assert_eq!(manifest.modules["web"].platform.as_deref(), Some("wasm"));
        assert_eq!(
            manifest.platforms["wasm"].bindings.as_deref(),
            Some(&["typescript".to_string()][..])
        );
    }

    #[test]
    fn defaults_the_toolchain_version() {
        let manifest = parse(
            r#"
            [project]
            name = "demo"
            version = "0.1.0"

            [modules.core]
            path = "shared"
            "#,
        )
        .unwrap();

        assert_eq!(manifest.toolchain.fe, DEFAULT_FE_VERSION);
    }

    #[test]
    fn rejects_unknown_keys() {
        let error = parse(
            r#"
            [project]
            name = "demo"
            version = "0.1.0"
            flavour = "spicy"

            [modules.core]
            path = "shared"
            "#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("flavour"), "{error}");
    }

    #[test]
    fn rejects_a_project_without_modules() {
        let error = parse(
            r#"
            [project]
            name = "demo"
            version = "0.1.0"
            "#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("no modules"), "{error}");
    }

    #[test]
    fn rejects_invalid_module_names() {
        let error = parse(
            r#"
            [project]
            name = "demo"
            version = "0.1.0"

            [modules."bad name"]
            path = "shared"
            "#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("bad name"), "{error}");
    }
}
