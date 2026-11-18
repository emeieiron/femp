# FeMP

FeMP generates inputs, executes the same Fe property across targets, finds behavioral
divergences, minimizes counterexamples, and preserves them as regressions.

## The problem

Fe supports multiple compilation targets, but building an application across them involves
more than compiling the same code.

Developers need to organize shared logic, manage platform-specific implementations, and
establish that their programs behave consistently across environments.

A program that works on the EVM should not silently produce different results when compiled
natively or to WebAssembly.

## The approach

FeMP brings these concerns into one project.

- **Share logic.** Write reusable Fe code once and expose it through platform-specific modules.
- **Build everywhere.** Produce EVM contracts, native executables, and WebAssembly modules with
  generated bindings.
- **Test behavior.** Run shared tests and generated inputs across platforms, detecting
  inconsistencies and minimizing counterexamples.
- **Prove properties.** Use formal verification to establish correctness claims beyond
  individual test cases.

FeMP builds on the official Fe compiler. It does not replace the language or require a custom
dialect.

[Documentation](https://emeieiron.github.io/femp/) · [Quick start](docs/quick-start.md) ·
[FeMP and Fe](docs/fe-and-femp.md) · [Example project](examples/vesting)

## Install

```sh
curl -fsSL https://github.com/emeieiron/femp/releases/latest/download/installer.sh | sh
```

## Create a project

```sh
femp new my-app && cd my-app
```

```text
shared/   the logic, its tests and specs
spec/     the femp_spec library that #[spec] functions use
evm/      a contract that exposes it
native/   a command-line program
wasm/     functions exported to JavaScript
```

## Test on every platform

```sh
femp test
```

```text
                            evm     native  wasm
    core::caps_the_rate     pass    pass    pass
    core::charges_the_rate  pass    pass    pass
    core::rounds_down       pass    pass    pass
```

A shared test that passes on one platform and fails on another fails the run.
[Testing](docs/testing.md)

## Test with generated inputs

Add a property to `shared/src/lib.fe`:

```fe
#[property]
pub fn fee_never_exceeds_the_amount(amount: u64, rate: u64) -> bool {
    fee(amount, rate) <= amount
}
```

```sh
femp prop
```

```text
PASS  core::fee_never_exceeds_the_amount  256 cases on evm, native, wasm
PASS  core::fee_spec                      256 cases on evm, native
      not run on wasm: a WebAssembly module traps alike when a spec rejects a case and when it fails
```

A failure is shrunk to a minimal counterexample, saved under `regressions/` and replayed on
every run. [Property testing](docs/property-testing.md) ·
[Differential testing](docs/differential-testing.md)

## Prove it for every input

The template already states what `fee` promises:

```fe
#[spec]
pub fn fee_spec(amount: u64, rate: u64) -> u64 {
    let charged = fee(amount, rate)
    ensures(charged <= amount)
    charged
}
```

```sh
femp prove
```

```text
PROVED      core::fee_never_exceeds_the_amount  on evm, native, wasm
PROVED      core::fee_spec                      on evm, native, wasm
```

[Proving](docs/proving.md) · [Contracts](docs/contracts.md)

## Build and use it

```sh
femp build
./build/native/cli/my_app_cli 20000 250   # 500
```

```js
import { MyAppWeb } from "./build/wasm/web/my_app_web.js";

const app = await MyAppWeb.load();
app.fee(20000n, 250n); // 500n
```

Contracts get their bytecode, ABI and a typed TypeScript module. `femp package` collects every
output under `dist/` with SHA-256 hashes.
[Outputs and bindings](docs/bindings.md) · [Command line](docs/cli.md)

## Status

- `evm` is stable. `native` and `wasm` use Fe 26.6's experimental backends; native executables
  build on x86-64 Linux and Apple silicon.
- Code the prover cannot model yet, such as strings, recursion and machines that deploy
  contracts, is reported as not supported. See [limits](docs/proving.md#limits).

## Develop

```sh
cargo install --path crates/femp-cli   # Rust 1.97 or newer
cargo test --workspace
```

[Architecture](docs/architecture.md) · [Adding a platform](docs/adding-a-platform.md)

## License

[Apache License 2.0](LICENSE). Release archives also include the Fe compiler (Apache 2.0)
and Z3 (MIT).
