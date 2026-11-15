mod package_json;
mod render;

use std::path::Path;

use femp_core::artifact::{Artifact, ArtifactKind, of_kind, write_file};
use femp_core::project::{Module, Project};
use femp_core::{Error, Result};

use crate::build::MANIFEST;
use crate::manifest::WasmManifest;

pub use render::HOST_SCRIPT_FILE;

pub const TYPESCRIPT: &str = "typescript";
pub const RUNTIME: ArtifactKind = ArtifactKind("runtime");
pub const PACKAGE_MANIFEST: ArtifactKind = ArtifactKind("package-manifest");

pub const HOST_SCRIPT: &str = include_str!("../../../runtime/wasm-host/host.js");
const HOST_TYPES: &str = include_str!("../../../runtime/wasm-host/host.d.ts");

pub fn generate(
    project: &Project,
    module: &Module,
    built: &[Artifact],
    out_dir: &Path,
) -> Result<Vec<Artifact>> {
    let manifest_artifact = of_kind(built, MANIFEST)
        .next()
        .ok_or_else(|| Error::Artifact("no WebAssembly manifest was built".to_string()))?;
    let (manifest, manifest_json) = WasmManifest::read(&manifest_artifact.path)?;
    let stem = manifest
        .module
        .file
        .strip_suffix(".wasm")
        .unwrap_or(&manifest.module.file);
    let script = format!("{stem}.js");
    let types = format!("{stem}.d.ts");

    let mut files = vec![
        (
            ArtifactKind::BINDING,
            script.clone(),
            render::script(&manifest, &manifest_json),
        ),
        (
            ArtifactKind::BINDING,
            types.clone(),
            render::types(&manifest),
        ),
        (
            RUNTIME,
            render::HOST_SCRIPT_FILE.to_string(),
            HOST_SCRIPT.to_string(),
        ),
        (
            RUNTIME,
            render::HOST_TYPES_FILE.to_string(),
            HOST_TYPES.to_string(),
        ),
    ];
    let listed: Vec<String> = built
        .iter()
        .map(Artifact::file_name)
        .chain(files.iter().map(|(_, name, _)| name.clone()))
        .collect();
    files.push((
        PACKAGE_MANIFEST,
        package_json::FILE.to_string(),
        package_json::render(project, module, &script, &types, &listed),
    ));

    files
        .into_iter()
        .map(|(kind, name, contents)| {
            let path = out_dir.join(name);
            write_file(&path, contents)?;
            Ok(Artifact::new(kind, path))
        })
        .collect()
}
