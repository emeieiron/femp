use std::path::Path;

use femp_core::Result;
use femp_core::artifact::{Artifact, ArtifactKind, reset_dir, write_file};
use femp_core::platform::Context;
use femp_core::project::Module;
use femp_toolchain::invoke::{self, BuildRequest};

use crate::bundle::bundle;

pub const MODULE: ArtifactKind = ArtifactKind("wasm-module");
pub const MANIFEST: ArtifactKind = ArtifactKind("wasm-manifest");

const STAGE_DIR: &str = "stage";

pub fn build(
    cx: &Context<'_>,
    backend: &str,
    module: &Module,
    out_dir: &Path,
) -> Result<Vec<Artifact>> {
    reset_dir(out_dir)?;

    // Checking the real ingot first reports mistakes against the files the
    // user wrote rather than against the generated bundle.
    invoke::check(&cx.environment.toolchain, backend, &module.dir)?;

    let name = &module.ingot.name;
    let staged = out_dir.join(STAGE_DIR).join(format!("{name}.fe"));
    write_file(&staged, bundle(cx.project, module)?)?;

    invoke::build(
        &cx.environment.toolchain,
        &BuildRequest {
            backend,
            input: &staged,
            out_dir,
            emit: &["wasm"],
            optimize: cx.optimize,
            standalone: true,
        },
    )?;

    Ok(vec![
        Artifact::existing(MODULE, out_dir.join(format!("{name}.wasm")))?,
        Artifact::existing(MANIFEST, out_dir.join(format!("{name}.manifest.json")))?,
    ])
}
