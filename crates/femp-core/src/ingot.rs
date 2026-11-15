use std::fs;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::error::{Error, Result};
use crate::naming::looks_like_version;

pub const INGOT_FILE: &str = "fe.toml";
pub const SOURCE_DIR: &str = "src";
pub const ROOT_SOURCE: &str = "src/lib.fe";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngotManifest {
    pub name: String,
    pub version: Option<String>,
    pub target: Option<String>,
    pub dependencies: Vec<IngotDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngotDependency {
    pub alias: String,
    pub name: Option<String>,
    pub source: DependencySource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencySource {
    Path(PathBuf),
    Name(String),
    Remote(GitDependency),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDependency {
    pub source: String,
    pub rev: String,
    pub path: Option<PathBuf>,
}

impl IngotManifest {
    pub fn read(dir: &Path) -> Result<Self> {
        let path = dir.join(INGOT_FILE);
        let text = fs::read_to_string(&path).map_err(|source| Error::io("read", &path, source))?;
        Self::parse(&text, &path)
    }

    pub fn parse(text: &str, path: &Path) -> Result<Self> {
        let document: Table = text
            .parse()
            .map_err(|error: toml::de::Error| Error::invalid_manifest(path, error.message()))?;
        let ingot = document
            .get("ingot")
            .and_then(Value::as_table)
            .ok_or_else(|| Error::invalid_manifest(path, "missing `[ingot]` table"))?;
        let name = string_field(ingot, "name")
            .ok_or_else(|| Error::invalid_manifest(path, "missing `ingot.name`"))?;

        Ok(Self {
            name,
            version: string_field(ingot, "version"),
            target: string_field(ingot, "target"),
            dependencies: parse_dependencies(&document),
        })
    }
}

fn string_field(table: &Table, key: &str) -> Option<String> {
    table.get(key).and_then(Value::as_str).map(str::to_string)
}

pub fn parse_dependencies(document: &Table) -> Vec<IngotDependency> {
    let Some(dependencies) = document.get("dependencies").and_then(Value::as_table) else {
        return Vec::new();
    };
    dependencies
        .iter()
        .filter_map(|(alias, value)| {
            dependency_source(alias, value).map(|source| IngotDependency {
                alias: alias.clone(),
                name: value
                    .as_table()
                    .and_then(|table| string_field(table, "name")),
                source,
            })
        })
        .collect()
}

// Mirrors how the Fe compiler reads a dependency entry: a bare string is a
// version of the ingot named by the alias when it parses as one, and a
// relative path otherwise; a table is remote when it has `source`, a path
// when it has `path`, and a named ingot in every other case.
fn dependency_source(alias: &str, value: &Value) -> Option<DependencySource> {
    match value {
        Value::Boolean(true) => Some(DependencySource::Name(alias.to_string())),
        Value::String(text) if looks_like_version(text) => {
            Some(DependencySource::Name(alias.to_string()))
        }
        Value::String(text) => Some(DependencySource::Path(PathBuf::from(text))),
        Value::Table(table) if table.contains_key("source") => {
            Some(DependencySource::Remote(GitDependency {
                source: string_field(table, "source")?,
                rev: string_field(table, "rev")?,
                path: string_field(table, "path").map(PathBuf::from),
            }))
        }
        Value::Table(table) => Some(match string_field(table, "path") {
            Some(path) => DependencySource::Path(PathBuf::from(path)),
            None => DependencySource::Name(
                string_field(table, "name").unwrap_or_else(|| alias.to_string()),
            ),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<IngotManifest> {
        IngotManifest::parse(text, Path::new("fe.toml"))
    }

    #[test]
    fn parses_name_version_and_target() {
        let ingot = parse(
            r#"
            [ingot]
            name = "vesting_cli"
            version = "0.1.0"
            target = "native"
            "#,
        )
        .unwrap();

        assert_eq!(ingot.name, "vesting_cli");
        assert_eq!(ingot.version.as_deref(), Some("0.1.0"));
        assert_eq!(ingot.target.as_deref(), Some("native"));
        assert!(ingot.dependencies.is_empty());
    }

    #[test]
    fn classifies_each_dependency_form() {
        let ingot = parse(
            r#"
            [ingot]
            name = "app"

            [dependencies]
            by_path = "../shared"
            by_table = { path = "../other", allow_unsafe = true }
            by_version = "1.2.3"
            renamed = { name = "real_name", version = "0.1.0" }
            fetched = { source = "https://example.com/repo.git", rev = "abc" }
            "#,
        )
        .unwrap();

        let source = |alias: &str| {
            ingot
                .dependencies
                .iter()
                .find(|dependency| dependency.alias == alias)
                .map(|dependency| dependency.source.clone())
                .unwrap()
        };
        assert_eq!(
            source("by_path"),
            DependencySource::Path("../shared".into())
        );
        assert_eq!(
            source("by_table"),
            DependencySource::Path("../other".into())
        );
        assert_eq!(
            source("by_version"),
            DependencySource::Name("by_version".into())
        );
        assert_eq!(
            source("renamed"),
            DependencySource::Name("real_name".into())
        );
        assert_eq!(
            source("fetched"),
            DependencySource::Remote(GitDependency {
                source: "https://example.com/repo.git".into(),
                rev: "abc".into(),
                path: None,
            })
        );
    }

    #[test]
    fn requires_an_ingot_table() {
        let error = parse("[workspace]\nname = \"ws\"\n").unwrap_err();
        assert!(error.to_string().contains("[ingot]"), "{error}");
    }
}
