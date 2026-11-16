//! Minimized counterexamples are kept under `regressions/<module>/<entry>.json`
//! with the seed, the choice tape and the context they were found in. Every
//! run replays them before generating new cases.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use femp_core::property::U256;
use femp_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::report::{Counterexample, FailureKindName, TargetOutcome};

pub const DIR: &str = "regressions";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Regression {
    pub failure: FailureKindName,
    pub seed: String,
    pub case: Option<u64>,
    /// The choice tape, in decimal, that regenerates the counterexample.
    pub choices: Vec<String>,
    pub counterexample: Counterexample,
    pub outcomes: Vec<TargetOutcome>,
    pub recorded: Recorded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recorded {
    pub femp: String,
    pub fe: String,
    pub optimize: Vec<String>,
    pub unix_time: u64,
}

impl Recorded {
    pub fn now(fe: &str, optimize: Vec<String>) -> Self {
        Self {
            femp: env!("CARGO_PKG_VERSION").to_string(),
            fe: fe.to_string(),
            optimize,
            unix_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs()),
        }
    }
}

impl Regression {
    pub fn tape(&self) -> Option<Vec<U256>> {
        self.choices
            .iter()
            .map(|choice| U256::from_str_radix(choice, 10).ok())
            .collect()
    }
}

pub fn encode_tape(choices: &[U256]) -> Vec<String> {
    choices.iter().map(U256::to_string).collect()
}

/// The regressions of one project, read once and written back per entry.
pub struct Store {
    root: PathBuf,
    loaded: BTreeMap<(String, String), Vec<Regression>>,
}

impl Store {
    pub fn open(project_root: &Path) -> Self {
        Self {
            root: project_root.join(DIR),
            loaded: BTreeMap::new(),
        }
    }

    pub fn path(&self, module: &str, entry: &str) -> PathBuf {
        self.root.join(module).join(format!("{entry}.json"))
    }

    pub fn load(&mut self, module: &str, entry: &str) -> Result<&[Regression]> {
        let key = (module.to_string(), entry.to_string());
        if !self.loaded.contains_key(&key) {
            let path = self.path(module, entry);
            let regressions = if path.is_file() {
                let text =
                    fs::read_to_string(&path).map_err(|source| Error::io("read", &path, source))?;
                serde_json::from_str(&text).map_err(|error| {
                    Error::Artifact(format!(
                        "{} is not a regression file: {error}",
                        path.display()
                    ))
                })?
            } else {
                Vec::new()
            };
            self.loaded.insert(key.clone(), regressions);
        }
        Ok(&self.loaded[&key])
    }

    /// Adds a regression unless an equal one is stored: one with the same
    /// counterexample, or with the same non-empty tape. Regressions a proof
    /// found carry no tape, so only their counterexample tells them apart.
    /// Returns the file the regression lives in, relative to the project.
    pub fn save(&mut self, module: &str, entry: &str, regression: Regression) -> Result<PathBuf> {
        self.load(module, entry)?;
        let path = self.path(module, entry);
        let stored = self
            .loaded
            .get_mut(&(module.to_string(), entry.to_string()))
            .expect("loaded above");
        let relative = Path::new(DIR).join(module).join(format!("{entry}.json"));
        if stored.iter().any(|existing| {
            existing.counterexample == regression.counterexample
                || (!regression.choices.is_empty() && existing.choices == regression.choices)
        }) {
            return Ok(relative);
        }
        stored.push(regression);
        let text = serde_json::to_string_pretty(stored)
            .map_err(|error| Error::Artifact(format!("cannot encode regressions: {error}")))?;
        femp_core::artifact::write_file(&path, format!("{text}\n"))?;
        Ok(relative)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Arg;

    fn regression(choices: &[&str]) -> Regression {
        found(choices, "1000")
    }

    fn found(choices: &[&str], x: &str) -> Regression {
        Regression {
            failure: FailureKindName::Diverged,
            seed: "0x1".to_string(),
            case: Some(3),
            choices: choices.iter().map(|choice| choice.to_string()).collect(),
            counterexample: Counterexample::Args {
                args: vec![Arg {
                    name: "x".into(),
                    value: x.into(),
                }],
            },
            outcomes: vec![TargetOutcome {
                target: "evm".into(),
                outcome: "returned 1".into(),
            }],
            recorded: Recorded::now("26.6.0", vec!["1".into()]),
        }
    }

    #[test]
    fn saves_and_reloads_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path());
        assert!(store.load("core", "p").unwrap().is_empty());
        let path = store.save("core", "p", regression(&["1000"])).unwrap();
        store.save("core", "p", regression(&["1000"])).unwrap();
        store.save("core", "p", found(&["7"], "7")).unwrap();
        assert_eq!(path, Path::new("regressions/core/p.json"));
        assert!(dir.path().join(&path).is_file());

        let mut reopened = Store::open(dir.path());
        let loaded = reopened.load("core", "p").unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].tape(), Some(vec![U256::from(1000u16)]));
    }

    #[test]
    fn tapeless_regressions_are_told_apart_by_their_values() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path());
        store.save("core", "p", found(&[], "250")).unwrap();
        store.save("core", "p", found(&[], "250")).unwrap();
        store.save("core", "p", found(&[], "7")).unwrap();
        store.save("core", "p", found(&["500"], "250")).unwrap();
        assert_eq!(store.load("core", "p").unwrap().len(), 2);
    }

    #[test]
    fn rejects_malformed_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("regressions/core/p.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{").unwrap();
        assert!(Store::open(dir.path()).load("core", "p").is_err());
    }
}
