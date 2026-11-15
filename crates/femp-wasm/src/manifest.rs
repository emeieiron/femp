use std::fs;
use std::path::Path;

use femp_core::{Error, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct WasmManifest {
    pub module: ModuleInfo,
    pub exports: Vec<Export>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModuleInfo {
    pub file: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Export {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub params: Vec<Param>,
    #[serde(default)]
    pub result: Option<ValueType>,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Param {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ValueType {
    #[serde(rename = "type")]
    pub ty: String,
}

impl WasmManifest {
    pub fn read(path: &Path) -> Result<(Self, String)> {
        let text = fs::read_to_string(path).map_err(|source| Error::io("read", path, source))?;
        let manifest = Self::parse(&text).map_err(|error| {
            Error::Artifact(format!(
                "`{}` is not a valid WebAssembly build manifest: {error}",
                path.display()
            ))
        })?;
        Ok((manifest, text))
    }

    pub fn parse(text: &str) -> serde_json::Result<Self> {
        serde_json::from_str(text)
    }

    pub fn functions(&self) -> impl Iterator<Item = &Export> {
        self.exports
            .iter()
            .filter(|export| export.kind == "function" && export.role.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = include_str!("../tests/fixtures/vesting_web.manifest.json");

    #[test]
    fn lists_program_functions() {
        let manifest = WasmManifest::parse(MANIFEST).unwrap();
        let names: Vec<&str> = manifest.functions().map(|f| f.name.as_str()).collect();

        assert_eq!(manifest.module.file, "vesting_web.wasm");
        assert_eq!(
            names,
            [
                "claim",
                "claimable_at",
                "claimable_now",
                "validate",
                "vested_at"
            ]
        );
    }

    #[test]
    fn skips_host_support_exports() {
        let manifest = WasmManifest::parse(
            r#"{
                "module": { "file": "m.wasm" },
                "exports": [
                    { "name": "alloc", "kind": "function", "role": "allocator" },
                    { "name": "memory", "kind": "memory" },
                    { "name": "run", "kind": "function", "params": [], "result": null }
                ]
            }"#,
        )
        .unwrap();

        let names: Vec<&str> = manifest.functions().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["run"]);
    }
}
