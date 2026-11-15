mod render;
mod ts_types;

use std::fs;
use std::path::Path;

use femp_core::artifact::{Artifact, ArtifactKind, of_kind, write_file};
use femp_core::{Error, Result};

use crate::build::{ABI, BYTECODE, RUNTIME_BYTECODE, contract_of};

pub const TYPESCRIPT: &str = "typescript";

pub fn generate(built: &[Artifact], out_dir: &Path) -> Result<Vec<Artifact>> {
    of_kind(built, ABI)
        .map(|abi| {
            let contract = contract_of(abi);
            let abi_text = read(&abi.path)?;
            let source = render::render(&render::Contract {
                name: &contract,
                entries: parse(&abi_text, &abi.path)?,
                abi: &abi_text,
                bytecode: &read_hex(built, BYTECODE, &contract)?,
                runtime_bytecode: &read_hex(built, RUNTIME_BYTECODE, &contract)?,
            });
            let path = out_dir.join(format!("{contract}.ts"));
            write_file(&path, source)?;
            Ok(Artifact::new(ArtifactKind::BINDING, path))
        })
        .collect()
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|source| Error::io("read", path, source))
}

fn parse<T: serde::de::DeserializeOwned>(text: &str, path: &Path) -> Result<T> {
    serde_json::from_str(text).map_err(|error| {
        Error::Artifact(format!("`{}` is not a valid ABI: {error}", path.display()))
    })
}

fn read_hex(built: &[Artifact], kind: ArtifactKind, contract: &str) -> Result<String> {
    let artifact = of_kind(built, kind)
        .find(|artifact| contract_of(artifact) == contract)
        .ok_or_else(|| Error::Artifact(format!("no {kind} was built for `{contract}`")))?;
    Ok(format!("0x{}", read(&artifact.path)?.trim()))
}
