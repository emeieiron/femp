use std::env;
use std::fs;

use anyhow::{Context as _, Result};
use femp_core::project::{BUILD_DIR, DIST_DIR, Project};

use crate::console;

pub fn run() -> Result<bool> {
    let project = Project::discover(&env::current_dir()?)?;
    for name in [BUILD_DIR, DIST_DIR] {
        let dir = project.root.join(name);
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("cannot remove `{}`", dir.display()))?;
            console::step("Removed", name);
        }
    }
    Ok(true)
}
