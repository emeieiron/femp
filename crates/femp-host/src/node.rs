//! Finds the Node.js that runs WebAssembly code. A new enough `node` on
//! `PATH` wins. Otherwise femp uses the copy its installer keeps under
//! `FEMP_HOME/node`, which is not put on the user's `PATH`, so installing
//! femp never changes which `node` other tools see.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::home::home_dir;
use crate::process::capture;

pub const NODE_ENV: &str = "FEMP_NODE";
pub const MIN_MAJOR: u32 = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub path: PathBuf,
    pub major: u32,
}

/// The Node.js femp uses, found once per process. An older one is returned
/// when nothing new enough exists, so that `femp doctor` can name it.
pub fn found() -> Option<&'static Node> {
    static FOUND: OnceLock<Option<Node>> = OnceLock::new();
    FOUND.get_or_init(find).as_ref()
}

fn find() -> Option<Node> {
    if let Some(path) = env::var_os(NODE_ENV) {
        return probe(Path::new(&path));
    }
    let found: Vec<Node> = candidates().iter().filter_map(|path| probe(path)).collect();
    choose(found)
}

fn candidates() -> Vec<PathBuf> {
    let binary = format!("node{}", env::consts::EXE_SUFFIX);
    let on_path = env::var_os("PATH")
        .map(|path| {
            env::split_paths(&path)
                .map(|dir| dir.join(&binary))
                .find(|candidate| candidate.is_file())
        })
        .unwrap_or_default();
    let managed = home_dir()
        .ok()
        .map(|home| home.join("node").join("bin").join(&binary));
    on_path.into_iter().chain(managed).collect()
}

fn choose(found: Vec<Node>) -> Option<Node> {
    let usable = found.iter().position(|node| node.major >= MIN_MAJOR);
    match usable {
        Some(index) => found.into_iter().nth(index),
        None => found.into_iter().next(),
    }
}

fn probe(path: &Path) -> Option<Node> {
    let captured = capture(Command::new(path).arg("--version")).ok()?;
    let major = captured.success.then(|| parse_major(&captured.stdout))??;
    Some(Node {
        path: path.to_path_buf(),
        major,
    })
}

pub fn parse_major(version: &str) -> Option<u32> {
    version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// `PATH` for child processes with the chosen Node's directory first, so
/// that tools femp runs, such as the compiler's WebAssembly test runner,
/// use the same Node.
pub fn search_path() -> Option<OsString> {
    let dir = found()?.path.parent()?.to_path_buf();
    let rest = env::var_os("PATH").unwrap_or_default();
    env::join_paths(std::iter::once(dir).chain(env::split_paths(&rest))).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(path: &str, major: u32) -> Node {
        Node {
            path: PathBuf::from(path),
            major,
        }
    }

    #[test]
    fn reads_major_versions() {
        assert_eq!(parse_major("v24.16.0\n"), Some(24));
        assert_eq!(parse_major("18.0.0"), Some(18));
        assert_eq!(parse_major("node"), None);
    }

    #[test]
    fn prefers_the_first_node_that_is_new_enough() {
        let old = node("/usr/bin/node", 18);
        let managed = node("/home/me/.femp/node/bin/node", 24);
        assert_eq!(choose(vec![old.clone(), managed.clone()]), Some(managed));
        assert_eq!(choose(vec![old.clone()]), Some(old));
        assert_eq!(choose(Vec::new()), None);
    }
}
