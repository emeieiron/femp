use std::fs;
use std::path::Path;

use femp_core::artifact::{Artifact, ArtifactKind, reset_dir};
use femp_core::platform::Context;
use femp_core::project::Module;
use femp_core::{Error, Result};
use femp_toolchain::invoke::{self, BuildRequest};

pub const BYTECODE: ArtifactKind = ArtifactKind("bytecode");
pub const RUNTIME_BYTECODE: ArtifactKind = ArtifactKind("runtime-bytecode");
pub const ABI: ArtifactKind = ArtifactKind("abi");
pub const METADATA: ArtifactKind = ArtifactKind("metadata");

const EMIT: &[&str] = &["bytecode", "runtime-bytecode", "abi", "metadata"];
const ABI_SUFFIX: &str = ".abi.json";

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
            emit: EMIT,
            optimize: cx.optimize,
            standalone: false,
        },
    )?;

    let mut artifacts = Vec::new();
    for contract in contract_names(out_dir)? {
        let file = |suffix: &str| out_dir.join(format!("{contract}{suffix}"));
        artifacts.extend([
            Artifact::existing(BYTECODE, file(".bin"))?,
            Artifact::existing(RUNTIME_BYTECODE, file(".runtime.bin"))?,
            Artifact::existing(ABI, file(ABI_SUFFIX))?,
            Artifact::existing(METADATA, file(".metadata.json"))?,
        ]);
    }
    Ok(artifacts)
}

// The compiler names every artifact after its contract, so the ABI files it
// wrote are the list of contracts in the module.
fn contract_names(out_dir: &Path) -> Result<Vec<String>> {
    let entries = fs::read_dir(out_dir).map_err(|source| Error::io("list", out_dir, source))?;
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|name| name.strip_suffix(ABI_SUFFIX).map(str::to_string))
        .collect();
    names.sort();
    Ok(names)
}

pub fn contract_of(artifact: &Artifact) -> String {
    artifact
        .file_name()
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string()
}
