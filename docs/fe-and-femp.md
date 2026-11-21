# FeMP and the Fe toolchain

FeMP is built on the official Fe compiler, not beside it. `femp` links the compiler's own
crates at one release (26.7.0) and calls them for every check, build and test, so the two
never disagree about the language: a contract `femp build` writes is byte for byte what
`fe build` writes. What FeMP adds is a level above the compiler: a project that spans several
targets, and tools that check those targets against each other and against stated claims.

## At a glance

| | Fe toolchain (`fe`) | FeMP (`femp`) |
| --- | --- | --- |
| **Unit of work** | An ingot, or a workspace whose members share one target | A project of modules for several platforms, with shared ingots compiled into each |
| **EVM contracts** | `fe build`: bytecode, runtime bytecode, ABI, metadata | The same files, plus a TypeScript module with the typed ABI and bytecode |
| **Native programs** | `fe build --backend native`: executables, and objects and static libraries of exported functions | Executables |
| **WebAssembly** | `fe build --backend wasm`: `.wasm` module and build manifest | The same files, plus JavaScript and TypeScript bindings, a host runtime and an npm `package.json` |
| **Type checking** | `fe check`, once per ingot for its target | `femp check`: every shared module once per platform, and layering rules that keep platform-only capabilities out of shared code |
| **Unit tests** | `fe test` on the EVM (Sonatina), natively or in WebAssembly, per ingot | `femp test`: each shared test on the EVM, natively and in WebAssembly, side by side; a divergence fails the run |
| **Property testing** | – | `femp prop`: generated inputs, shrinking to a minimal counterexample, stored regressions |
| **Differential testing** | – | Every target, and every optimization level with `-O`, must compute the same outcome |
| **State machines** | – | `#[machine]` sequences, including machines that deploy and drive the project's contracts |
| **Contract claims** | – | Storage invariants and message claims, tested on the deployed contract and proved by induction |
| **Formal verification** | – | `femp prove`: properties, machines, differentials, `#[spec]` functions and contracts, with Z3 |
| **Source generation** | – | Generators declared per module, run before every command through the Fe Source Generator Protocol |
| **Packaging** | Files in an output directory | `femp package`: `dist/` with a manifest of every file's kind and SHA-256 |
| **Environment check** | – | `femp doctor`: compiler, backends, Node.js, C compiler, Z3 and per-platform availability |
| **Editor support** | `fe lsp`, `fe fmt`, `fe doc`, `fe completion`, LSIF and SCIP indexes | Uses `fe`'s; `femp generate` writes generated sources for the editor to see |
| **EVM diagnostics** | Gas benchmarks against Solidity (`fe bench`), EVM traces and test reports | – |
| **Compiler releases** | Any release you install | The release FeMP links; a project pins it in `femp.toml` |

## Where they overlap

- **Compilation.** Both use the same compiler. FeMP adds no language features and no
  preprocessing beyond the source generators you declare. Its attributes, such as
  `#[property]`, are ones the compiler accepts and ignores, and the `femp_spec` statements are
  ordinary functions that check at run time what they state.
- **Tests.** `femp test` runs `#[test]` functions as `fe test` does — in the compiler's
  in-process EVM, as native executables and in WebAssembly through Node.js — and compares shared
  tests across all three.
- **Project files.** Every FeMP module is an ingot with its own `fe.toml`. Shared, EVM, native
  and WebAssembly modules build with `fe` as they are.

## When to use plain Fe

- You build for one target, such as a set of contracts, and need nothing beyond unit tests.
- You publish a library ingot for others to depend on with `fe`.
- You need the compiler's formatter, language server, documentation, gas benchmarks or EVM
  traces. Use them alongside FeMP in a FeMP project, too.
- You want a Fe release newer than the one FeMP links.

## When FeMP pays off

- **The same logic runs on chain and off chain.** A contract, a command-line tool and a web
  front end compute fees, schedules or validation rules from one source, and `femp test`
  checks that they agree instead of trusting three reimplementations.
- **You ship WebAssembly to JavaScript.** FeMP generates typed bindings, embeds the host
  runtime and packages an npm-ready directory alongside the `.wasm` module.
- **Correctness matters more than examples can show.** Properties and differentials search
  for the input that breaks an assumption or makes two targets disagree, and proofs settle a
  claim for every input, including what a contract's storage keeps after any sequence of
  messages.
- **Code is generated.** Serializers or bindings that a generator writes from your types stay
  current on every command, without checked-in output.

## Using both

FeMP projects are Fe projects. Keep `fe` installed for editing (`fe lsp`, `fe fmt`), run
`femp generate` once so the editor sees generated sources, and use `femp` for the commands that
span platforms. Moving a shared ingot out of a FeMP project needs no changes to its code.
