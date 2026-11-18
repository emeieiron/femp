# FeMP

FeMP (Fe Multiplatform) builds one [Fe](https://fe-lang.org) codebase into EVM contracts,
native programs and WebAssembly modules, and makes the platforms' agreement something you
test and prove. Application logic lives once, in shared ingots. Thin platform modules expose
it as a contract, a command-line program and a JavaScript module. The same tests, generated
inputs and proofs then run against every compiled target, so a difference between platforms
surfaces as a failing check rather than a production incident.

## What you get

- **One project, three platforms.** A `femp.toml` names a shared module and one module per
  platform. `femp check` type-checks shared code once for every platform, and layering rules
  keep platform-only capabilities out of it. `femp build` writes bytecode and ABIs, native
  executables, and WebAssembly modules, with TypeScript and JavaScript bindings.
- **Tests that run everywhere.** `femp test` runs each shared test on the EVM, natively and
  in WebAssembly, prints the results side by side, and fails on any divergence.
- **Generated inputs.** `femp prop` runs `#[property]` and `#[differential]` functions,
  state machines and contract invariants with generated inputs on every target, shrinks each
  failure to a minimal counterexample, and replays it on every later run.
- **Proofs for all inputs.** `femp prove` proves the same claims, plus `#[spec]` functions and
  a contract's storage invariants, with the Z3 solver. A counterexample is run on the compiled
  targets before it is reported.
- **Generated source.** Modules declare source generators, such as a serializer generator,
  that run before every command through the [Fe Source Generator Protocol](source-generators.md).

## How it relates to Fe

FeMP builds on the official Fe compiler rather than replacing it. It links the compiler's own
crates, version 26.6.0, so `femp` checks, builds and tests with exactly what `fe` uses, and
its builds are byte-identical to `fe build`'s. It does not change the language: shared code is
ordinary Fe, platform differences are traits, effects and `#[cfg(target = ...)]`, and the
attributes FeMP reads are ones the compiler accepts and ignores.
[FeMP and the Fe toolchain](fe-and-femp.md) compares the two in detail.

## Where to go next

- [Install FeMP](installation.md), then follow the [quick start](quick-start.md).
- [Projects and modules](project-layout.md) explains the manifest and how to share code.
- [Testing](testing.md), [property testing](property-testing.md) and [proving](proving.md)
  cover verification.
- The [CLI reference](cli.md) and [attributes reference](attributes.md) list every option.
