# Quick start

This walks through a new project from creation to a contract, a command-line program and a
JavaScript module that compute the same fee, with the fee's promise proved for every input.

## Create a project

```bash
femp new my-app && cd my-app
```

```text
my-app/
  femp.toml     the modules, their platforms and the compiler version
  shared/       portable logic and its tests          (no target)
  evm/          a contract that exposes it on chain   (target = "evm")
  native/       a command-line program                (target = "native")
  wasm/         functions exported to JavaScript      (target = "wasm")
  spec/         the femp_spec library that #[spec] functions use
```

The shared module holds the logic once. Here it computes a fee in basis points:

```fe
use femp_spec::ensures

pub const BASIS_POINTS: u64 = 10000

/// The fee on `amount` at a rate in basis points, rounded down. Rates above
/// 100% are treated as 100%.
pub fn fee(amount: u64, rate: u64) -> u64 {
    let capped = if rate > BASIS_POINTS {
        BASIS_POINTS
    } else {
        rate
    }
    // Widened so the product cannot overflow; the result is at most `amount`.
    let scaled = (amount as u256) * (capped as u256) / (BASIS_POINTS as u256)
    scaled.downcast_unchecked()
}

/// What `fee` promises for every input: it never charges more than the amount.
#[spec]
pub fn fee_spec(amount: u64, rate: u64) -> u64 {
    let charged = fee(amount, rate)
    ensures(charged <= amount)
    charged
}

#[test]
fn charges_the_rate() {
    assert(fee(amount: 20000, rate: 250) == 500)
}
```

Each platform module is a thin layer over it. The contract answers a message, the program reads
its arguments, and the WebAssembly module exports a function:

```fe
// evm/src/lib.fe
msg FeesMsg {
    #[selector = sol("fee(uint64,uint64)")]
    Fee { amount: u64, rate: u64 } -> u64,
}

pub contract Fees {
    recv FeesMsg {
        Fee { amount, rate } -> u64 {
            fee(amount, rate)
        }
    }
}
```

```fe
// wasm/src/lib.fe
/// Exported to JavaScript.
#[unsafe(export)]
pub fn fee(amount: u64, rate: u64) -> u64 {
    my_app_core::fee(amount, rate)
}
```

## Test on every platform

```bash
femp test
```

```text
    Tested core [evm]: 3 passed, 0 failed
    Tested core [native]: 3 passed, 0 failed
    Tested core [wasm]: 3 passed, 0 failed
    Tested contracts [evm]: 1 passed, 0 failed

Shared tests across platforms

                            evm     native  wasm
    core::caps_the_rate     pass    pass    pass
    core::charges_the_rate  pass    pass    pass
    core::rounds_down       pass    pass    pass

    Parity every shared test behaves the same on all platforms it ran on
```

Every shared test ran three times, compiled for each platform. Had one passed on the EVM and
failed in WebAssembly, the command would report the divergence and fail.

## Check with generated inputs

```bash
femp prop
```

```text
PASS  core::fee_spec  256 cases on evm, native, wasm

1 passed, 0 failed (seed 0x000000000000002a, normalized comparison)
```

A public spec is also an entry point: `femp prop` called `fee` with 256 generated inputs,
biased toward boundary values, and checked what the spec ensures. Add `#[property]` and
`#[differential]` functions to test more; see [property testing](property-testing.md).

## Prove it for every input

```bash
femp prove
```

```text
PROVED      core::fee_spec  on evm, native, wasm

1 proved, 0 refuted, 0 unknown (evm, native, wasm)
```

The solver established that `fee` never charges more than the amount, for all 2¹²⁸ input
pairs and on each platform's program, and that it never panics. A false claim would come back
with a minimal counterexample, run on the compiled targets.

## Build and use it

```bash
femp build
```

```text
     Built contracts [evm]
    bytecode         build/evm/contracts/Fees.bin
    runtime-bytecode build/evm/contracts/Fees.runtime.bin
    abi              build/evm/contracts/Fees.abi.json
    metadata         build/evm/contracts/Fees.metadata.json
    binding          build/evm/contracts/Fees.ts
     Built cli [native]
    executable       build/native/cli/my_app_cli
     Built web [wasm]
    wasm-module      build/wasm/web/my_app_web.wasm
    wasm-manifest    build/wasm/web/my_app_web.manifest.json
    binding          build/wasm/web/my_app_web.js
    binding          build/wasm/web/my_app_web.d.ts
    runtime          build/wasm/web/host.js
    runtime          build/wasm/web/host.d.ts
    package-manifest build/wasm/web/package.json
```

```bash
./build/native/cli/my_app_cli 20000 250
```

```text
500
```

```js
import { MyAppWeb } from "./build/wasm/web/my_app_web.js";

const app = await MyAppWeb.load();
app.fee(20000n, 250n); // 500n
```

`Fees.ts` exports the contract's ABI `as const` and its bytecode, for viem, ethers or any
other library; see [build outputs and bindings](bindings.md). `femp package` collects
everything under `dist/` with a manifest of SHA-256 hashes.

## Next

- [Projects and modules](project-layout.md): the manifest, layering rules, and sharing code
  through ports and adapters.
- [Property testing](property-testing.md) and [differential testing](differential-testing.md).
- [Contracts](contracts.md): storage invariants and message claims, tested and proved.
- [Proving](proving.md): specs, machines, loops and verdicts.
