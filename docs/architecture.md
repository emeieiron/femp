# Architecture

FeMP is a build orchestrator. It owns the project model, decides which compiler invocations a
command needs, and turns compiler output into artifacts, bindings and reports. Compilation
itself is always done by `fe`.

## Crates

```
femp-cli ──┬────────────┬────────────┬────────────┬──────────────┬─────────────┐
   │       │            │            │            │              │             │
femp-testing  femp-prop  femp-prove  femp-frontend  femp-evm   femp-native  femp-wasm
   │       │      │      │      │      │             │              ├─────────────┤
   │       │      └──┬───┘      femp-vir             │         femp-harness       │
   │       │     femp-spec          │                └──────┬───────┴─────────────┘
   │       │         │              │                femp-toolchain
   └───────┴─────────┴──────────────┴───────────────────────┴─── femp-core
```

| Crate | Responsibility | Depends on |
| --- | --- | --- |
| `femp-core` | `femp.toml` and `fe.toml` parsing, the module graph, layering rules, the `Platform` trait and registry, artifact and test-outcome types, a small source writer for generated code. No process spawning. | – |
| `femp-toolchain` | Finding the pinned compiler, building it from source, probing its backends and the host (Node.js, a C compiler, Z3), running `fe check/build/test`, parsing test output. The only crate that knows the compiler's command line. | core |
| `femp-evm`, `femp-native`, `femp-wasm` | One `Platform` implementation each: availability, build, bindings, packaging. | core, toolchain |
| `femp-testing` | Planning which module runs on which platform, running the jobs, and the parity matrix. | core |
| `femp-spec` | The claims both checking tools read: discovering `#[property]`, `#[differential]`, `#[machine]` and `#[spec]` declarations in Fe sources, resolving `#[gen]` texts into domains, and the counterexample format. | core |
| `femp-prop` | Property testing: generators and choice tapes, the engine, shrinking, comparison, regressions and reports. No process spawning, no platform knowledge. | core, spec |
| `femp-vir` | The verification IR: types, SSA functions with block parameters, a builder and a stable text form. No compiler types. | core |
| `femp-frontend` | Loads an ingot with the Fe compiler's own crates (a Git dependency pinned to the release tag) and lowers the functions claims name into VIR. The only crate that links the compiler. | core, vir |
| `femp-prove` | Proving: tasks per claim, the passes, verification conditions, the Z3 process, minimization, verdicts and reports. No compiler types, no platform knowledge. | core, spec, vir |
| `femp-harness` | Child processes that answer a line protocol, for harnesses that run as processes (native, and WebAssembly through Node.js). | core |
| `femp-cli` | Argument parsing, the session (project + registry + environment), commands, console output, project templates. `registry.rs` is the only file that names concrete platforms. | all |

`femp-core::property` holds the data the engine and the adapters share: primitive types and
values, normalized outcomes, suites of entry points, and the `Executor` trait.

## The platform extension point

Everything target-specific is behind `femp_core::platform::Platform`:

```rust
pub trait Platform {
    fn id(&self) -> &PlatformId;
    fn fe_target(&self) -> &str;
    fn stability(&self) -> Stability;
    fn capability(&self, environment: &Environment) -> Capability;
    fn binding_kinds(&self) -> &[&'static str];
    fn check(&self, cx: &Context<'_>, module: &Module) -> Result<()>;
    fn build(&self, cx: &Context<'_>, module: &Module, out_dir: &Path) -> Result<Vec<Artifact>>;
    fn test(&self, cx: &Context<'_>, module: &Module, filter: Option<&str>) -> Result<TestRun>;
    fn bindings(&self, cx: &Context<'_>, module: &Module, built: &[Artifact], kinds: &[String], out_dir: &Path) -> Result<Vec<Artifact>>;
    fn package(&self, cx: &Context<'_>, module: &Module, built: &[Artifact], dist_dir: &Path) -> Result<Vec<Artifact>>;
    fn property_harness(&self, cx: &Context<'_>, request: &HarnessRequest<'_>) -> Result<Box<dyn Executor>>;
}
```

Platform ids and artifact kinds are open sets (`PlatformId` wraps a string, `ArtifactKind` a
static string), so a new platform needs no change to shared enums. See
[adding-a-platform.md](adding-a-platform.md).

