use femp_core::project::Module;

use crate::session::Session;
use crate::target::Target;

/// Loads and checks a project module for `target`. Its warnings are printed
/// and its errors returned, both naming the files the user wrote.
pub fn load(module: &Module, target: Target) -> femp_core::Result<Session> {
    let session = Session::open(&module.dir, target)
        .map_err(|error| module.compiler_error(error.into()))?;
    if !session.warnings().trim().is_empty() {
        eprintln!("{}", module.diagnostics(session.warnings()).trim_end());
    }
    Ok(session)
}
