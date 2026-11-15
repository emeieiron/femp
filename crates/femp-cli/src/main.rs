mod cli;
mod commands;
mod console;
mod pipeline;
mod registry;
mod session;

use std::process::ExitCode;

use clap::Parser;

use cli::{Cli, Command};

fn main() -> ExitCode {
    let outcome = match Cli::parse().command {
        Command::New(args) => commands::new::run(&args),
        Command::Doctor => commands::doctor::run(),
        Command::Toolchain(args) => commands::toolchain::run(&args),
        Command::Check(args) => commands::check::run(&args),
        Command::Build(args) => commands::build::run(&args),
        Command::Test(args) => commands::test::run(&args),
        Command::Package(args) => commands::package::run(&args),
        Command::Clean => commands::clean::run(),
    };
    match outcome {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            console::error(&format!("{error:#}"));
            ExitCode::FAILURE
        }
    }
}
