use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use femp_core::{Error, Result};

use crate::process::capture;

pub const FE_OVERRIDE_ENV: &str = "FEMP_FE";
pub const HOME_ENV: &str = "FEMP_HOME";

const INSTALL_HINT: &str = "run `femp toolchain install`";

pub fn home_dir() -> Result<PathBuf> {
    if let Some(home) = env::var_os(HOME_ENV) {
        return Ok(PathBuf::from(home));
    }
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".femp"))
        .ok_or_else(|| Error::Toolchain(format!("cannot find a home directory; set {HOME_ENV}")))
}

pub fn managed_root(version: &str) -> Result<PathBuf> {
    Ok(home_dir()?.join("toolchains").join(format!("fe-{version}")))
}

pub fn managed_binary(version: &str) -> Result<PathBuf> {
    Ok(managed_root(version)?.join("bin").join(binary_name()))
}

fn binary_name() -> String {
    format!("fe{}", env::consts::EXE_SUFFIX)
}

pub fn locate(version: &str) -> Result<PathBuf> {
    if let Some(path) = env::var_os(FE_OVERRIDE_ENV) {
        let path = PathBuf::from(path);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(Error::Toolchain(format!(
                "{FE_OVERRIDE_ENV} points at `{}`, which does not exist",
                path.display()
            )))
        };
    }
    let managed = managed_binary(version)?;
    if managed.is_file() {
        return Ok(managed);
    }
    find_on_path(&binary_name())
        .ok_or_else(|| Error::Toolchain(format!("no Fe {version} compiler found; {INSTALL_HINT}")))
}

fn find_on_path(binary: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())
}

pub fn installed_version(fe: &Path) -> Result<String> {
    let output = capture(Command::new(fe).arg("--version"))?;
    parse_version(&output.stdout).ok_or_else(|| {
        Error::Toolchain(format!(
            "`{} --version` printed no version: {}",
            fe.display(),
            output.combined()
        ))
    })
}

pub fn ensure_version(fe: &Path, pinned: &str) -> Result<String> {
    let found = installed_version(fe)?;
    if found == pinned {
        Ok(found)
    } else {
        Err(Error::Toolchain(format!(
            "`{}` is Fe {found}, but the project pins Fe {pinned}; {INSTALL_HINT}",
            fe.display()
        )))
    }
}

fn parse_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|word| word.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_version_number() {
        assert_eq!(parse_version("fe 26.6.0\n").as_deref(), Some("26.6.0"));
        assert_eq!(
            parse_version("fe 26.6.0 (5b3f96a 2026-11-10)").as_deref(),
            Some("26.6.0")
        );
        assert_eq!(parse_version("fe"), None);
    }
}
