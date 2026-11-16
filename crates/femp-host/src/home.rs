use std::env;
use std::path::PathBuf;

use femp_core::{Error, Result};

pub const HOME_ENV: &str = "FEMP_HOME";

/// Where femp's installer keeps what it installs: `FEMP_HOME`, or `.femp`
/// in the user's home directory.
pub fn home_dir() -> Result<PathBuf> {
    if let Some(home) = env::var_os(HOME_ENV) {
        return Ok(PathBuf::from(home));
    }
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".femp"))
        .ok_or_else(|| Error::Toolchain(format!("cannot find a home directory; set {HOME_ENV}")))
}
