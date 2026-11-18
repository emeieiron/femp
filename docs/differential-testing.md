# Differential testing

Shared code compiles to three programs, one per platform, and each optimization level gives
another. Differential testing checks that they compute the same thing: not only on the inputs
your tests name, but on generated inputs chosen to break integer code, and with `femp prove`,
on every input.

FeMP compares targets at three levels:

| | Compares | Inputs |
| --- | --- | --- |
| [`femp test`](testing.md) | Pass or fail of each shared `#[test]` on every platform | The ones each test names |
| `femp prop` | The outcome of every entry point on every target | Generated, shrunk, replayed |
| `femp prove` | The outcome of each `#[differential]` on every target | All of them |

## Declaring a differential

A `#[differential]` is a public function of primitive parameters that returns `bool` or a
fixed-width integer. It states no expectation of its own; it passes when every target
computes the same outcome:

```fe
#[differential]
pub fn rounding(x: u64) -> u64 {
    adjusted(x)
}
```

Here `adjusted` has a variant per platform, and the WebAssembly one rounds differently from
1000 on:

```fe
#[cfg(target = "wasm")]
fn adjusted(_ x: u64) -> u64 {
    if x >= 1000 { x - 1 } else { x }
}
```

```bash
femp prop -f rounding --seed 7
```

```text
FAIL  core::rounding  diverges across targets after 2 cases
      x = 1000
      evm     returned 1000
      native  returned 1000
      wasm    returned 999
      seed 0x0000000000000007, case 1, minimized in 78 runs
      saved to regressions/core/rounding.json
      reproduce: femp prop -f rounding --seed 0x0000000000000007
```

The failing case was shrunk to the boundary, `x = 1000`, and saved, so every later run of
`femp prop` replays it first. `femp prove` settles the same claim for all inputs:

```text
REFUTED     core::rounding  diverges: evm+native and wasm compute different outcomes
            x = 1000
            evm returned 1000 · native returned 1000 · wasm returned 999
            saved to regressions/core/rounding.json
```

Targets that lower to the same code agree by construction, so the prover compares only code
that differs between them, such as `#[cfg(target = ..)]` variants.

## Every entry point is a differential

`femp prop` compares targets for every entry point, not only differentials. A `#[property]`
that returns `true` on the EVM and `false` in WebAssembly diverges; so does a `#[machine]`
whose invariants hold on one target and fail on another, step by step. Divergence is reported
before failure, because it is the more specific finding.

## Optimization levels

`-O` compiles every platform at each level listed, and each level becomes a target of its own:

```bash
femp prop -O 0,2 -t evm -t native
```

```text
PASS  core::rounding  256 cases on evm-O0, evm-O2, native-O0, native-O2
```

An optimizer that changes what code computes shows up as a divergence between `evm-O0` and
`evm-O2`, with the input that shows it.

## What counts as agreement

Each target's result is first normalized: a returned value, a panic and its reason when the
target reports one, a revert, an abort or a trap. `--compare` then decides how strictly the
targets must agree:

| Mode | Agreement |
| --- | --- |
| `normalized` | The default. Values must be equal. Panics agree unless both targets report a reason and the reasons differ, because WebAssembly traps carry none. |
| `strict` | Outcomes must be identical, reasons included. |
| `lenient` | Values must be equal; any two abnormal outcomes agree. |

Agreeing panics pass: a differential that overflows on every target for the same input agrees.
To require that it does not overflow, state it in a [`#[property]`](property-testing.md) or a
[`#[spec]`](proving.md#specs).

[Property testing](property-testing.md#outcomes-and-comparison) lists how each platform's
outcomes are read.

## Where differentials pay off

- **Code with platform variants**, such as adapters selected with `#[cfg(target = ..)]` that
  must compute the same values.
- **Arithmetic near type limits**, where `usize` widths and checked overflow behave
  differently.
- **A rewrite.** A `#[property]` that returns whether an old and a new implementation agree
  checks the two against each other, and both against every target, in the same run.
- **The compiler.** FeMP builds every target with the same compiler release; a differential
  run at several `-O` levels checks its backends against each other on your code.
