use std::path::Path;

use femp_compiler::Target;
use femp_core::Result;
use femp_core::artifact::{Artifact, ArtifactKind, reset_dir, write_file};
use femp_core::platform::Context;
use femp_core::project::Module;

pub const MODULE: ArtifactKind = ArtifactKind("wasm-module");
pub const MANIFEST: ArtifactKind = ArtifactKind("wasm-manifest");

/// Writes the module and its build manifest, `<ingot>.wasm` and
/// `<ingot>.manifest.json`.
pub fn build(cx: &Context<'_>, module: &Module, out_dir: &Path) -> Result<Vec<Artifact>> {
    reset_dir(out_dir)?;
    let session = femp_compiler::load(module, Target::Wasm)?;
    let built = session.build_wasm(cx.optimize)?;
    let wasm = out_dir.join(&built.file_name);
    write_file(&wasm, &built.wasm)?;
    let stem = built
        .file_name
        .strip_suffix(".wasm")
        .unwrap_or(&built.file_name);
    let manifest = out_dir.join(format!("{stem}.manifest.json"));
    write_file(&manifest, &built.manifest)?;
    Ok(vec![
        Artifact::existing(MODULE, wasm)?,
        Artifact::existing(MANIFEST, manifest)?,
    ])
}
