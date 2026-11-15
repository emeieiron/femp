# Architecture

FeMP is a build orchestrator. It owns the project model, decides which compiler invocations a
command needs, and turns compiler output into artifacts, bindings and reports. Compilation
itself is always done by `fe`.

## Crates

```
femp-cli ──────────────┬──────────────┬─────────────┐
   │                   │              │             │
femp-testing     femp-evm      femp-native     femp-wasm
   │                   └──────┬───────┴─────────────┘
   │                          │
   │                   femp-toolchain
   │                          │
   └──────────────────── femp-core
```

| Crate | Responsibility | Depends on |
| --- | --- | --- |
| `femp-core` | `femp.toml` and `fe.toml` parsing, the module graph, layering rules, the `Platform` trait and registry, artifact and test-outcome types, a small source writer for generated code. No process spawning. | – |
| `femp-toolchain` | Finding the pinned compiler, building it from source, probing its backends and the host, running `fe check/build/test`, parsing test output. The only crate that knows the compiler's command line. | core |
| `femp-evm`, `femp-native`, `femp-wasm` | One `Platform` implementation each: availability, build, bindings, packaging. | core, toolchain |
| `femp-testing` | Planning which module runs on which platform, running the jobs, and the parity matrix. | core |
| `femp-cli` | Argument parsing, the session (project + registry + environment), commands, console output, project templates. `registry.rs` is the only file that names concrete platforms. | all |

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

## Toolchain

`femp-toolchain` resolves the compiler in this order: `FEMP_FE`, the managed toolchain for the
pinned version under `FEMP_HOME` (default `~/.femp`), then `fe` on `PATH`. The version must equal
the project's pin. Backends are read from the compiler's own help text, which lists exactly the
ones compiled in.

`femp toolchain install` clones the release tag, installs the parser generator pinned by the
compiler's grammar crate (unless a `tree-sitter` CLI is already on `PATH`), and runs
`cargo install` with the native and WebAssembly features, choosing an installed Rust toolchain
that is new enough.

## Testing strategy

- Unit tests cover parsing, layering, the bundler, the test-output parser and the parity matrix.
- Generators are tested against files written by the real compiler
  (`crates/*/tests/fixtures`). Set `FEMP_UPDATE_FIXTURES=1` to rewrite the expected output.
- `runtime/wasm-host` has its own suite (`node --test`) against a module built from
  `test/probe.fe`.
- `crates/femp-cli/tests` drives the `femp` binary against the demonstrator when the compiler is
  installed.
- `examples/vesting/e2e` runs one table of cases against the three built artifacts.
