# {{name}}

One Fe codebase built for EVM, native, and WebAssembly with `femp`.

| Directory | Role |
| --- | --- |
| `shared/` | Portable logic and its tests. No `target` in its `fe.toml`. |
| `evm/` | Contracts that expose the shared logic on chain. |
| `native/` | A command-line program built on the shared logic. |
| `wasm/` | Functions exported to JavaScript. |
| `spec/` | The `femp_spec` library that `#[spec]` functions use. Not a module. |

```sh
femp doctor     # what this machine can build
femp check      # type-check every module for every platform it must support
femp test       # run shared tests on all platforms and compare the outcomes
femp prove      # prove the `#[spec]` functions and other claims for all inputs
femp build      # artifacts and bindings under build/
femp package    # distributable packages under dist/
```
