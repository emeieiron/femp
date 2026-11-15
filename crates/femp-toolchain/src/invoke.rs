use std::path::Path;
use std::process::Command;

use femp_core::environment::{OptLevel, Toolchain};
use femp_core::{Error, Result};

use crate::process::{Captured, capture, describe};

pub struct BuildRequest<'a> {
    pub backend: &'a str,
    pub input: &'a Path,
    pub out_dir: &'a Path,
    pub emit: &'a [&'a str],
    pub optimize: OptLevel,
    pub standalone: bool,
}

pub struct TestRequest<'a> {
    pub backend: &'a str,
    pub input: &'a Path,
    pub filter: Option<&'a str>,
    pub optimize: OptLevel,
}

pub fn check(toolchain: &Toolchain, backend: &str, input: &Path) -> Result<()> {
    let mut command = fe(toolchain, "check");
    command.args(["--backend", backend]).arg(input);
    require_success(&mut command).map(|_| ())
}

pub fn build(toolchain: &Toolchain, request: &BuildRequest<'_>) -> Result<()> {
    let mut command = fe(toolchain, "build");
    command
        .args(["--backend", request.backend])
        .args(["--optimize", request.optimize.as_arg()])
        .arg("--out-dir")
        .arg(request.out_dir);
    if !request.emit.is_empty() {
        command.args(["--emit", &request.emit.join(",")]);
    }
    if request.standalone {
        command.arg("--standalone");
    }
    command.arg(request.input);
    require_success(&mut command).map(|_| ())
}

pub fn test(toolchain: &Toolchain, request: &TestRequest<'_>) -> Result<Captured> {
    let mut command = fe(toolchain, "test");
    command
        .args(["--backend", request.backend])
        .args(["--optimize", request.optimize.as_arg()]);
    if let Some(filter) = request.filter {
        command.args(["--filter", filter]);
    }
    command.arg(request.input);
    capture(&mut command)
}

fn fe(toolchain: &Toolchain, subcommand: &str) -> Command {
    let mut command = Command::new(&toolchain.fe);
    command.args(["--color", "never", subcommand]);
    command
}

fn require_success(command: &mut Command) -> Result<Captured> {
    let captured = capture(command)?;
    if captured.success {
        Ok(captured)
    } else {
        Err(Error::Command {
            command: describe(command),
            output: captured.combined(),
        })
    }
}
