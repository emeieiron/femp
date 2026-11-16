use std::path::Path;

use femp_compiler::Target;
use femp_core::Result;
use femp_core::artifact::{Artifact, ArtifactKind, reset_dir, write_file};
use femp_core::platform::Context;
use femp_core::project::Module;

pub const BYTECODE: ArtifactKind = ArtifactKind("bytecode");
pub const RUNTIME_BYTECODE: ArtifactKind = ArtifactKind("runtime-bytecode");
pub const ABI: ArtifactKind = ArtifactKind("abi");
pub const METADATA: ArtifactKind = ArtifactKind("metadata");

/// Writes every contract of the module as the compiler names its files:
/// `<Contract>.bin`, `.runtime.bin`, `.abi.json` and `.metadata.json`.
pub fn build(cx: &Context<'_>, module: &Module, out_dir: &Path) -> Result<Vec<Artifact>> {
    reset_dir(out_dir)?;
    let session = femp_compiler::load(module, Target::Evm)?;
    let built = session.build_evm(cx.optimize)?;
    for warning in &built.warnings {
        eprintln!("Warning: {warning}");
    }
    let mut artifacts = Vec::new();
    for contract in built.contracts {
        let mut write = |kind, suffix: &str, contents: &str| -> Result<()> {
            let path = out_dir.join(format!("{}{suffix}", contract.name));
            write_file(&path, contents)?;
            artifacts.push(Artifact::existing(kind, path)?);
            Ok(())
        };
        write(BYTECODE, ".bin", &contract.bytecode)?;
        write(RUNTIME_BYTECODE, ".runtime.bin", &contract.runtime)?;
        if let Some(abi) = &contract.abi {
            write(ABI, ".abi.json", abi)?;
        }
        if let Some(metadata) = &contract.metadata {
            write(METADATA, ".metadata.json", metadata)?;
        }
    }
    Ok(artifacts)
}

pub fn contract_of(artifact: &Artifact) -> String {
    artifact
        .file_name()
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string()
}
