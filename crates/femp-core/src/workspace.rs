use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::ingot::{INGOT_FILE, IngotDependency, IngotManifest, parse_dependencies};
use crate::{Error, Result};

pub fn read_config(dir: &Path) -> Result<Table> {
    let path = dir.join(INGOT_FILE);
    fs::read_to_string(&path)
        .map_err(|error| Error::io("read", &path, error))?
        .parse()
        .map_err(|error: toml::de::Error| Error::invalid_manifest(&path, error.message()))
}

pub fn is_workspace(document: &Table) -> bool {
    document.contains_key("workspace")
        || (!document.contains_key("ingot") && document.contains_key("members"))
}

pub struct Workspace {
    pub root: PathBuf,
    pub members: BTreeMap<String, PathBuf>,
    pub dependencies: Vec<IngotDependency>,
}

impl Workspace {
    pub fn discover(dir: &Path) -> Result<Option<Self>> {
        for root in dir
            .ancestors()
            .skip(1)
            .filter(|root| root.join(INGOT_FILE).is_file())
        {
            let document = read_config(root)?;
            if is_workspace(&document) {
                let workspace = Self::load(root.to_owned(), document)?;
                if workspace.members.values().any(|member| member == dir) {
                    return Ok(Some(workspace));
                }
            }
        }
        Ok(None)
    }

    pub fn load(root: PathBuf, document: Table) -> Result<Self> {
        let config = document
            .get("workspace")
            .and_then(Value::as_table)
            .unwrap_or(&document);
        let mut members = BTreeMap::new();
        let entries = config.get("members").or_else(|| document.get("members"));
        let entries: Vec<_> = match entries {
            Some(Value::Array(items)) => items.iter().collect(),
            Some(Value::Table(groups)) => groups
                .values()
                .filter_map(Value::as_array)
                .flatten()
                .collect(),
            _ => Vec::new(),
        };
        let excludes: Vec<_> = config
            .get("exclude")
            .or_else(|| document.get("exclude"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(glob::Pattern::new)
            .collect::<std::result::Result<_, _>>()
            .map_err(|error| Error::invalid_manifest(&root.join(INGOT_FILE), error.to_string()))?;
        let prefix = glob::Pattern::escape(&root.to_string_lossy());
        for entry in entries {
            let (pattern, name) = match entry {
                Value::String(pattern) => (pattern.as_str(), None),
                Value::Table(table) => match table.get("path").and_then(Value::as_str) {
                    Some(path) => (path, table.get("name").and_then(Value::as_str)),
                    None => continue,
                },
                _ => continue,
            };
            let paths = glob::glob(&format!("{prefix}/{pattern}")).map_err(|error| {
                Error::invalid_manifest(&root.join(INGOT_FILE), error.to_string())
            })?;
            for path in paths {
                let path = path.map_err(|error| Error::Artifact(error.to_string()))?;
                if !path.join(INGOT_FILE).is_file()
                    || path.strip_prefix(&root).is_ok_and(|relative| {
                        excludes
                            .iter()
                            .any(|exclude| exclude.matches_path(relative))
                    })
                {
                    continue;
                }
                let path = path
                    .canonicalize()
                    .map_err(|error| Error::io("resolve", &path, error))?;
                let ingot = IngotManifest::read(&path)?;
                members.insert(name.unwrap_or(&ingot.name).to_string(), path);
            }
        }
        Ok(Self {
            root,
            members,
            dependencies: parse_dependencies(&document),
        })
    }
}
