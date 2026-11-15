# Fe Multiplatform

Fe Multiplatform (FeMP) builds one [Fe](https://fe-lang.org) codebase for several execution
environments. You write application logic once in a shared module and consume it from thin
platform modules; the `femp` command-line tool turns the project into:

| Platform | Output | Bindings |
| --- | --- | --- |
| `evm` | Deployable bytecode, runtime bytecode, ABI, contract metadata | TypeScript module with the typed ABI |
| `native` | A command-line executable for the host | – |
| `wasm` | A WebAssembly module and its build manifest | JavaScript wrapper, TypeScript declarations, host runtime |

FeMP sits on top of the Fe compiler. It does not change the language: shared code is ordinary
Fe ingots, platform differences are expressed with traits, effects and `#[cfg(target = ...)]`,
and every build step is a call to `fe`.

## Requirements

- Rust 1.88 or newer to build `femp`; Rust 1.97 or newer to build the Fe compiler.
- Node.js 20 or newer (the compiler's parser generator, WebAssembly tests, and the bindings).
- A C compiler (`cc`) to link native executables.
- Fe **26.6.0**, installed by `femp toolchain install`.

The native and WebAssembly backends of Fe 26.6 are experimental. `femp doctor` reports them as
such and tells you when one is unavailable on your machine.

## Getting started

```sh
cargo install --path crates/femp-cli   # installs the `femp` binary
femp toolchain install                 # builds Fe 26.6.0 with all three backends (a few minutes)
femp new my-app && cd my-app
femp doctor
femp test
femp prop
femp build
```

`femp toolchain install` compiles Fe from source because released Fe 26.6.0 binaries do not
include the WebAssembly backend. The compiler is kept under `~/.femp/toolchains/` (override with
`FEMP_HOME`). To use a compiler you built yourself, point `FEMP_FE` at it.

## Commands

| Command | What it does |
| --- | --- |
| `femp new <name>` | Creates a project with a shared module and one module per platform. |
| `femp doctor` | Shows the compiler, its backends, host tools, and which platforms are available. |
| `femp generate` | Materializes generated sources for editing and code navigation. |
| `femp toolchain install` | Builds the pinned Fe compiler with the backends FeMP drives. |
| `femp check [-t <platform>]` | Type-checks platform modules, and every shared module once per platform. |
| `femp build [-t <platform>] [-O <level>]` | Compiles platform modules into `build/` and generates bindings. |
| `femp test [-t <platform>] [-f <pattern>]` | Runs shared tests on every platform and compares the outcomes. |
| `femp prop [-t <platform>] [-f <pattern>] [--seed <n>]` | Runs properties, differentials and state machines with generated inputs on every platform, and minimizes failures. |
| `femp package` | Builds, then collects distributable packages and a hash index under `dist/`. |
| `femp clean` | Removes `build/` and `dist/`. |

Without `-t`, a command covers every platform the project targets and skips, with a warning,
those that are unavailable. Naming a platform makes it mandatory.

## A project

```
my-app/
  femp.toml     modules, platforms, compiler version
  shared/       portable logic and its tests (an ingot with no target)
  evm/          contracts                    (target = "evm")
  native/       command-line program         (target = "native")
  wasm/         functions exported to JS     (target = "wasm")
```

See [docs/project-layout.md](docs/project-layout.md) for the manifest and the rules that keep
shared code portable, and [examples/vesting](examples/vesting) for a complete project: a token
vesting library used by a contract, a CLI and a browser page.

Modules can declare source generators that run before checking, building, testing, and
packaging. FeMP includes the generated sources in the staged ingots for every target.
[Configure module generators](docs/project-layout.md#generated-source)

## Testing across platforms

`femp test` runs each shared module's tests on every platform and prints them side by side:

```
                                          evm     native  wasm
core::nothing_vests_before_the_cliff      pass    pass    pass
core::invalid_claims_are_rejected         pass    pass    pass
```

A test that passes on one platform and fails on another is reported as a divergence, and the
command exits with a failure.

## Property testing

Fe Property Testing checks properties with generated inputs. You mark Fe functions and structs
in the shared code, and `femp prop` runs them on the EVM (in an embedded EVM), as a native
executable and as WebAssembly, then compares the results:

```fe
#[property]
#[gen(cliff = "0..=10_000", duration = "1..=10_000")]
pub fn vested_never_exceeds_total(total: u64, start: u64, cliff: u64, duration: u64, now: u64) -> bool {
    match Schedule::new(total, start, cliff, duration) {
        Result::Ok(schedule) => schedule.vested_at(now) <= total,
        Result::Err(_) => true,
    }
}
```

- **`#[property]`** must hold on every platform.
- **`#[differential]`** must give the same value, error or panic on every platform.
- **`#[machine]`** generates sequences of commands and checks invariants after each step.
  Declared in an `evm` module, a machine can deploy and drive the project's contracts.

Each failure is shrunk to a minimal counterexample. It is reported with each platform's outcome
and the seed that reproduces it, and saved under `regressions/`, which later runs replay first:

```
FAIL  core::rounding  diverges across targets after 2 cases
      x = 1000
      evm     returned 1000
      native  returned 1000
      wasm    returned 999
      seed 0x0000000000000007, case 1, minimized in 78 runs
      saved to regressions/core/rounding.json
```

See [docs/property-testing.md](docs/property-testing.md) for generators, machines, comparison
modes and options.

## Repository

| Path | Contents |
| --- | --- |
| `crates/femp-core` | Project model, layering rules, and the `Platform` trait. |
| `crates/femp-toolchain` | Locating, installing and invoking the Fe compiler. |
| `crates/femp-evm`, `femp-native`, `femp-wasm` | One adapter per platform. |
| `crates/femp-testing` | Cross-platform test runs and the parity matrix. |
| `crates/femp-prop` | Property testing: discovery, generators, the engine, shrinking, regressions, reports. |
| `crates/femp-harness` | Line-protocol sessions with harnesses that run as processes. |
| `crates/femp-cli` | The `femp` binary and the templates for `femp new`. |
| `runtime/wasm-host` | The JavaScript host runtime shipped with WebAssembly bindings, and the property-harness runner. |
| `examples/vesting` | The demonstrator project. |
| `docs/` | [Architecture](docs/architecture.md), [project layout](docs/project-layout.md), [property testing](docs/property-testing.md), [adding a platform](docs/adding-a-platform.md). |

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node --test runtime/wasm-host/*.test.js
```

Tests that need the Fe compiler skip themselves when it is not installed.
