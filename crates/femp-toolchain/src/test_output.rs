use femp_core::outcome::{TestCase, TestRun, TestStatus};
use femp_core::platform::PlatformId;

use crate::process::Captured;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedTests {
    pub cases: Vec<TestCase>,
    pub suite_error: Option<String>,
}

/// Reads the status lines `fe test` prints, which look like
/// `PASS  [0.0004s] test_name`. Everything else (failure details, compiler
/// diagnostics, the summary) is ignored here and kept in the run's log.
pub fn parse(output: &str) -> ParsedTests {
    let mut parsed = ParsedTests::default();
    for line in output.lines() {
        let Some((label, subject)) = status_line(line) else {
            continue;
        };
        match label {
            "PASS" => parsed.cases.push(case(subject, TestStatus::Passed)),
            "FAIL" => parsed.cases.push(case(subject, TestStatus::Failed)),
            "ERROR" => parsed.suite_error = Some(subject.to_string()),
            _ => {}
        }
    }
    parsed
}

pub fn into_run(module: &str, platform: &PlatformId, captured: &Captured) -> TestRun {
    let parsed = parse(&captured.stdout);
    let reported_failure = parsed.suite_error.is_some()
        || parsed
            .cases
            .iter()
            .any(|case| case.status == TestStatus::Failed);
    let suite_error = parsed.suite_error.or_else(|| {
        (!captured.success && !reported_failure)
            .then(|| "the test run failed without reporting a failing test".to_string())
    });
    TestRun {
        module: module.to_string(),
        platform: platform.clone(),
        cases: parsed.cases,
        suite_error,
        log: captured.combined(),
    }
}

fn status_line(line: &str) -> Option<(&str, &str)> {
    let (label, rest) = line.split_once(char::is_whitespace)?;
    let (timing, subject) = rest.trim_start().split_once("] ")?;
    timing.starts_with('[').then_some((label, subject.trim()))
}

fn case(name: &str, status: TestStatus) -> TestCase {
    TestCase {
        name: name.to_string(),
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIXED: &str = include_str!("../tests/fixtures/test_mixed.txt");
    const COMPILE_ERROR: &str = include_str!("../tests/fixtures/test_compile_error.txt");

    fn captured(stdout: &str, success: bool) -> Captured {
        Captured {
            success,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    #[test]
    fn reads_passes_and_failures() {
        let parsed = parse(MIXED);
        assert_eq!(
            parsed.cases,
            [
                case("nested_passes", TestStatus::Passed),
                case("fails", TestStatus::Failed),
                case("native_only", TestStatus::Passed),
                case("passes", TestStatus::Passed),
            ]
        );
        assert_eq!(parsed.suite_error, None);
    }

    #[test]
    fn reports_compilation_errors_as_suite_errors() {
        let parsed = parse(COMPILE_ERROR);
        assert!(parsed.cases.is_empty());
        assert_eq!(parsed.suite_error.as_deref(), Some("Compilation errors"));
    }

    #[test]
    fn a_silent_failure_becomes_a_suite_error() {
        let run = into_run("core", &PlatformId::new("wasm"), &captured("", false));
        assert!(run.suite_error.is_some());
        assert!(!run.succeeded());
    }

    #[test]
    fn a_reported_failure_is_not_a_suite_error() {
        let run = into_run("core", &PlatformId::new("native"), &captured(MIXED, false));
        assert_eq!(run.suite_error, None);
        assert!(!run.succeeded());
    }

    #[test]
    fn an_empty_successful_run_succeeds() {
        let run = into_run("core", &PlatformId::new("evm"), &captured("", true));
        assert!(run.succeeded());
    }
}
