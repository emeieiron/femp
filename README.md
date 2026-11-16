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

## Installing

```sh
curl -fsSL https://github.com/emeieiron/femp/releases/latest/download/installer.sh | sh
```

The installer works on Linux (x86-64 and ARM, glibc) and macOS (Apple silicon and Intel). It
installs into `~/.femp` (or `FEMP_HOME`) and adds `~/.femp/bin` to `PATH` in your shell's
startup files. It sets up:

- **femp.**
- **The Fe 26.6.0 compiler** built with the EVM, native and WebAssembly backends. Released Fe
  binaries lack the WebAssembly backend, so femp ships its own build.
- **Node.js**, for WebAssembly, when no Node.js 20 or newer is on `PATH`. It is kept in
  `~/.femp/node` for femp alone and does not change which `node` other programs see.
- **A C compiler**, for native executables, when none is found:
  - on Linux, `gcc` from the system package manager (`apt-get`, `dnf`, `yum`, `pacman` or
    `zypper`);
  - on macOS, Apple's Command Line Tools.

  The installer asks first, because this step may need your password.

Every download is checked against a SHA-256 list. Options go after `sh -s --`:

```sh
curl -fsSL https://github.com/emeieiron/femp/releases/latest/download/installer.sh | sh -s -- --yes
```

| Option | Effect |
| --- | --- |
| `--yes` | Install a missing C compiler without asking, for scripts and CI. |
| `--no-modify-path` | Leave shell startup files alone. |
| `--skip-deps` | Install femp and Fe only. |

`FEMP_VERSION=v0.1.0` installs a specific release. To uninstall, delete `~/.femp` and the line
the installer added to `~/.profile`, `~/.bashrc`, `~/.bash_profile` or `~/.zshrc`.

The native and WebAssembly backends of Fe 26.6 are experimental. `femp doctor` reports them as
such and tells you when one is unavailable on your machine. Native executables can be built on
x86-64 Linux and Apple silicon.

## Getting started

```sh
femp new my-app && cd my-app
femp doctor
femp test
femp prop
femp build
```

### Building from source

Building from source needs Rust 1.96 or newer for femp, Rust 1.97 or newer and npm for the Fe
compiler, Node.js 20 or newer, and a C compiler. femp links the Fe compiler's own crates for
`femp prove`; Cargo fetches them from the Fe repository at the pinned release, so the first
build takes a few minutes longer.

```sh
cargo install --path crates/femp-cli   # installs the `femp` binary
femp toolchain install                 # builds Fe 26.6.0 with all three backends (a few minutes)
```

`femp toolchain install` compiles Fe from source into `~/.femp/toolchains/` (override with
`FEMP_HOME`). To use a compiler you built yourself, point `FEMP_FE` at it; to use a particular
Node.js, point `FEMP_NODE` at it.

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
| `install/` | The installer and the script that builds release archives (`.github/workflows/release.yml` runs it). |
| `docs/` | [Architecture](docs/architecture.md), [project layout](docs/project-layout.md), [property testing](docs/property-testing.md), [adding a platform](docs/adding-a-platform.md). |

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node --test runtime/wasm-host/*.test.js
```

Tests that need the Fe compiler skip themselves when it is not installed.

## Releasing

Pushing a tag such as `v0.1.0` runs `.github/workflows/release.yml`. The workflow:

1. builds an archive per host with `install/build-archive.sh`, containing femp and the Fe
   compiler;
2. installs each archive with the installer as a smoke test;
3. publishes the archives, `installer.sh` and `SHA256SUMS` as a GitHub release.

The installer URL always points at the latest release.

## License

femp is licensed under the [Apache License, Version 2.0](LICENSE). Release archives also
contain the Fe compiler, which is licensed under the Apache License, Version 2.0 as well
(`LICENSE-fe`).
