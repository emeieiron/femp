# Vesting

A token vesting library written once and used from three places.

| Module | Directory | What it is |
| --- | --- | --- |
| `core` | `shared/` | Schedules (cliff + linear vesting), positions, claim rules, error codes, and the `Clock` port. |
| `contracts` | `evm/` | A `Vesting` contract that keeps one position in storage and reads time from the block. |
| `cli` | `native/` | A calculator: `vested`, `claimable`, `claim`. Exits with the error code on failure. |
| `web` | `wasm/` | The same calculations exported to JavaScript; `web/` is a page that uses them. |

The shared module also declares property tests (`shared/src/properties.fe`), a state
machine (`shared/src/model.fe`) and specs (`shared/src/specs.fe`), and the contract module
declares a machine that drives the deployed contract (`evm/src/machines.fe`). `spec/` holds
the `femp_spec` library the specs use.

Amounts and timestamps are `u64`, the widest integer that crosses the WebAssembly boundary. The
contract accounts for vesting only; it does not move tokens.

## Error codes

Every platform reports failures with the codes defined in `shared/src/error.fe`: the contract
reverts with `VestingFailed(code)`, the CLI exits with the code, and the WebAssembly module
aborts with it.

| Code | Meaning |
| --- | --- |
| 1 | Total is zero |
| 2 | Duration is zero |
| 3 | Cliff is longer than the duration |
| 4 | The schedule ends beyond the largest timestamp |
| 5 | Claim of zero |
| 6 | Claim exceeds the claimable amount |
| 7 | Claimed amount exceeds the total |

## Try it

```sh
femp test       # 9 shared tests on evm, native and wasm, plus the contract's own tests
femp prop       # properties, differentials and machines with generated inputs
femp build
```

```sh
./build/native/cli/vesting_cli vested 1000 100 50 200 200            # 500
./build/native/cli/vesting_cli claim  1000 100 50 200 0 501 200      # error 6, exit status 6
```

```js
import { VestingWeb } from "./build/wasm/web/vesting_web.js";

const vesting = await VestingWeb.load();
vesting.vestedAt(1000n, 100n, 50n, 200n, 200n); // 500n
```

To open the page, serve this directory and visit `/web/`:

```sh
python3 -m http.server 4173
```

## One table, three artifacts

`e2e/run.mjs` runs the cases in `e2e/vectors.mjs` against the built contract (on a local `anvil`
chain, when Foundry is installed), the executable and the WebAssembly module, and fails unless
all of them return the expected value or error code.

```sh
node e2e/run.mjs
```

## Properties on every platform

`femp prop` generates inputs for the entry points marked in the Fe sources and runs them on the
contract harness (in an embedded EVM), a native executable and a WebAssembly module:

| Entry point | Kind | What it checks |
| --- | --- | --- |
| `vested_never_exceeds_total`, `vesting_never_decreases`, `vesting_waits_for_the_cliff`, `vesting_completes_at_the_end`, `claims_stay_within_what_vested` | `#[property]` | Invariants of `Schedule` and `Position` hold for generated schedules. |
| `schedule_error`, `vested_amount` | `#[differential]` | Every platform rejects the same schedules with the same code and computes the same amounts, including 256-bit intermediate products. |
| `VestingModel` | `#[machine]` | Sequences of `wait` and `claim` keep claims within the total and within what vested, on all three platforms. |
| `VestingContractMachine` | `#[machine]` | Claims sent to the deployed contract, with the block clock advancing between them, are accepted and rejected exactly as `Position` predicts, and the stored total matches. |

```
PASS  contracts::VestingContractMachine     50 runs on evm
PASS  core::vested_never_exceeds_total      256 cases on evm, native, wasm
...
PASS  core::VestingModel                    100 runs on evm, native, wasm
```

A failure is minimized and saved under `regressions/`, which is replayed first on later runs;
commit it to keep the case. See [property testing](../../docs/property-testing.md).

## Proved for all inputs

`femp prove` proves the same entry points for every input, together with two specs in
`shared/src/specs.fe`:

| Spec | What it states |
| --- | --- |
| `vested_at_spec` | Nothing vests before the cliff, everything vests at the end, and never more than the total. It is `transparent`: callers keep `vested_at`'s body, because they also rely on vesting growing over time, which the spec does not state. |
| `claim_spec` | A successful claim adds exactly the amount claimed, keeps the schedule, and never passes what has vested. Proofs of `VestingModel` and `claims_stay_within_what_vested` use it in place of `Position::claim`. |

`VestingModel` is proved by induction. Its `schedule_stays_valid` invariant records what
`Schedule::new` checks, which keeps `vested_at` from overflowing on any state the invariants
allow, and `wait` stops the clock at the end of `u64` instead of overflowing.

```
PROVED      core::vested_never_exceeds_total      on evm, native, wasm
...
PROVED      core::VestingModel                    on evm, native, wasm
PROVED      core::vested_at_spec                  on evm, native, wasm
PROVED      core::claim_spec                      on evm, native, wasm

10 proved, 0 refuted, 0 unknown, 1 not supported (evm, native, wasm)
```

`VestingContractMachine` drives the deployed contract through effects, which the prover does
not model yet. See [proving](../../docs/proving.md).
