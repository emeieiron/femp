use std::path::Path;

use common::InputDb;
use driver::DriverDataBase;
use url::Url;

use crate::diagnostics::{self, has_source_files};
use crate::error::CompileError;
use crate::names::file_name_or;
use crate::target::Target;

/// One ingot, loaded with the ingots it depends on and checked for one
/// target, as the compiler does before it generates code.
pub struct Session {
    pub(crate) db: DriverDataBase,
    pub(crate) url: Url,
    pub(crate) target: Target,
    warnings: String,
}

impl Session {
    /// Loads and checks the ingot in `dir`. Errors in it or in a dependency
    /// reject it; warnings are kept for [`Session::warnings`].
    pub fn open(dir: &Path, target: Target) -> Result<Self, CompileError> {
        let dir = dir
            .canonicalize()
            .map_err(|error| CompileError::NotAnIngot(format!("{}: {error}", dir.display())))?;
        let url = Url::from_directory_path(&dir).map_err(|()| {
            CompileError::NotAnIngot(format!("{} is not a directory", dir.display()))
        })?;
        let mut db = DriverDataBase::default();
        driver::project_target::set_target(&mut db, target.fe());
        // Loading prints what is wrong with a manifest or a dependency itself.
        if driver::init_ingot(&mut db, &url) {
            return Err(CompileError::Rejected(format!(
                "the ingot in `{}` or one of its dependencies cannot be loaded",
                dir.display()
            )));
        }
        if let Some(mismatch) = driver::project_target::dependency_target_mismatch(&db, &url) {
            return Err(CompileError::Rejected(format!("Error: {mismatch}")));
        }
        let warnings = {
            let ingot = db
                .workspace()
                .containing_ingot(&db, url.clone())
                .ok_or_else(|| {
                    CompileError::NotAnIngot(format!("{} holds no Fe ingot", dir.display()))
                })?;
            if !has_source_files(&db, ingot) {
                return Err(CompileError::NotAnIngot(format!(
                    "the ingot in {} has no source files",
                    dir.display()
                )));
            }
            let report = diagnostics::check(&db, ingot);
            if report.has_errors {
                return Err(CompileError::Rejected(report.text));
            }
            report.text
        };
        Ok(Self {
            db,
            url,
            target,
            warnings,
        })
    }

    pub fn target(&self) -> Target {
        self.target
    }

    /// The compiler's warnings, formatted as it prints them; empty when there
    /// are none.
    pub fn warnings(&self) -> &str {
        &self.warnings
    }

    pub(crate) fn ingot(&self) -> hir::Ingot<'_> {
        self.db
            .workspace()
            .containing_ingot(&self.db, self.url.clone())
            .expect("an open session holds its ingot")
    }

    /// The stem the compiler gives the ingot's outputs: its name, or the
    /// last segment of its directory.
    pub fn artifact_stem(&self) -> String {
        let ingot = self.ingot();
        let name = ingot
            .config(&self.db)
            .as_ref()
            .and_then(|config| config.metadata.name.as_ref().map(ToString::to_string))
            .or_else(|| {
                ingot
                    .base(&self.db)
                    .path_segments()
                    .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
                    .map(str::to_string)
            })
            .unwrap_or_default();
        file_name_or(&name, "module")
    }
}
