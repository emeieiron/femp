use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

const ROOT_FILE: &str = "lib.fe";
const SOURCE_EXTENSION: &str = "fe";

/// An ingot's sources arranged the way the compiler sees them: `lib.fe` is
/// the root, `a.fe` is module `a`, and `a/b.fe` is module `a::b`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ModuleTree {
    pub source: String,
    /// The file the source was read from; `None` for a module that only has
    /// a directory.
    pub file: Option<PathBuf>,
    pub children: BTreeMap<String, ModuleTree>,
}

impl ModuleTree {
    pub fn read(source_dir: &Path) -> Result<Self> {
        let mut root = Self::default();
        root.read_dir(source_dir, true)?;
        Ok(root)
    }

    fn read_dir(&mut self, dir: &Path, is_root: bool) -> Result<()> {
        let entries = fs::read_dir(dir).map_err(|source| Error::io("list", dir, source))?;
        for entry in entries {
            let path = entry
                .map_err(|source| Error::io("list", dir, source))?
                .path();
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if path.is_dir() {
                self.child(stem).read_dir(&path, false)?;
            } else if path.extension().is_some_and(|ext| ext == SOURCE_EXTENSION) {
                let source =
                    fs::read_to_string(&path).map_err(|source| Error::io("read", &path, source))?;
                let is_root_file =
                    is_root && path.file_name().is_some_and(|name| name == ROOT_FILE);
                let node = if is_root_file {
                    &mut *self
                } else {
                    self.child(stem)
                };
                node.source = source;
                node.file = Some(path.clone());
            }
        }
        Ok(())
    }

    fn child(&mut self, name: &str) -> &mut Self {
        self.children.entry(name.to_string()).or_default()
    }

    pub fn map_sources(&mut self, transform: &dyn Fn(&str) -> String) {
        self.source = transform(&self.source);
        for child in self.children.values_mut() {
            child.map_sources(transform);
        }
    }

    /// Renders the tree as one source text, with every file-based module
    /// turned into an inline module. Sources are not re-indented so that
    /// multi-line string literals keep their contents.
    pub fn render(&self, out: &mut String) {
        out.push_str(&self.source);
        if !self.source.ends_with('\n') {
            out.push('\n');
        }
        for (name, child) in &self.children {
            out.push_str(&format!("\npub mod {name} {{\n"));
            child.render(out);
            out.push_str("}\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, relative: &str, contents: &str) {
        let path = dir.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn mirrors_the_module_layout() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "lib.fe", "pub use a::f\n");
        write(dir.path(), "a.fe", "pub fn f() {}\n");
        write(dir.path(), "a/b.fe", "pub fn g() {}");
        write(dir.path(), "notes.txt", "ignored");

        let tree = ModuleTree::read(dir.path()).unwrap();
        let mut rendered = String::new();
        tree.render(&mut rendered);

        assert_eq!(
            rendered,
            "pub use a::f\n\npub mod a {\npub fn f() {}\n\npub mod b {\npub fn g() {}\n}\n}\n"
        );
    }

    #[test]
    fn a_nested_lib_file_is_an_ordinary_module() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "lib.fe", "");
        write(dir.path(), "a/lib.fe", "pub fn f() {}\n");

        let tree = ModuleTree::read(dir.path()).unwrap();
        assert!(tree.children["a"].children.contains_key("lib"));
        assert_eq!(
            tree.file.as_deref(),
            Some(dir.path().join("lib.fe").as_path())
        );
        assert_eq!(tree.children["a"].file, None);
    }

    #[test]
    fn transforms_every_source() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "lib.fe", "x");
        write(dir.path(), "a.fe", "x");

        let mut tree = ModuleTree::read(dir.path()).unwrap();
        tree.map_sources(&|source| source.replace('x', "y"));

        assert_eq!(tree.source, "y");
        assert_eq!(tree.children["a"].source, "y");
    }
}
