use std::env;

use anyhow::Result;
use femp_core::manifest::DEFAULT_FE_VERSION;
use femp_core::project::Project;
use femp_toolchain::{install, resolve};

use crate::cli::{ToolchainArgs, ToolchainCommand};
use crate::console;

pub fn run(args: &ToolchainArgs) -> Result<bool> {
    let version = pinned_version();
    match args.command {
        ToolchainCommand::Install => {
            console::step(
                "Installing",
                &format!("Fe {version} (this compiles the compiler and takes a few minutes)"),
            );
            let fe = install::install(&version)?;
            console::step("Installed", &fe.display().to_string());
        }
        ToolchainCommand::Path => {
            console::line(format!("{}", resolve::locate(&version)?.display()))
        }
    }
    Ok(true)
}

pub fn pinned_version() -> String {
    env::current_dir()
        .ok()
        .and_then(|dir| Project::discover(&dir).ok())
        .map(|project| project.manifest.toolchain.fe)
        .unwrap_or_else(|| DEFAULT_FE_VERSION.to_string())
}
