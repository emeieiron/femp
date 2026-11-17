mod claims;
mod cli;
mod commands;
mod console;
mod contracts;
mod generation;
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
        Command::Check(args) => commands::check::run(&args),
        Command::Build(args) => commands::build::run(&args),
        Command::Test(args) => commands::test::run(&args),
        Command::Prop(args) => commands::prop::run(&args),
        Command::Prove(args) => commands::prove::run(&args),
        Command::Package(args) => commands::package::run(&args),
        Command::Clean => commands::clean::run(),
        Command::Generate => std::env::current_dir()
            .map_err(anyhow::Error::from)
            .and_then(|dir| Ok(femp_core::project::Project::discover(&dir)?))
            .and_then(|project| generation::editor_sources(&project).map(|()| true)),
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
