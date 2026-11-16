# Fe Multiplatform

Fe Multiplatform (FeMP) builds one [Fe](https://fe-lang.org) codebase for several execution
environments. You write application logic once in a shared module and consume it from thin
platform modules; the `femp` command-line tool turns the project into:

| Platform | Output | Bindings |
| --- | --- | --- |
| `evm` | Deployable bytecode, runtime bytecode, ABI, contract metadata | TypeScript module with the typed ABI |
| `native` | A command-line executable for the host | – |
| `wasm` | A WebAssembly module and its build manifest | JavaScript wrapper, TypeScript declarations, host runtime |

FeMP sits on top of the Fe compiler, which it links as a library: `femp` checks, builds, tests
and proves with the compiler's own crates, so there is no separate `fe` to install. It does not
change the language: shared code is ordinary Fe ingots, and platform differences are expressed
with traits, effects and `#[cfg(target = ...)]`.

## Installing

```sh
curl -fsSL https://github.com/emeieiron/femp/releases/latest/download/installer.sh | sh
```

The installer works on Linux (x86-64 and ARM, glibc) and macOS (Apple silicon and Intel). It
installs into `~/.femp` (or `FEMP_HOME`) and adds `~/.femp/bin` to `PATH` in your shell's
startup files. It sets up:

- **femp**, with the Fe 26.6.0 compiler and its EVM, native and WebAssembly backends built in.
- **Z3**, the solver `femp prove` uses, in `~/.femp/z3`. Its Linux builds need glibc 2.38 or
  newer; on older systems femp uses a `z3` from the system's package manager instead.
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
| `--skip-deps` | Install femp and Z3 only. |

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
femp prove
femp build
```

### Building from source

Building from source needs Rust 1.97 or newer, and to run what femp builds, Node.js 20 or
newer, a C compiler, and Z3 for `femp prove`. Cargo fetches the Fe compiler's crates from the
Fe repository at the pinned release, so the first build takes a few minutes longer.

```sh
cargo install --path crates/femp-cli   # installs `femp`, with the compiler built in
```

To use a particular Node.js, point `FEMP_NODE` at it, and a particular Z3, `FEMP_Z3`.

A project pins the compiler release in `femp.toml` (`[toolchain] fe = "26.6.0"`). Each femp
build links one release, and refuses a project pinned to another.

## Commands

| Command | What it does |
| --- | --- |
| `femp new <name>` | Creates a project with a shared module and one module per platform. |
| `femp doctor` | Shows the linked compiler, its backends, host tools, and which platforms are available. |
| `femp generate` | Materializes generated sources for editing and code navigation. |
| `femp check [-t <platform>]` | Type-checks platform modules, and every shared module once per platform. |
| `femp build [-t <platform>] [-O <level>]` | Compiles platform modules into `build/` and generates bindings. |
| `femp test [-t <platform>] [-f <pattern>]` | Runs shared tests on every platform and compares the outcomes. |
| `femp prop [-t <platform>] [-f <pattern>] [--seed <n>]` | Runs properties, differentials and state machines with generated inputs on every platform, and minimizes failures. |
| `femp prove [-t <platform>] [-f <pattern>]` | Proves the same claims, `#[spec]` functions and contract `#[invariant]`s for all inputs, and replays counterexamples on every platform. |
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

## Proving

`femp prove` proves the same declarations for all inputs with the Z3 solver, and `#[spec]`
functions state what a function promises:

```fe
#[spec(transparent, target = "Schedule::vested_at")]
fn vested_at_spec(schedule: Schedule, now: u64) -> u64 {
    requires(schedule.cliff <= schedule.duration)
    let vested = schedule.vested_at(now)
    ensures(vested <= schedule.total)
    vested
}
```

A property is proved when it returns `true` for every input in its domains without panicking,
a machine by induction over its commands, and a differential when every target computes the
same outcome. A counterexample is minimized, run on every platform, and saved under
`regressions/`, so `femp prop` keeps replaying it:

```
PROVED      core::vested_never_exceeds_total  on evm, native, wasm
REFUTED     core::halves_stay_small           returns false
            x = 250
            evm returned false · native returned false · wasm returned false
            saved to regressions/core/halves_stay_small.json
```

A contract's `#[invariant]`s are proved for every sequence of messages it can receive. A
broken one comes back as a sequence from deployment, which is run on the compiled contract:

```fe
#[invariant]
fn claims_stay_within_total(store: VestingStore) -> bool {
    store.claimed <= store.total
}
```

```
REFUTED     contracts::Vesting  invariant `claims_stay_within_total` fails after step 1
            init(total = 1, start = 0, cliff = 0, duration = 1)
            1. Claim(ctx.timestamp = 1, amount = 1)
            evm accepts every transaction
```

See [docs/proving.md](docs/proving.md) for specs, contracts, loops, verdicts and limits.

## Repository

| Path | Contents |
| --- | --- |
| `crates/femp-core` | Project model, layering rules, and the `Platform` trait. |
| `crates/femp-compiler` | The Fe compiler, linked as a library: checks, builds, tests, and lowering into the prover's IR. |
| `crates/femp-host` | Finding Node.js, a C compiler and Z3 on the machine. |
| `crates/femp-evm`, `femp-native`, `femp-wasm` | One adapter per platform. |
| `crates/femp-testing` | Cross-platform test runs and the parity matrix. |
| `crates/femp-spec` | The claims both tools read: discovery, `#[gen]` domains, counterexamples. |
| `crates/femp-prop` | Property testing: generators, the engine, shrinking, regressions, reports. |
| `crates/femp-vir` | The verification IR. |
| `crates/femp-prove` | Proving: tasks, passes, verification conditions, the solver, verdicts, reports. |
| `crates/femp-harness` | Line-protocol sessions with harnesses that run as processes. |
| `crates/femp-cli` | The `femp` binary and the templates for `femp new`. |
| `runtime/wasm-host` | The JavaScript host runtime shipped with WebAssembly bindings, and the property-harness runner. |
| `ingots/spec` | The `femp_spec` library of `#[spec]` statements, which `femp new` copies into projects. |
| `examples/vesting` | The demonstrator project. |
| `install/` | The installer and the script that builds release archives (`.github/workflows/release.yml` runs it). |
| `docs/` | [Architecture](docs/architecture.md), [project layout](docs/project-layout.md), [property testing](docs/property-testing.md), [proving](docs/proving.md), [adding a platform](docs/adding-a-platform.md). |

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node --test runtime/wasm-host/*.test.js
```

Tests that need the Fe compiler or Z3 skip themselves when they are not installed.

## Releasing

Pushing a tag such as `v0.1.0` runs `.github/workflows/release.yml`. The workflow:

1. builds an archive per host with `install/build-archive.sh`, containing femp, the Fe
   compiler and a pinned Z3 build;
2. installs each archive with the installer as a smoke test;
3. publishes the archives, `installer.sh` and `SHA256SUMS` as a GitHub release.

The installer URL always points at the latest release.

## License

femp is licensed under the [Apache License, Version 2.0](LICENSE). Release archives also
contain the Fe compiler, which is licensed under the Apache License, Version 2.0 as well
(`LICENSE-fe`), and Z3, which is licensed under the MIT License (`LICENSE-z3`).
