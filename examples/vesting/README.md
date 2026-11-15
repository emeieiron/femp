# Vesting

A token vesting library written once and used from three places.

| Module | Directory | What it is |
| --- | --- | --- |
| `core` | `shared/` | Schedules (cliff + linear vesting), positions, claim rules, error codes, and the `Clock` port. |
| `contracts` | `evm/` | A `Vesting` contract that keeps one position in storage and reads time from the block. |
| `cli` | `native/` | A calculator: `vested`, `claimable`, `claim`. Exits with the error code on failure. |
| `web` | `wasm/` | The same calculations exported to JavaScript; `web/` is a page that uses them. |

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
