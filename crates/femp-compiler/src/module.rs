use femp_core::outcome::{TestCase, TestRun, TestStatus};
use femp_core::platform::PlatformId;
use femp_core::project::Module;

use crate::session::Session;
use crate::target::Target;
use crate::test::TestOptions;

/// Loads and checks a project module for `target`. Its warnings are printed
/// and its errors returned, both naming the files the user wrote.
pub fn load(module: &Module, target: Target) -> femp_core::Result<Session> {
    let session =
        Session::open(&module.dir, target).map_err(|error| module.compiler_error(error.into()))?;
    if !session.warnings().trim().is_empty() {
        eprintln!("{}", module.diagnostics(session.warnings()).trim_end());
    }
    Ok(session)
}

/// Runs a project module's tests on one platform.
pub fn test(
    module: &Module,
    platform: &PlatformId,
    target: Target,
    options: &TestOptions<'_>,
) -> femp_core::Result<TestRun> {
    let report = load(module, target)?
        .test(options)
        .map_err(|error| module.compiler_error(error.into()))?;
    Ok(TestRun {
        module: module.name.clone(),
        platform: platform.clone(),
        cases: report
            .cases
            .into_iter()
            .map(|case| TestCase {
                name: case.name,
                status: if case.passed {
                    TestStatus::Passed
                } else {
                    TestStatus::Failed
                },
            })
            .collect(),
        suite_error: None,
        log: module.diagnostics(report.log.trim_end()),
    })
}
