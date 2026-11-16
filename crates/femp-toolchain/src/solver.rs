//! Finds the Z3 solver that `femp prove` drives: `FEMP_Z3` when set, then
//! the copy femp's installer keeps under `FEMP_HOME/z3`, then `z3` on `PATH`.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::process::capture;
use crate::resolve::home_dir;

pub const Z3_ENV: &str = "FEMP_Z3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Z3 {
    pub path: PathBuf,
    pub version: String,
}

/// The solver femp uses, found once per process.
pub fn found() -> Option<&'static Z3> {
    static FOUND: OnceLock<Option<Z3>> = OnceLock::new();
    FOUND.get_or_init(find).as_ref()
}

fn find() -> Option<Z3> {
    if let Some(path) = env::var_os(Z3_ENV) {
        return probe(Path::new(&path));
    }
    candidates().iter().find_map(|path| probe(path))
}

fn binary() -> String {
    format!("z3{}", env::consts::EXE_SUFFIX)
}

fn candidates() -> Vec<PathBuf> {
    let managed = home_dir()
        .ok()
        .map(|home| home.join("z3").join("bin").join(binary()));
    let on_path = env::var_os("PATH")
        .map(|path| {
            env::split_paths(&path)
                .map(|dir| dir.join(binary()))
                .find(|candidate| candidate.is_file())
        })
        .unwrap_or_default();
    managed.into_iter().chain(on_path).collect()
}

fn probe(path: &Path) -> Option<Z3> {
    let captured = capture(Command::new(path).arg("--version")).ok()?;
    let version = captured
        .success
        .then(|| parse_version(&captured.stdout))??;
    Some(Z3 {
        path: path.to_path_buf(),
        version,
    })
}

/// Reads `Z3 version 4.13.0 - 64 bit`.
pub fn parse_version(text: &str) -> Option<String> {
    let rest = text.trim().strip_prefix("Z3 version ")?;
    let version = rest.split_whitespace().next()?;
    version
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
        .then(|| version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_versions() {
        assert_eq!(
            parse_version("Z3 version 4.13.0 - 64 bit\n").as_deref(),
            Some("4.13.0")
        );
        assert_eq!(parse_version("z3 4.8"), None);
        assert_eq!(parse_version("Z3 version - 64 bit"), None);
    }
}