`capability` receives plain data — the compiler's version and backends, the host's OS,
architecture and tools — and answers `Available` or `Unavailable { reason, fix }`. Commands use
it to skip or refuse a platform before anything is compiled, which is how the experimental
backends degrade gracefully.

## How a command runs

1. **Session.** Find `femp.toml` upward from the current directory, load every module's
   `fe.toml`, validate the layering rules against the registry, then locate the compiler and
   probe the environment.
2. **Selection.** Resolve `--target` to platforms (see the README for the skip/refuse rule).
3. **Work.** Per command:

| Command | Invocations |
| --- | --- |
| `check` | For each platform: `fe check --backend <b>` on every shared module and on the platform's own modules. |
| `build` | For each platform module: the adapter's `build`, then `bindings`. |
| `test` | For each platform: `fe test --backend <b>` on every shared module and the platform's own modules; results feed the parity matrix. |
| `package` | `build`, then each adapter's `package`, then `dist/manifest.json`. |
| `prop` | Discovery in the Fe sources, then for each module with entry points and each target (platform × optimization level) the adapter's `property_harness`: render, `fe build`, start an executor. The engine then drives every executor. |

One `fe` invocation compiles for one target, and a workspace whose members set different targets
is rejected by the compiler. Driving the compiler once per module and platform is what lets a
single project hold all three.

## Per-platform notes

**EVM.** `fe build --emit bytecode,runtime-bytecode,abi,metadata` on the ingot. The compiler
names artifacts after contracts, so the ABI files it wrote identify the contracts. Bindings
embed the ABI verbatim `as const`, the bytecode, and typed signatures derived from the ABI.

**Native.** `fe build --backend native` on the ingot, producing an executable named after it.
Available on x86-64 Linux and AArch64 macOS with a C compiler.

**WebAssembly.** Fe 26.6 accepts one source file for a WebAssembly build. The adapter therefore
bundles first (`femp-wasm/src/bundle`): the module's sources stay at the root of the generated
file, and each project module it depends on becomes an inline module named after its ingot.
Paths are redirected accordingly — a dependency alias `dep::x` becomes `ingot::<name>::x`, and a
dependency's own `ingot::x` becomes `ingot::<name>::x` — by a scanner that only touches the
first segment of a path and leaves comments and string literals alone. The real ingot is checked
before bundling so mistakes are reported against the original files. When Fe gains ingot builds
for WebAssembly, the bundle step can be removed without touching anything outside this adapter.

The bindings are generated from the manifest the compiler writes next to the module. The wrapper
imports `runtime/wasm-host/host.js`, a dependency-free implementation of Fe's WebAssembly host
contract: it verifies the module against the manifest (versions, SHA-256, imports, exports),
supplies the `env` imports (log, clock, abort, read-only files), places text arguments in the
module's memory for the duration of a call, and converts values by their Fe types.

## Property testing

`femp prop` is described for users in [property-testing.md](property-testing.md). Internally:

1. **Discovery** (`femp-spec/src/discover`) tokenizes each module's sources and reads the
   attributes and signatures of entry points. It does not resolve names. The harness calls each
   entry point by its path, so the compiler checks anything discovery reads wrongly.
2. **Generation** (`cases`, `generators`, `choices`): every case is a function of a tape of
   bounded choices. Random generation records the tape, exhaustive runs construct it, and
   regressions store it.
3. **Execution** (`engine`): each batch of cases goes to every executor in parallel, one
   thread per target. Each case is judged by a pluggable `Comparator` (`verdict`).
4. **Shrinking** (`shrink`) edits the failing tape with independent `ShrinkPass`es and keeps
   only simpler tapes that still fail the same way.
5. **Results** (`regress`, `report`): failures are written to `regressions/`, and the report is
   rendered as text or JSON.

The harnesses differ per platform but share one shape. A property is a typed call. A machine
run is a sequence executed from a fresh state that returns one record per step: on the EVM
through transactions against storage, in one native process reading standard input, and in
one WebAssembly call that reads the sequence from a host file.

## Proving

`femp prove` is described for users in [proving.md](proving.md). One run:

