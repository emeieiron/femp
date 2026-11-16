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
    /// Run property and differential tests with generated inputs on every platform
    Prop(PropArgs),
    /// Prove properties, machines and specs for all inputs with an SMT solver
    Prove(ProveArgs),
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

#[derive(Args)]
pub struct PropArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    /// Only run entry points whose name contains this text
    #[arg(short, long, value_name = "PATTERN")]
    pub filter: Option<String>,

    /// Optimization levels to compile every platform at, comma separated
    #[arg(short = 'O', long, value_name = "LEVELS", value_delimiter = ',', default_value = "1", value_parser = parse_opt_level)]
    pub optimize: Vec<OptLevel>,

    /// Cases or machine runs per entry point, overriding their own settings
    #[arg(long, value_name = "N")]
    pub cases: Option<u32>,

    /// Seed for generating cases, decimal or 0x hex; random when omitted
    #[arg(long, value_name = "SEED", env = "FEMP_PROP_SEED")]
    pub seed: Option<String>,

    /// Enumerate every input of entry points whose domain is small enough
    #[arg(long)]
    pub exhaustive: bool,

    /// Report failures as found, without minimizing them
    #[arg(long)]
    pub no_shrink: bool,

    /// Most replays spent minimizing one failure
    #[arg(long, value_name = "N", default_value_t = femp_prop::engine::DEFAULT_MAX_SHRINKS)]
    pub max_shrinks: usize,

    /// How targets must agree: normalized, strict or lenient
    #[arg(long, value_name = "MODE", default_value = "normalized")]
    pub compare: String,

    /// Neither replay nor store regressions
    #[arg(long)]
    pub no_regressions: bool,

    /// Seconds a single case may run before it counts as a timeout
    #[arg(long, value_name = "SECONDS", default_value_t = 10)]
    pub timeout: u64,

    /// List the entry points instead of running them
    #[arg(long)]
    pub list: bool,

    /// Also write the report as JSON to `build/prop/report.json`
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct ProveArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    /// Only prove claims whose name contains this text
    #[arg(short, long, value_name = "PATTERN")]
    pub filter: Option<String>,

    /// Seconds the solver may spend on each assertion
    #[arg(long, value_name = "SECONDS", default_value_t = 20)]
    pub timeout: u64,

    /// Times a loop without an `invariant(..)` is unrolled
    #[arg(long, value_name = "N", default_value_t = 8)]
    pub unroll: u32,

    /// Steps the search for a machine counterexample takes from `#[init]`
    #[arg(long, value_name = "K", default_value_t = 6)]
    pub steps: u32,

    /// Tasks proved at once; the number of CPUs by default
    #[arg(long, value_name = "N")]
    pub jobs: Option<usize>,

    /// Report counterexamples without running them on the compiled targets
    #[arg(long)]
    pub no_replay: bool,

    /// List the claims instead of proving them
    #[arg(long)]
    pub list: bool,

    /// Also write the report as JSON to `build/prove/report.json`
    #[arg(long)]
    pub json: bool,

    /// Write each task's VIR after every pass under `build/prove/vir`
    #[arg(long)]
    pub dump_vir: bool,

    /// Keep each task's SMT-LIB script under `build/prove/smt`
    #[arg(long)]
    pub keep_smt: bool,

    /// Also fail on claims the prover does not support
    #[arg(long)]
    pub strict: bool,
}

fn parse_opt_level(text: &str) -> Result<OptLevel, String> {
    OptLevel::parse(text).ok_or_else(|| "expected 0, 1, 2 or s".to_string())
}
