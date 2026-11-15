//! Drives `femp prop` against the planted-bug fixture, whose properties fail
//! on purpose, and against the demonstrator, whose properties hold.

mod common;

use std::fs;

use common::{femp, stdout};

const PLANTED: &str = "tests/fixtures/planted";
const DEMONSTRATOR: &str = "../../examples/vesting";

fn block<'a>(report: &'a str, entry: &str) -> &'a str {
    let start = report
        .find(&format!("FAIL  core::{entry} "))
        .unwrap_or_else(|| panic!("no failure for {entry} in\n{report}"));
    let rest = &report[start..];
    let end = rest[1..]
        .find("\nFAIL")
        .or_else(|| rest.find("\n\n"))
        .map_or(rest.len(), |end| end + 1);
    &rest[..end]
}

#[test]
fn planted_failures_are_minimized_to_their_boundaries() {
    let Some(project) = common::project(PLANTED) else {
        return;
    };

    let output = femp(project.path(), &["prop", "--seed", "7"]);
    let report = stdout(&output);
    assert!(!output.status.success(), "{report}");

    let rounding = block(&report, "rounding");
    assert!(rounding.contains("diverges across targets"), "{rounding}");
    assert!(rounding.contains("      x = 1000\n"), "{rounding}");
    assert!(rounding.contains("wasm    returned 999"), "{rounding}");
    assert!(rounding.contains("native  returned 1000"), "{rounding}");

    let halves = block(&report, "halves_stay_small");
    assert!(halves.contains("      x = 250\n"), "{halves}");
    assert!(halves.contains("evm     returned false"), "{halves}");

    let negation = block(&report, "negation_round_trips");
    assert!(negation.contains("x = -9223372036854775808"), "{negation}");
    assert!(
        negation.contains("evm     panicked (arithmetic overflow)"),
        "{negation}"
    );
    assert!(
        negation.contains("wasm    panicked (reason not reported)"),
        "{negation}"
    );

    let counter = block(&report, "Counter");
    for step in [
        "1. add(amount = 10)",
        "2. add(amount = 10)",
        "3. add(amount = 10)",
    ] {
        assert!(counter.contains(step), "{counter}");
    }
    assert!(!counter.contains("4."), "{counter}");
    assert!(
        counter.contains("invariant `few_big_adds` fails after step 3"),
        "{counter}"
    );

    let stored = fs::read_to_string(project.path().join("regressions/core/rounding.json")).unwrap();
    assert!(stored.contains("\"value\": \"1000\""), "{stored}");
    assert!(stored.contains("\"fe\": \"26.6.0\""), "{stored}");
}

#[test]
fn stored_regressions_fail_first_on_the_next_run() {
    let Some(project) = common::project(PLANTED) else {
        return;
    };
    femp(project.path(), &["prop", "--seed", "7", "-f", "rounding"]);

    let output = femp(project.path(), &["prop", "--seed", "8", "-f", "rounding"]);
    let report = stdout(&output);
    assert!(!output.status.success(), "{report}");
    assert!(
        report.contains("diverges across targets on a stored regression"),
        "{report}"
    );
    assert!(report.contains("      x = 1000\n"), "{report}");
}

#[test]
fn the_same_seed_gives_the_same_report() {
    let Some(project) = common::project(PLANTED) else {
        return;
    };
    let args = ["prop", "--seed", "0x2a", "--no-regressions"];
    let first = stdout(&femp(project.path(), &args));
    let second = stdout(&femp(project.path(), &args));
    let report = |text: &str| {
        text.lines()
            .filter(|line| !line.contains("Building"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(report(&first), report(&second));
    assert!(!project.path().join("regressions").exists());
}

#[test]
fn optimization_levels_are_separate_targets() {
    let Some(project) = common::project(PLANTED) else {
        return;
    };
    let output = femp(
        project.path(),
        &[
            "prop",
            "-f",
            "rounding",
            "-O",
            "0,2",
            "--seed",
            "1",
            "--no-regressions",
            "--json",
        ],
    );
    let report = stdout(&output);
    for target in [
        "evm-O0",
        "evm-O2",
        "native-O0",
        "native-O2",
        "wasm-O0",
        "wasm-O2",
    ] {
        assert!(report.contains(target), "missing {target} in {report}");
    }
    assert!(report.contains("wasm-O2    returned 999"), "{report}");
    let json = fs::read_to_string(project.path().join("build/prop/report.json")).unwrap();
    assert!(json.contains("\"status\": \"failed\""), "{json}");
}

#[test]
fn lists_entry_points() {
    let Some(project) = common::project(PLANTED) else {
        return;
    };
    let output = femp(project.path(), &["prop", "--list"]);
    let listed = stdout(&output);
    assert!(output.status.success(), "{listed}");
    assert!(
        listed.contains("core::rounding  differential  shared/src/lib.fe:"),
        "{listed}"
    );
    assert!(listed.contains("core::Counter  machine"), "{listed}");
}

#[test]
fn invalid_declarations_are_reported_with_their_location() {
    let Some(project) = common::project(PLANTED) else {
        return;
    };
    fs::write(
        project.path().join("shared/src/broken.fe"),
        "#[property]\n#[gen(y = \"1..=2\")]\npub fn broken(x: usize) -> bool {\n    true\n}\n",
    )
    .unwrap();
    let output = femp(project.path(), &["prop", "--list"]);
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(errors.contains("shared/src/broken.fe:3"), "{errors}");
    assert!(errors.contains("is a `usize`"), "{errors}");
}

#[test]
fn the_demonstrator_holds_on_every_platform() {
    let Some(project) = common::project(DEMONSTRATOR) else {
        return;
    };
    let output = femp(project.path(), &["prop", "--seed", "3"]);
    let report = stdout(&output);
    assert!(output.status.success(), "{report}");
    assert!(report.contains("PASS  core::VestingModel"), "{report}");
    assert!(
        report.contains("PASS  contracts::VestingContractMachine"),
        "{report}"
    );
    assert!(report.contains("on evm, native, wasm"), "{report}");
    assert!(report.contains("9 passed, 0 failed"), "{report}");
}
