use std::env;
use std::path::Path;

use femp_core::Result;
use femp_core::artifact::{Artifact, ArtifactKind, reset_dir};
use femp_core::platform::Context;
use femp_core::project::Module;
use femp_toolchain::invoke::{self, BuildRequest};

pub const EXECUTABLE: ArtifactKind = ArtifactKind("executable");

pub fn build(
    cx: &Context<'_>,
    backend: &str,
    module: &Module,
    out_dir: &Path,
) -> Result<Vec<Artifact>> {
    reset_dir(out_dir)?;
    invoke::build(
        &cx.environment.toolchain,
        &BuildRequest {
            backend,
            input: &module.dir,
            out_dir,
            emit: &["executable"],
            optimize: cx.optimize,
            standalone: false,
        },
    )?;

    let file_name = format!("{}{}", module.ingot.name, env::consts::EXE_SUFFIX);
    Ok(vec![Artifact::existing(
        EXECUTABLE,
        out_dir.join(file_name),
    )?])
}
