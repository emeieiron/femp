use clap::{Args, Parser, Subcommand};
use femp_core::environment::OptLevel;

#[derive(Parser)]
#[command(
    name = "femp",
    version,
    about = "Build one Fe codebase for EVM, native, and WebAssembly"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a new multiplatform project
    New(NewArgs),
    /// Report what the toolchain and this machine can build
    Doctor,
    /// Manage the Fe compiler used by femp
    Toolchain(ToolchainArgs),
    /// Materialize project-generated sources for editing and code navigation
    Generate,
    /// Type-check every module for the platforms it must support
    Check(TargetArgs),
    /// Compile platform modules and generate their bindings
    Build(BuildArgs),
    /// Run tests on every platform and compare the outcomes
    Test(TestArgs),
    /// Build, then collect distributable packages under `dist/`
    Package(BuildArgs),
    /// Remove build and package output
    Clean,
}

#[derive(Args)]
pub struct NewArgs {
    /// Name of the project and of the directory to create
    pub name: String,
}

#[derive(Args)]
pub struct ToolchainArgs {
    #[command(subcommand)]
    pub command: ToolchainCommand,
}

#[derive(Subcommand)]
pub enum ToolchainCommand {
    /// Build the pinned Fe compiler from source with every backend femp uses
    Install,
    /// Print the path of the Fe compiler femp would run
    Path,
}

#[derive(Args)]
pub struct TargetArgs {
    /// Platforms to operate on; defaults to every platform the project targets
    #[arg(short, long = "target", value_name = "PLATFORM")]
    pub targets: Vec<String>,
}

#[derive(Args)]
pub struct BuildArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    /// Optimization level: 0, 1, 2 or s
    #[arg(short = 'O', long, value_name = "LEVEL", default_value = "1", value_parser = parse_opt_level)]
    pub optimize: OptLevel,
}

#[derive(Args)]
pub struct TestArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    /// Only run tests whose name contains this text
    #[arg(short, long, value_name = "PATTERN")]
    pub filter: Option<String>,

    /// Print the compiler's full output for every suite, not only failing ones
    #[arg(long)]
    pub verbose: bool,
}

fn parse_opt_level(text: &str) -> Result<OptLevel, String> {
    OptLevel::parse(text).ok_or_else(|| "expected 0, 1, 2 or s".to_string())
}
