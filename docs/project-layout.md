# Project layout

A FeMP project is a directory with a `femp.toml` and one Fe ingot per module.

## `femp.toml`

```toml
[project]
name = "vesting"
version = "0.1.0"

[toolchain]
fe = "26.6.0"            # optional; must be the release femp is built with

[modules.core]           # no platform: a shared module
path = "shared"

[modules.contracts]
path = "evm"
platform = "evm"

[modules.cli]
path = "native"
platform = "native"

[modules.web]
path = "wasm"
platform = "wasm"

[platform.wasm]          # optional per-platform settings
bindings = ["typescript"]
```

- **`[modules.<name>]`** — `path` is the ingot's directory. `platform` names the platform the
  module is built for; leave it out for a shared module.
- **`[platform.<id>]`** — `bindings` lists the binding kinds to generate. Left out, the platform
  generates everything it offers; `bindings = []` turns generation off.

Dependencies between modules are not repeated here. They are read from each ingot's own
`fe.toml`, where the compiler needs them anyway:

```toml
[ingot]
name = "vesting_web"
version = "0.1.0"
target = "wasm"

[dependencies]
vesting_core = "../shared"
```

A dependency need not be a module. `femp new` puts the `femp_spec` library, whose statements
`#[spec]` functions use, in `spec/`, and the shared module depends on it by path:

```toml
[dependencies]
femp_spec = "../spec"
```

## Generated source

A module can declare a named generator in `femp.toml`.

~~~toml
[modules.core.generate.serializers]
command = ["my-serializer-generator", "--source", "src/models.fe", "--type", "Transfer"]
output = "src/serializers.fe"
~~~

`femp check`, `femp build`, `femp test`, and `femp package` run each declared generator
once for each participating module before invoking a backend. FeMP starts the executable in the module's original
directory, passes the arguments directly, and reads UTF-8 Fe source from standard output.
Use standard error for diagnostics. A failing generator stops the command before compilation.

FeMP writes the source to the declared path inside the staged ingot. Each path must be a
unique `.fe` destination under `src/` and must be absent from the original ingot. The staged
ingots are private to the FeMP invocation and are removed when it finishes. FeMP writes
platform artifacts and packages to their usual directories.

Run `femp generate` to create the configured outputs in their source locations for code
navigation and direct Fe tooling. Put those generated paths in the project's ignore list.
Subsequent FeMP builds refresh these copies when their generators run. The command protects
authored files and requires its own generated header before replacing an existing file.
`femp doctor` reports missing generator executables without running them.

## Shared and platform modules

| | Shared module | Platform module |
| --- | --- | --- |
| `target` in `fe.toml` | Must be absent | Must match the module's platform |
| May depend on | Shared modules | Shared modules and modules of the same platform |
| Checked for | Every platform | Its own platform |
| Tested on | Every platform | Its own platform |
| Built into | Each platform module that uses it | Its own artifacts |

`femp` refuses to run when these rules are broken and lists every violation. Together with
`femp check`, which type-checks each shared module once per platform, they guarantee that shared
code cannot reach for a capability only one platform has: Fe rejects a `uses` clause naming an
effect the target does not provide.

## Ports and adapters

A **port** is an interface that shared code needs and platforms provide. An **adapter** is one
platform's implementation. Both are plain Fe.

Declare the port as a trait in the shared module and take it as an effect:

```fe
pub trait Clock {
    fn now(self) -> u64
}

pub fn claimable_now(_ position: Position) -> u64 uses (clock: Clock) {
    position.claimable_at(clock.now())
}
```

Provide the adapter in each platform module, built from that platform's own capability, and bind
it with `with`:

```fe
// wasm/src/lib.fe
struct HostTime { seconds: u64 }

impl Clock for HostTime {
    fn now(self) -> u64 { self.seconds }
}

pub fn claimable_now(/* ... */) -> u64 uses (time: mut std::wasm::Clock) {
    with (Clock = HostTime { seconds: time.now_ms() / 1000 }) {
        vesting_core::claimable_now(position)
    }
}
```

The contract's adapter reads the block timestamp and the CLI's reads the system clock; the
shared function is the same in all three. Shared tests bind a fixed clock the same way, so
time-dependent logic is tested deterministically on every platform.

When a shared module should carry its own adapters, select them with conditional compilation:

```fe
#[cfg(target = "evm")]
pub fn word_size() -> u64 { 32 }

#[cfg(target = "native")]
pub fn word_size() -> u64 { 8 }

#[cfg(target = "wasm")]
pub fn word_size() -> u64 { 4 }
```

`femp check` fails for a platform whose variant is missing, and `femp test` reports a
divergence if the variants disagree in a way a shared test observes.

## Writing portable tests

- Assert on returned values, including `Result` errors. Reverts cannot be asserted on
  WebAssembly, and `Result::unwrap` needs an error type that encodes as revert data.
- A test compiled for only some platforms (`#[cfg(target = ...)]`) is shown as `-` elsewhere and
  mentioned in a warning; it is not a failure.

## Output

```
build/<platform>/<module>/    artifacts and bindings of one module
dist/<platform>/...           packages (`femp package`)
dist/manifest.json            every packaged file with its kind and SHA-256
```

| Platform | `build/<platform>/<module>/` |
| --- | --- |
| `evm` | `<Contract>.bin`, `.runtime.bin`, `.abi.json`, `.metadata.json`, `<Contract>.ts` |
| `native` | `<ingot name>` (packaged under `dist/native/<arch>-<os>/`) |
| `wasm` | `<ingot name>.wasm`, `.manifest.json`, `.js`, `.d.ts`, `host.js`, `host.d.ts`, `package.json` |

### WebAssembly modules

Every non-generic `pub fn` in the root file (`src/lib.fe`) of a `wasm` module is exported.
Parameters and results may be `bool`, integers of up to 64 bits, text in
(`std::wasm::HostText`) and a tagged reply out (`std::wasm::Reply`). In the bindings, 64-bit
integers are `bigint` and narrower ones `number`.

The module is compiled as an ingot, with its dependencies, into one WebAssembly module.
Declare library dependencies in `fe.toml`; the FeMP manifest lists application modules and
generator configurations.
