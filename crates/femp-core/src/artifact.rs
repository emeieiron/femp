use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArtifactKind(pub &'static str);

impl ArtifactKind {
    pub const BINDING: Self = Self("binding");
}

impl fmt::Display for ArtifactKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub kind: ArtifactKind,
    pub path: PathBuf,
}

impl Artifact {
    pub fn new(kind: ArtifactKind, path: impl Into<PathBuf>) -> Self {
        Self {
            kind,
            path: path.into(),
        }
    }

    pub fn existing(kind: ArtifactKind, path: impl Into<PathBuf>) -> Result<Self> {
        let artifact = Self::new(kind, path);
        if artifact.path.is_file() {
            Ok(artifact)
        } else {
            Err(Error::Artifact(format!(
                "expected {kind} artifact `{}` was not produced",
                artifact.path.display()
            )))
        }
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

pub fn of_kind(artifacts: &[Artifact], kind: ArtifactKind) -> impl Iterator<Item = &Artifact> {
    artifacts
        .iter()
        .filter(move |artifact| artifact.kind == kind)
}

pub fn sha256_hex(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|source| Error::io("read", path, source))?;
    Ok(sha256(&bytes))
}

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::io("create", parent, source))?;
    }
    fs::write(path, contents).map_err(|source| Error::io("write", path, source))
}

pub fn copy_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::io("create", parent, source))?;
    }
    fs::copy(from, to)
        .map(|_| ())
        .map_err(|source| Error::io("copy", from, source))
}

pub fn reset_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|source| Error::io("clear", dir, source))?;
    }
    fs::create_dir_all(dir).map_err(|source| Error::io("create", dir, source))
}

pub fn copy_flat(artifacts: &[Artifact], to_dir: &Path) -> Result<Vec<Artifact>> {
    artifacts
        .iter()
        .map(|artifact| {
            let target = to_dir.join(artifact.file_name());
            copy_file(&artifact.path, &target)?;
            Ok(Artifact::new(artifact.kind, target))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/hello.txt");
        write_file(&path, "hello").unwrap();

        assert_eq!(
            sha256_hex(&path).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn refuses_missing_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let error = Artifact::existing(ArtifactKind("bytecode"), dir.path().join("a.bin"));
        assert!(error.is_err());
    }
}
