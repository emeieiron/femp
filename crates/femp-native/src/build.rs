use std::env;
use std::path::Path;

use femp_compiler::Target;
use femp_core::Result;
use femp_core::artifact::{Artifact, ArtifactKind, reset_dir};
use femp_core::platform::Context;
use femp_core::project::Module;

pub const EXECUTABLE: ArtifactKind = ArtifactKind("executable");

pub fn build(cx: &Context<'_>, module: &Module, out_dir: &Path) -> Result<Vec<Artifact>> {
    reset_dir(out_dir)?;
    let session = femp_compiler::load(module, Target::Native)?;
    let executable = out_dir.join(format!(
        "{}{}",
        session.artifact_stem(),
        env::consts::EXE_SUFFIX
    ));
    session.build_native(cx.optimize, &executable)?;
    Ok(vec![Artifact::existing(EXECUTABLE, executable)?])
}
