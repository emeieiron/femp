use std::process::Command;

use femp_core::{Error, Result};

#[derive(Debug, Clone)]
pub struct Captured {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Captured {
    pub fn combined(&self) -> String {
        match (self.stdout.trim_end(), self.stderr.trim_end()) {
            (stdout, "") => stdout.to_string(),
            ("", stderr) => stderr.to_string(),
            (stdout, stderr) => format!("{stdout}\n{stderr}"),
        }
    }
}

pub fn describe(command: &Command) -> String {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn capture(command: &mut Command) -> Result<Captured> {
    let output = command.output().map_err(|error| {
        Error::Toolchain(format!("could not run `{}`: {error}", describe(command)))
    })?;
    Ok(Captured {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

pub fn try_stdout(program: &str, args: &[&str]) -> Option<String> {
    let captured = capture(Command::new(program).args(args)).ok()?;
    captured.success.then_some(captured.stdout)
}
