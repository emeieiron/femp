use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

use femp_core::environment::OptLevel;

use crate::error::CompileError;

/// Runs a backend and turns its failure or panic into an error that says
/// which backend it was.
pub fn generate<T, E: std::fmt::Display>(
    backend: &str,
    run: impl FnOnce() -> Result<T, E>,
) -> Result<T, CompileError> {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(CompileError::Failed(format!(
            "the {backend} backend failed: {error}"
        ))),
        Err(payload) => Err(CompileError::Failed(format!(
            "the {backend} backend panicked: {}",
            panic_message(payload.as_ref())
        ))),
    }
}

pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_string())
}

pub fn opt_level(level: OptLevel) -> codegen::OptLevel {
    match level {
        OptLevel::None => codegen::OptLevel::O0,
        OptLevel::Balanced => codegen::OptLevel::O1,
        OptLevel::Aggressive => codegen::OptLevel::O2,
        OptLevel::Size => codegen::OptLevel::Os,
    }
}
