//! Runs the fixture's tests on every target and checks each result against
//! what `fe test` reports for them.

use std::path::{Path, PathBuf};

use femp_compiler::{CaseResult, Session, Target, TestOptions, WasmRunner};
use femp_core::environment::OptLevel;

const HOST_SCRIPT: &str = include_str!("../../../runtime/wasm-host/host.js");
const OVERFLOW: &str =
    "Test reverted: 0x4e487b710000000000000000000000000000000000000000000000000000000000000011";

fn core() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tally/core")
}

fn node() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("node"))
        .find(|candidate| candidate.is_file())
}

fn results(target: Target, filter: Option<&str>, node: Option<&Path>) -> Vec<CaseResult> {
    let session = Session::open(&core(), target).unwrap_or_else(|error| panic!("{error}"));
    let report = session
        .test(&TestOptions {
            optimize: OptLevel::default(),
            filter,
            wasm: node.map(|node| WasmRunner {
                node,
                host_script: HOST_SCRIPT,
            }),
        })
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        report
            .log
            .lines()
            .filter(|line| line.starts_with("PASS ") || line.starts_with("FAIL "))
            .count(),
        report.cases.len()
    );
    report.cases
}

fn find<'a>(cases: &'a [CaseResult], name: &str) -> &'a CaseResult {
    cases
        .iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("no test `{name}` in {cases:?}"))
}

#[test]
fn runs_evm_tests() {
    let cases = results(Target::Evm, None, None);
    assert_eq!(cases.len(), 3);
    assert!(find(&cases, "adds").passed);
    assert!(find(&cases, "reverts_on_overflow").passed);
    let overflows = find(&cases, "overflows");
    assert!(!overflows.passed);
    assert_eq!(overflows.message.as_deref(), Some(OVERFLOW));
}

#[test]
#[cfg(any(
    all(target_arch = "x86_64", target_os = "linux"),
    all(target_arch = "aarch64", target_os = "macos")
))]
fn runs_native_tests() {
    let cases = results(Target::Native, None, None);
    assert_eq!(cases.len(), 3);
    assert!(find(&cases, "adds").passed);
    assert!(find(&cases, "reverts_on_overflow").passed);
    let overflows = find(&cases, "overflows");
    assert!(!overflows.passed);
    let message = overflows.message.as_deref().unwrap_or_default();
    assert!(message.starts_with(OVERFLOW), "{message}");
}

#[test]
fn runs_wasm_tests() {
    let Some(node) = node() else {
        eprintln!("skipping: no `node` on PATH");
        return;
    };
    let cases = results(Target::Wasm, None, Some(&node));
    assert_eq!(cases.len(), 3);
    assert!(find(&cases, "adds").passed);
    let overflows = find(&cases, "overflows");
    let message = overflows.message.as_deref().unwrap_or_default();
    assert!(message.contains("trap: unreachable"), "{message}");
    // `should_revert` has no meaning outside the EVM.
    let reverts = find(&cases, "reverts_on_overflow");
    let message = reverts.message.as_deref().unwrap_or_default();
    assert!(message.contains("cannot build"), "{message}");
}

#[test]
fn filters_tests_by_name() {
    let cases = results(Target::Evm, Some("add"), None);
    assert_eq!(
        cases
            .iter()
            .map(|case| case.name.as_str())
            .collect::<Vec<_>>(),
        ["adds"]
    );
}

#[test]
fn wasm_tests_need_node() {
    let session = Session::open(&core(), Target::Wasm).unwrap();
    let error = session
        .test(&TestOptions {
            optimize: OptLevel::default(),
            filter: None,
            wasm: None,
        })
        .err()
        .expect("an error");
    assert!(error.to_string().contains("Node.js"), "{error}");
}

#[test]
fn an_ingot_without_tests_has_no_results() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tally/web");
    let session = Session::open(&dir, Target::Wasm).unwrap();
    let report = session
        .test(&TestOptions {
            optimize: OptLevel::default(),
            filter: None,
            wasm: None,
        })
        .unwrap();
    assert!(report.cases.is_empty());
}
