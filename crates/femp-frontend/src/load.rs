use std::path::Path;

use fe_common::InputDb;
use fe_common::compilation::Target;
use fe_driver::DriverDataBase;
use url::Url;

use crate::error::LoadError;

/// One ingot, type-checked for one target.
pub struct Loaded {
    pub(crate) db: DriverDataBase,
    pub(crate) url: Url,
    pub(crate) target: Target,
}

impl Loaded {
    /// Loads the ingot in `dir` and the ingots it depends on, as the
    /// compiler would for `target` (`evm`, `native` or `wasm`).
    pub fn open(dir: &Path, target: &str) -> Result<Self, LoadError> {
        let target =
            Target::parse(target).ok_or_else(|| LoadError::UnknownTarget(target.to_string()))?;
        let dir = dir
            .canonicalize()
            .map_err(|error| LoadError::NotAnIngot(format!("{}: {error}", dir.display())))?;
        let url = Url::from_directory_path(&dir)
            .map_err(|()| LoadError::NotAnIngot(format!("{} is not a directory", dir.display())))?;
        let mut db = DriverDataBase::default();
        fe_driver::project_target::set_target(&mut db, target);
        fe_driver::init_ingot(&mut db, &url);
        let loaded = Self { db, url, target };
        loaded.check(&dir)?;
        Ok(loaded)
    }

    fn check(&self, dir: &Path) -> Result<(), LoadError> {
        let ingot = self
            .ingot()
            .ok_or_else(|| LoadError::NotAnIngot(format!("{} holds no Fe ingot", dir.display())))?;
        let diagnostics = self.db.run_on_ingot(ingot);
        if diagnostics.has_errors(&self.db) {
            return Err(LoadError::Diagnostics(diagnostics.format_diags(&self.db)));
        }
        Ok(())
    }

    pub(crate) fn ingot(&self) -> Option<fe_hir::Ingot<'_>> {
        self.db
            .workspace()
            .containing_ingot(&self.db, self.url.clone())
    }

    pub fn target_name(&self) -> &'static str {
        self.target.name()
    }

    /// The width of `usize` and `isize` on the target.
    pub(crate) fn word_bits(&self) -> u16 {
        match self.target {
            Target::Evm => 256,
            Target::Native => 64,
            Target::Wasm => 32,
        }
    }
}
