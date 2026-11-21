//! WebAssembly builds: the ingot's root module as one module, with the
//! build manifest a host checks it against.

mod manifest;

use femp_core::environment::OptLevel;
use hir::hir_def::HirIngot;

use crate::error::CompileError;
use crate::guard::{generate, opt_level};
use crate::names::file_name_or;
use crate::session::Session;
use crate::{LINKED_FE_COMMIT, LINKED_FE_VERSION};

pub struct WasmModule {
    /// The module's file name, which the manifest records.
    pub file_name: String,
    pub wasm: Vec<u8>,
    /// The manifest, as JSON text.
    pub manifest: String,
}

impl Session {
    pub fn build_wasm(
        &self,
        optimize: OptLevel,
        stack_size: Option<u32>,
    ) -> Result<WasmModule, CompileError> {
        let db = &self.db;
        let root = self.ingot().root_mod(db);
        let level = opt_level(optimize);
        let artifacts = generate("WebAssembly", || {
            codegen::emit_module_wasm_artifacts(
                db,
                root,
                level,
                codegen::WasmOutputSelection {
                    ir: false,
                    wasm: true,
                },
                stack_size,
            )
        })?;
        let wasm = artifacts.wasm.ok_or_else(|| {
            CompileError::Failed("the WebAssembly backend emitted no module".into())
        })?;
        let file_name = format!("{}.wasm", file_name_or(&self.artifact_stem(), "main"));
        let manifest = manifest::render(&manifest::ManifestInput {
            module_file: &file_name,
            module: &wasm,
            optimization: &level.to_string(),
            compiler: &format!("fe {LINKED_FE_VERSION} ({LINKED_FE_COMMIT})"),
            exports: &artifacts.exports,
        })
        .map_err(|error| {
            CompileError::Failed(format!("cannot describe the WebAssembly module: {error}"))
        })?;
        Ok(WasmModule {
            file_name,
            wasm,
            manifest,
        })
    }
}
