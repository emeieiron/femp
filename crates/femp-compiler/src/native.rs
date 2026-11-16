//! Native builds: the ingot's root module compiled for the host and linked
//! into an executable by the C compiler driver, as `fe build` does.

use std::fs;
use std::path::Path;
use std::process::Command;

use femp_core::environment::OptLevel;
use hir::hir_def::HirIngot;

use crate::error::CompileError;
use crate::guard::{generate, opt_level};
use crate::session::Session;

impl Session {
    /// Compiles the ingot and links the executable at `executable`.
    pub fn build_native(&self, optimize: OptLevel, executable: &Path) -> Result<(), CompileError> {
        let db = &self.db;
        let root = self.ingot().root_mod(db);
        let artifacts = generate("native", || {
            codegen::emit_module_native_artifacts(
                db,
                root,
                codegen::NativeTarget::Host,
                codegen::NativeOutputKind::Executable,
                opt_level(optimize),
                codegen::NativeOutputSelection {
                    ir: false,
                    object: true,
                },
            )
        })?;
        let object = artifacts
            .object
            .ok_or_else(|| CompileError::Failed("the native backend emitted no object".into()))?;
        let scratch = scratch_dir("femp-native-")?;
        let object_path = scratch.path().join(format!("{}.o", self.artifact_stem()));
        fs::write(&object_path, object).map_err(|error| {
            CompileError::Failed(format!("cannot write {}: {error}", object_path.display()))
        })?;
        link(&[&object_path], executable)
    }
}

pub(crate) fn scratch_dir(prefix: &str) -> Result<tempfile::TempDir, CompileError> {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .map_err(|error| {
            CompileError::Failed(format!("cannot create a scratch directory: {error}"))
        })
}

/// Links `inputs` into `executable`. The C compiler driver supplies the
/// platform's startup objects and linker settings.
pub(crate) fn link(inputs: &[&Path], executable: &Path) -> Result<(), CompileError> {
    if let Some(dir) = executable.parent() {
        fs::create_dir_all(dir).map_err(|error| {
            CompileError::Failed(format!("cannot create {}: {error}", dir.display()))
        })?;
    }
    let output = Command::new("cc")
        .args(inputs)
        .arg("-o")
        .arg(executable)
        .output()
        .map_err(|error| CompileError::Failed(format!("cannot run the linker `cc`: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CompileError::Failed(format!(
            "the linker `cc` failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}
