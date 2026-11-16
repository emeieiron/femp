use femp_core::Error;
use femp_core::environment::Toolchain;

use crate::target::Target;
use crate::{LINKED_FE_COMMIT, LINKED_FE_VERSION};

/// The compiler this build of femp links.
pub fn toolchain() -> Toolchain {
    Toolchain {
        version: LINKED_FE_VERSION.to_string(),
        commit: LINKED_FE_COMMIT.to_string(),
        backends: Target::ALL
            .iter()
            .map(|target| target.backend().to_string())
            .collect(),
    }
}

/// Checks that a project's `[toolchain] fe` names the release femp links,
/// since that is the only compiler it can build the project with.
pub fn check_pin(pinned: &str) -> femp_core::Result<()> {
    if pinned == LINKED_FE_VERSION {
        return Ok(());
    }
    Err(Error::Toolchain(format!(
        "the project pins Fe {pinned}, but this femp is built with Fe {LINKED_FE_VERSION}; \
         set `fe = \"{LINKED_FE_VERSION}\"` under `[toolchain]` in femp.toml, or use a femp \
         release built with Fe {pinned}"
    )))
}

#[cfg(test)]
mod tests {
    use femp_core::manifest::DEFAULT_FE_VERSION;

    use super::*;

    #[test]
    fn new_projects_pin_the_linked_release() {
        assert_eq!(DEFAULT_FE_VERSION, LINKED_FE_VERSION);
        assert!(check_pin(DEFAULT_FE_VERSION).is_ok());
    }

    #[test]
    fn another_release_is_refused() {
        let error = check_pin("26.5.0").expect_err("an error").to_string();
        assert!(
            error.contains("26.5.0") && error.contains(LINKED_FE_VERSION),
            "{error}"
        );
    }
}
