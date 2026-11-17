//! Helpers shared by the tests that drive the `femp` binary. They need the
//! Fe compiler and skip themselves when the machine cannot build every
//! platform a project targets.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const GENERATED_DIRS: &[&str] = &["build", "dist", "regressions"];

pub fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if GENERATED_DIRS.iter().any(|generated| name == *generated) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

pub fn femp(project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_femp"))
        .args(args)
        .current_dir(project)
        .env_remove("FEMP_PROP_SEED")
        .output()
        .unwrap()
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A private copy of the project at `relative`, or `None` when this machine
/// cannot build it.
pub fn project(relative: &str) -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&fixture(relative), dir.path());
    if femp(dir.path(), &["doctor"]).status.success() {
        Some(dir)
    } else {
        eprintln!("skipping: `femp doctor` reports that this machine cannot build {relative}");
        None
    }
}

/// A project whose one module is the compiler's token fixture: a contract
/// with a storage map, a broken invariant and message claims.
#[allow(dead_code)]
pub fn token_project() -> tempfile::TempDir {
    contract_project("token")
}

/// A project whose one module is one of the compiler's contract fixtures.
#[allow(dead_code)]
pub fn contract_project(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(
        &fixture(&format!("../femp-compiler/tests/fixtures/{name}")),
        &dir.path().join("evm"),
    );
    fs::write(
        dir.path().join("femp.toml"),
        format!(
            "[project]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[modules.contracts]\npath = \"evm\"\nplatform = \"evm\"\n"
        ),
    )
    .unwrap();
    dir
}
