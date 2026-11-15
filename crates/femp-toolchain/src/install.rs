use std::path::{Path, PathBuf};
use std::process::Command;

use femp_core::{Error, Result};

use crate::process::{run_attached, try_stdout};
use crate::resolve::{managed_binary, managed_root};

const REPOSITORY: &str = "https://github.com/argotorg/fe";
/// Release binaries ship without the WebAssembly backend, so the managed
/// toolchain is built from source with every backend the framework drives.
const FEATURES: &str = "cranelift,wasm";
const MIN_RUST_MINOR: u32 = 97;
const SOURCE_DIR: &str = "src";
const COMPILER_CRATE: &str = "crates/fe";
const GRAMMAR_CRATE: &str = "crates/tree-sitter-fe";

pub fn install(version: &str) -> Result<PathBuf> {
    let root = managed_root(version)?;
    let source = root.join(SOURCE_DIR);

    fetch_source(version, &source)?;
    prepare_parser_generator(&source.join(GRAMMAR_CRATE))?;
    build_compiler(&source.join(COMPILER_CRATE), &root)?;
    managed_binary(version)
}

fn fetch_source(version: &str, source: &Path) -> Result<()> {
    if source.join(".git").is_dir() {
        return Ok(());
    }
    run_attached(
        Command::new("git")
            .args(["clone", "--depth", "1", "--branch"])
            .arg(format!("v{version}"))
            .arg(REPOSITORY)
            .arg(source),
    )
}

// The compiler's grammar crate generates its parser at build time with the
// tree-sitter CLI, looking first in its own `node_modules` and then on
// `PATH`. Installing the version pinned by the crate's lockfile keeps the
// generated parser identical to the one upstream releases are built with.
fn prepare_parser_generator(grammar: &Path) -> Result<()> {
    if try_stdout("tree-sitter", &["--version"]).is_some() {
        return Ok(());
    }
    if try_stdout("npm", &["--version"]).is_none() {
        return Err(Error::Toolchain(
            "building the Fe compiler needs Node.js with npm (or a `tree-sitter` CLI on PATH) \
             to generate its parser"
                .to_string(),
        ));
    }
    run_attached(
        Command::new("npm")
            .args(["ci", "--ignore-scripts"])
            .current_dir(grammar),
    )?;
    run_attached(
        Command::new("npm")
            .args(["rebuild", "tree-sitter-cli"])
            .current_dir(grammar),
    )
}

fn build_compiler(compiler: &Path, root: &Path) -> Result<()> {
    let mut command = Command::new("cargo");
    if let Some(toolchain) = rust_toolchain_override()? {
        command.arg(format!("+{toolchain}"));
    }
    command
        .args(["install", "--locked", "--features", FEATURES, "--path"])
        .arg(compiler)
        .arg("--root")
        .arg(root);
    run_attached(&mut command)
}

fn rust_toolchain_override() -> Result<Option<String>> {
    let default_minor = try_stdout("rustc", &["--version"])
        .as_deref()
        .and_then(rust_minor);
    if default_minor.is_some_and(|minor| minor >= MIN_RUST_MINOR) {
        return Ok(None);
    }
    let installed = try_stdout("rustup", &["toolchain", "list"]).unwrap_or_default();
    newest_sufficient_toolchain(&installed)
        .map(Some)
        .ok_or_else(|| {
            Error::Toolchain(format!(
                "building the Fe compiler needs Rust 1.{MIN_RUST_MINOR} or newer; \
                 run `rustup update stable`"
            ))
        })
}

fn newest_sufficient_toolchain(rustup_list: &str) -> Option<String> {
    rustup_list
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter_map(|name| Some((rust_minor(name)?, name)))
        .filter(|(minor, _)| *minor >= MIN_RUST_MINOR)
        .max_by_key(|(minor, _)| *minor)
        .map(|(_, name)| name.to_string())
}

/// Finds the minor version in text such as `rustc 1.98.0 (..)` or
/// `1.98.0-aarch64-apple-darwin`. Channel names like `stable` have none.
fn rust_minor(text: &str) -> Option<u32> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find_map(|token| token.strip_prefix("1."))
        .and_then(|rest| rest.split('.').next())
        .and_then(|minor| minor.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_minor_versions() {
        assert_eq!(rust_minor("rustc 1.96.0 (ac68faa20 2026-05-25)"), Some(96));
        assert_eq!(rust_minor("1.98.0-aarch64-apple-darwin"), Some(98));
        assert_eq!(rust_minor("stable-aarch64-apple-darwin (default)"), None);
    }

    #[test]
    fn picks_the_newest_toolchain_that_is_new_enough() {
        let list = "stable-aarch64-apple-darwin (active, default)\n\
                    1.95.0-aarch64-apple-darwin\n\
                    1.97.1-aarch64-apple-darwin\n\
                    1.98.0-aarch64-apple-darwin\n";
        assert_eq!(
            newest_sufficient_toolchain(list).as_deref(),
            Some("1.98.0-aarch64-apple-darwin")
        );
        assert_eq!(
            newest_sufficient_toolchain("1.95.0-x86_64-unknown-linux-gnu\n"),
            None
        );
    }
}