1. **Claims** come from the same discovery as property testing (`femp-spec`).
2. **Lowering** (`femp-frontend`) loads each module once per platform with the platform's Fe
   target, finds each claim's functions, and lowers every function they reach from the
   compiler's normalized semantic IR: SSA, monomorphized, typed, with source positions. Views
   and shared borrows become the values they read. `extern` functions of the core library have
   models (`intrinsics.rs`); calls into `femp_spec` become specification statements.
3. **Tasks** (`femp-prove/src/obligation`): each claim becomes one or more procedures of
   `assume` and `assert` around calls: a property's call and result; a machine's initializer, one
   induction step per command, and the bounded search; a spec's arbitrary arguments; two
   targets' programs side by side for a differential. Targets whose tasks print the same,
   ignoring source positions, share one task.
4. **Passes** (`passes`, in order): `scope` keeps what the task reaches · `recursion` rejects
   cycles · `spec_shape` gives a spec under proof its meaning · `summaries` replaces calls to
   specified functions with their specs · `inline` · `slots` turns local storage into SSA ·
   `checks` makes every panic an assertion tagged with its `PanicKind` · `loops` cuts loops at
   their invariants or unrolls them · `simplify`. Blocks carry a mode, so that what an assumed
   invariant or a proved spec rules out is assumed rather than checked again.
5. **Verification conditions** (`vc`): block reachability over the acyclic procedure. Integers
   are SMT integers with range facts, wrapping by `mod`, truncating division, bitwise operations
   through bit vectors; structs, enums and tuples are datatypes; arrays are SMT arrays; a
   result a spec leaves open is an uninterpreted function of the arguments.
6. **Solver** (`solver`): one `z3 -in -smt2` process per task, a fresh context per assertion
   (a context reused across queries is weaker on nonlinear arithmetic), a fixed seed and a
   per-query timeout. A satisfiable query's inputs are minimized by bisection on their distance
   from the simplest value, in property testing's order.
7. **Replay and report** (`femp-cli`): counterexamples run on the property harnesses, are judged
   as `femp prop` judges them, and are saved as regressions. The report renders as text or JSON.

## Toolchain

`femp-toolchain` resolves the compiler in this order: `FEMP_FE`, the managed toolchain for the
pinned version under `FEMP_HOME` (default `~/.femp`), then `fe` on `PATH`. The version must equal
the project's pin. Backends are read from the compiler's own help text, which lists exactly the
ones compiled in. Z3 is found the same way Node.js is: `FEMP_Z3`, then the copy under
`FEMP_HOME/z3`, then `z3` on `PATH`, skipping any that does not run.

`femp toolchain install` clones the release tag, installs the parser generator pinned by the
compiler's grammar crate (unless a `tree-sitter` CLI is already on `PATH`), and runs
`cargo install` with the native and WebAssembly features, choosing an installed Rust toolchain
that is new enough.

## Testing strategy

- Unit tests cover parsing, layering, the bundler, the test-output parser and the parity matrix.
- Generators are tested against files written by the real compiler
  (`crates/*/tests/fixtures`). Set `FEMP_UPDATE_FIXTURES=1` to rewrite the expected output.
- `runtime/wasm-host` has its own suite (`node --test runtime/wasm-host/*.test.js`): the host
  runtime against a module built from `test/probe.fe`, and the property runner against one built
  from `test/harness_probe.fe`.
- `crates/femp-cli/tests` drives the `femp` binary against the demonstrator when the compiler is
  installed.
- `examples/vesting/e2e` runs one table of cases against the three built artifacts.
- `femp-frontend` snapshots the VIR of a fixture ingot for each target
  (`tests/fixtures/basics.*.vir`), and `femp-prove` snapshots tasks after every pass and an
  SMT-LIB script, then proves and refutes its fixture's claims with Z3.
  `crates/femp-cli/tests/proofs.rs` runs `femp prove` on the planted fixture and the
  demonstrator and checks verdicts, counterexamples, replays and regressions.
- `femp-prop` tests the engine against in-memory executors. Each harness renderer is tested
  against a committed fixture, and the revm executor runs committed harness bytecode.
  `crates/femp-cli/tests/properties.rs` runs `femp prop` on a fixture whose properties fail on
  purpose (`tests/fixtures/planted`) and checks the exact minimal counterexamples.
