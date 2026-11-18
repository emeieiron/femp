# Command line

```text
femp <COMMAND> [OPTIONS]
```

Every command except `new` and `doctor` works on the project whose `femp.toml` is in the current
directory or the nearest one above it. Commands that compile first run the project's
[source generators](source-generators.md).

| Command | Does |
| --- | --- |
| [`femp new`](#femp-new) | Create a project |
| [`femp doctor`](#femp-doctor) | Report the linked compiler and what this machine can build |
| [`femp generate`](#femp-generate) | Write generated sources into the source tree |
| [`femp check`](#femp-check) | Type-check every module for its platforms |
| [`femp build`](#femp-build) | Compile platform modules and generate bindings |
| [`femp test`](#femp-test) | Run tests on every platform and compare them |
| [`femp prop`](#femp-prop) | Test properties with generated inputs on every target |
| [`femp prove`](#femp-prove) | Prove claims for all inputs |
| [`femp package`](#femp-package) | Build, then collect packages under `dist/` |
| [`femp clean`](#femp-clean) | Remove `build/` and `dist/` |

`femp --version` prints femp's version, and `femp <command> --help` a command's options.

## Choosing platforms

Every command that compiles takes `-t, --target <PLATFORM>`, repeated for several. Without it,
a command uses every platform the project targets, and skips one this machine cannot build
with a warning that says why. A platform named with `-t` that is unavailable is an error.
`-t all` is the same as leaving it out.

```bash
femp test -t evm -t wasm
```

Platforms are `evm`, `native` and `wasm`.

## Optimization levels

`-O, --optimize` takes `0`, `1`, `2` or `s` (size); the default is `1`. `femp build` and
`femp package` take one level. `femp prop` takes a comma-separated list and runs each level
as a target of its own, such as `evm-O0` and `evm-O2`.

## femp new

```text
femp new <NAME>
```

Creates the directory `<NAME>` with a shared module, one module per platform, and the
`femp_spec` library, then prints the next commands. See the [quick start](quick-start.md).

## femp doctor

Prints the linked compiler and its backends, the host's architecture and operating system,
the Node.js, C compiler and Z3 femp would use, and each platform's availability with a fix
when one is unavailable. Inside a project it also checks the layout, the compiler pin, and
that every source generator's executable exists.

## femp generate

Runs every source generator and writes its output to the declared path in the source tree,
for editors, code navigation and plain `fe` commands. Other commands run generators
themselves and do not need this. Keep generated paths out of version control.

## femp check

```text
femp check [-t <PLATFORM>]...
```

Type-checks every platform module for its platform, and every shared module once for each
platform. Every module that fails is reported with the compiler's diagnostics.

## femp build

```text
femp build [-t <PLATFORM>]... [-O <LEVEL>]
```

Compiles every platform module and writes its artifacts and bindings under
`build/<platform>/<module>/`, printing each file with its kind. See
[build outputs and bindings](bindings.md).

## femp test

```text
femp test [-t <PLATFORM>]... [-f <PATTERN>] [--verbose]
```

| Option | Effect |
| --- | --- |
| `-f, --filter <PATTERN>` | Only tests whose name contains the pattern. |
| `--verbose` | Print the compiler's output for every suite, not only failing ones. |

Fails when a test fails, a suite cannot run, or a shared test diverges. See
[testing](testing.md).

## femp prop

```text
femp prop [-t <PLATFORM>]... [-f <PATTERN>] [-O <LEVELS>] [OPTIONS]
```

| Option | Default | Effect |
| --- | --- | --- |
| `-f, --filter <PATTERN>` | | Only entry points whose name contains the pattern. |
| `-O, --optimize <LEVELS>` | `1` | Levels to compile every platform at, comma separated. |
| `--cases <N>` | from the attributes | Cases, or machine runs, per entry point. |
| `--seed <SEED>` | random, printed | Seed, decimal or `0x` hex. Also `FEMP_PROP_SEED`. |
| `--exhaustive` | | Enumerate every input of entry points with at most 2²⁰ inputs. |
| `--no-shrink` | | Report failures as found, without minimizing them. |
| `--max-shrinks <N>` | `2000` | Most replays spent minimizing one failure. |
| `--compare <MODE>` | `normalized` | How targets must agree: `normalized`, `strict` or `lenient`. |
| `--no-regressions` | | Neither replay nor store regressions. |
| `--timeout <SECONDS>` | `10` | Longest a single case may run before it counts as a timeout. |
| `--list` | | List the entry points instead of running them. |
| `--json` | | Also write the report to `build/prop/report.json`. |

Fails when an entry point fails or diverges, or cannot be run. See
[property testing](property-testing.md) and [differential testing](differential-testing.md).

## femp prove

```text
femp prove [-t <PLATFORM>]... [-f <PATTERN>] [OPTIONS]
```

| Option | Default | Effect |
| --- | --- | --- |
| `-f, --filter <PATTERN>` | | Only claims whose name contains the pattern. |
| `--timeout <SECONDS>` | `20` | Seconds the solver may spend on each assertion. |
| `--unroll <N>` | `8` | Iterations a loop without an `invariant(..)` is unrolled for. |
| `--steps <K>` | `6` | Longest sequence the search for a machine or contract counterexample tries. |
| `--jobs <N>` | CPUs | Tasks proved at once, each with its own solver. |
| `--no-replay` | | Report counterexamples without running them on the compiled targets. |
| `--list` | | List the claims instead of proving them. |
| `--json` | | Also write the report to `build/prove/report.json`. |
| `--dump-vir` | | Write each task's intermediate form after every pass under `build/prove/vir`. |
| `--keep-smt` | | Keep each task's SMT-LIB script under `build/prove/smt`. |
| `--strict` | | Also fail on claims the prover does not support. |

Fails when a claim is refuted or unknown, and with `--strict` when one is not supported. See
[proving](proving.md).

## femp package

```text
femp package [-t <PLATFORM>]... [-O <LEVEL>]
```

Builds, then copies every platform's outputs under `dist/<platform>/` and writes
`dist/manifest.json` with each file's kind and SHA-256. Native executables go under
`dist/native/<arch>-<os>/`.

## femp clean

Removes `build/` and `dist/`. Stored regressions under `regressions/` are kept.

## Environment

| Variable | Used by | Effect |
| --- | --- | --- |
| `FEMP_HOME` | femp, installer | Where femp is installed and looks for its Node.js and Z3. Default `~/.femp`. |
| `FEMP_NODE` | femp | The Node.js that runs WebAssembly. Otherwise a Node.js 20 or newer on `PATH`, then the one under `FEMP_HOME/node`. |
| `FEMP_Z3` | femp | The Z3 `femp prove` drives. Otherwise `FEMP_HOME/z3/bin/z3`, then `z3` on `PATH`. |
| `FEMP_PROP_SEED` | `femp prop` | The seed, as `--seed`. |
| `FEMP_VERSION` | installer | The release tag to install, such as `v0.1.0`. Default the latest. |
| `FEMP_DOWNLOAD_URL` | installer | Base URL of the release files, for mirrors. |
| `FEMP_NODE_MIRROR` | installer | Base URL of Node.js releases. Default `https://nodejs.org/dist`. |

## Exit status

Every command exits with `0` on success and `1` when it fails, after printing why. Errors
are printed as `error` lines, warnings as `warning` lines.
