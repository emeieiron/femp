# Testing

`femp test` runs the project's `#[test]` functions. Tests in a shared module run once for every
platform the project targets, each time compiled for that platform, and their results are
compared. Tests in a platform module run on that platform.

```bash
femp test
```

```text
    Tested core [evm]: 3 passed, 0 failed
    Tested core [native]: 3 passed, 0 failed
    Tested core [wasm]: 3 passed, 0 failed
    Tested contracts [evm]: 1 passed, 0 failed
    Tested cli [native]: no tests
    Tested web [wasm]: no tests

Shared tests across platforms

                            evm     native  wasm
    core::caps_the_rate     pass    pass    pass
    core::charges_the_rate  pass    pass    pass
    core::rounds_down       pass    pass    pass

    Parity every shared test behaves the same on all platforms it ran on
```

## How each platform runs a test

| Platform | Runs in |
| --- | --- |
| `evm` | The compiler's test runner, on its in-process EVM, as `fe test` does. |
| `native` | A native executable per suite, as `fe test --backend native` does. |
| `wasm` | Node.js, through the same host runtime as the generated JavaScript bindings. |

A test is an ordinary Fe function, so a shared test needs nothing platform-specific:

```fe
#[test]
fn caps_the_rate() {
    assert(fee(amount: 1000, rate: 20000) == 1000)
}
```

## Divergences

A shared test that passes on one platform and fails on another is a divergence. The command
names it, prints the failing suite's output, and fails:

```fe
#[test]
fn counts_past_four_billion() {
    let n: usize = 4_000_000_000
    assert(n + n > n)
}
```

```text
    Tested core [evm]: 4 passed, 0 failed
    Tested core [native]: 4 passed, 0 failed
    Tested core [wasm]: 3 passed, 1 failed

Shared tests across platforms

                                    evm     native  wasm
    core::caps_the_rate             pass    pass    pass
    core::charges_the_rate          pass    pass    pass
    core::counts_past_four_billion  pass    pass    FAIL
    core::rounds_down               pass    pass    pass

     error divergence: core::counts_past_four_billion passes on evm, native but fails on wasm

core [wasm]

    FAIL  [0.0309s] counts_past_four_billion
        wasm test failed: trap: unreachable (Fe code traps on a failed assertion, arithmetic overflow, division by zero and an index outside an array)
```

`usize` is 256 bits on the EVM, 64 natively and 32 in WebAssembly, so the addition overflows
only in WebAssembly. Run on one target, the test would have passed. Common sources of
divergence:

- **Width-dependent types.** `usize` and `isize` differ per platform. Use fixed-width integers
  for values that cross a platform boundary or are stored.
- **Conditional compilation.** `#[cfg(target = ..)]` variants that disagree on some input.
- **Reverts.** A revert cannot be asserted on in WebAssembly, where a failure is a trap
  without a reason, and `Result::unwrap` needs an error type that encodes as revert data.

To search for divergences beyond the inputs your tests name, write a
[differential](differential-testing.md).

## Writing portable tests

- **Assert on returned values**, including `Result` errors, rather than on reverts.
- **Bind ports to fixed adapters.** A shared function that takes a `Clock` effect gets a fixed
  clock in its tests, so time-dependent logic is deterministic on every platform. See
  [ports and adapters](project-layout.md#ports-and-adapters).
- **Keep platform-only tests in platform modules**, or mark them `#[cfg(target = ..)]`. A
  shared test compiled for only some platforms shows as `-` elsewhere and is reported in a
  warning, not as a failure.

## Contract tests

Tests in an `evm` module can deploy and call the module's contracts through the `Evm` effect:

```fe
use std::evm::Evm

#[test]
fn quotes_fees_on_chain() uses (evm: mut Evm) {
    let fees = evm.create2<Fees>(value: Wei::zero(), args: (), salt: 0)
    let quoted: u64 = evm.call(
        addr: fees,
        gas: 100000,
        value: Wei::zero(),
        message: FeesMsg::Fee { amount: 20000, rate: 250 },
    )
    assert(quoted == 500)
}
```

They appear as `Tested contracts [evm]`. For claims about every sequence of messages rather
than one, see [contracts](contracts.md).

## Options

| Option | Effect |
| --- | --- |
| `-t, --target <platform>` | Run on these platforms only. Repeat it for several. |
| `-f, --filter <pattern>` | Only tests whose name contains the pattern. |
| `--verbose` | Print the compiler's output for every suite, not only failing ones. |

The command fails when any test fails, a suite cannot be built or run, or a shared test
diverges.
