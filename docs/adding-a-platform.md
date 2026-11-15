# Adding a platform

A platform is one implementation of `femp_core::platform::Platform` in its own crate, plus one
line in the registry. Nothing else in the workspace needs to change.

## 1. Create the adapter crate

```
crates/femp-<id>/
  Cargo.toml        depends on femp-core and femp-toolchain
  src/lib.rs        the Platform implementation
  src/build.rs      compiler invocation and artifact collection
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
| `check`, `test` | Usually `femp_toolchain::module_ops::{check, test}` with your backend name. |
| `build` | Compile `module` into `out_dir` and return what was written. Use `Artifact::existing` so a file the compiler did not produce is an error, and `reset_dir` so stale output cannot leak into a build. |
| `bindings` | Generate the requested kinds from `built` into `out_dir`. Mark generated sources with `ArtifactKind::BINDING`. |
| `package` | Arrange `built` under `dist_dir` for distribution. `copy_flat` is enough for most platforms. |

Declare the artifact kinds you produce as constants:

```rust
pub const ELF: ArtifactKind = ArtifactKind("elf");
```

If the compiler needs input in a different shape than an ingot directory, prepare it inside
`build` and keep that step private to the crate, as the WebAssembly adapter does with its
bundler.

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
