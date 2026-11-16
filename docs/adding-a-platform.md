# Adding a platform

A platform is one implementation of `femp_core::platform::Platform` in its own crate, plus one
line in the registry. Nothing else in the workspace needs to change.

## 1. Create the adapter crate

```
crates/femp-<id>/
  Cargo.toml        depends on femp-core and femp-compiler
  src/lib.rs        the Platform implementation
  src/build.rs      the build through femp-compiler, and the files it writes
```

`femp-native` is the smallest existing adapter and a good starting point.

## 2. Implement `Platform`

| Method | What to return |
| --- | --- |
| `id` | The name used in `femp.toml` and `--target`. |
| `fe_target` | The value of `target` in `fe.toml` that modules of this platform must declare. |
| `stability` | `Experimental` while the compiler backend is. |
| `capability` | `Available`, or `Unavailable` with a reason and, where one exists, a fix. Decide from the `Environment` only; do not spawn processes here. |
| `binding_kinds` | The binding kinds you can generate, or `&[]`. Projects select among them in `[platform.<id>]`. |
| `check`, `test` | Usually `femp_compiler::load(module, target)` and `femp_compiler::test(module, id, target, options)`. |
| `build` | Compile `module` into `out_dir` and return what was written. Use `Artifact::existing` so a file the compiler did not produce is an error, and `reset_dir` so stale output cannot leak into a build. |
| `bindings` | Generate the requested kinds from `built` into `out_dir`. Mark generated sources with `ArtifactKind::BINDING`. |
| `package` | Arrange `built` under `dist_dir` for distribution. `copy_flat` is enough for most platforms. |

Declare the artifact kinds you produce as constants:

```rust
pub const ELF: ArtifactKind = ArtifactKind("elf");
```

A new compiler target also needs a `femp_compiler::Target` and a build method on
`femp_compiler::Session`, since `femp-compiler` is the only crate that links the compiler.

## 3. Register it

```rust
// crates/femp-cli/src/registry.rs
registry.register(Box::new(MyPlatform::new()));
```

Add the crate to `[workspace.dependencies]` and to `femp-cli`'s dependencies.

## 4. What you get

Once registered, the platform takes part in every command without further work:

- `femp doctor` lists it with its stability and availability.
- Layering validation accepts `platform = "<id>"` and checks the module's `target`.
- `femp check` and `femp test` run every shared module on it, and its column appears in the
  parity matrix.
- `femp build` and `femp package` build its modules and include them in `dist/manifest.json`.

## 5. Test it

- Unit-test `capability` against hand-built `Environment` values.
- Capture real compiler output as fixtures and test generators against them.
- Add a platform module to a project and run `femp check`, `femp test` and `femp build`.

## 6. Property harness

To take part in `femp prop`, override `Platform::property_harness`. The default reports that
the platform cannot run property tests, and `femp prop` then fails when the platform is
selected.

The method receives a `HarnessRequest`:

- `subject`: the module whose entry points are tested;
- `suite`: the entry points, numbered properties first, then machines;
- `out_dir`: where to put the harness;
- `timeout`: the longest a case may take.

It returns a `Box<dyn Executor>`. The existing adapters all follow the same three steps:

1. **Render a harness ingot** that depends on the subject and exposes every entry point in the
   platform's calling convention. Use `femp_core::property::harness_ingot::write_harness_ingot`
   and the helpers in `femp_core::codegen::fe`.
2. **Compile it** with the adapter's own `build`.
3. **Start an executor:**
   - `supports_property` and `supports_machine` return `Support::No(reason)` for entry points
     the platform cannot run; the report shows them as not run on it.
   - `call` runs a batch of independent cases.
   - `run` runs one machine sequence from a fresh state and returns its trace. `records[0]`
     follows the initializer, and a `Halt` marks the step that stopped abnormally.
   - Every result is normalized to an `Outcome`. Use `Outcome::from_revert` for EVM-style
     revert data and `Outcome::from_panic_output` for a process's panic message.

A harness that runs as a child process can reuse `femp-harness`: `Session` manages the child,
and `wire` implements the line protocol and decodes replies.
