//! Drives `femp prove` against the planted-bug fixture, whose claims fail on
//! purpose, and against the demonstrator, whose claims hold. The tests need
//! the Fe compiler and Z3 and skip themselves when either is missing.

mod common;

use std::fs;
use std::process::Command;

use common::{femp, stdout};

const PLANTED: &str = "tests/fixtures/planted";
const DEMONSTRATOR: &str = "../../examples/vesting";

fn solver_available() -> bool {
    let program = std::env::var_os("FEMP_Z3").unwrap_or_else(|| "z3".into());
    let found = Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !found {
        eprintln!("skipping: Z3 is not installed");
    }
    found
}

fn project(relative: &str) -> Option<tempfile::TempDir> {
    if !solver_available() {
        return None;
    }
    common::project(relative)
}

/// The lines of one claim's entry in a report: its status line and the
/// indented details under it.
fn entry(report: &str, label: &str) -> String {
    let lines: Vec<&str> = report.lines().collect();
    let start = lines
        .iter()
        .position(|line| !line.starts_with(' ') && line.split_whitespace().nth(1) == Some(label))
        .unwrap_or_else(|| panic!("no entry for {label} in\n{report}"));
    let mut entry = vec![lines[start]];
    entry.extend(
        lines[start + 1..]
            .iter()
            .take_while(|line| line.starts_with(' ') && !line.trim().is_empty()),
    );
    entry.join("\n") + "\n"
}

#[test]
fn planted_properties_are_refuted_and_replayed() {
    let Some(project) = project(PLANTED) else {
        return;
    };
    let output = femp(project.path(), &["prove", "--json"]);
    let report = stdout(&output);
    assert!(!output.status.success(), "{report}");

    let halves = entry(&report, "core::halves_stay_small");
    assert!(halves.starts_with("REFUTED"), "{halves}");
    assert!(halves.contains("returns false"), "{halves}");
    assert!(halves.contains("            x = 250\n"), "{halves}");
    assert!(
        halves.contains("evm returned false · native returned false · wasm returned false"),
        "{halves}"
    );
    assert!(
        halves.contains("saved to regressions/core/halves_stay_small.json"),
        "{halves}"
    );

    let negation = entry(&report, "core::negation_round_trips");
    assert!(
        negation.contains("panics (arithmetic overflow) at shared/src/lib.fe:"),
        "{negation}"
    );
    assert!(negation.contains("x = -9223372036854775808"), "{negation}");
    assert!(
        negation.contains("evm panicked (arithmetic overflow)"),
        "{negation}"
    );

    let stored = fs::read_to_string(
        project
            .path()
            .join("regressions/core/halves_stay_small.json"),
    )
    .unwrap();
    assert!(stored.contains("\"seed\": \"proof\""), "{stored}");
    let json = fs::read_to_string(project.path().join("build/prove/report.json")).unwrap();
    assert!(json.contains("\"status\": \"refuted\""), "{json}");
}

#[test]
fn proved_counterexamples_fail_property_tests_first() {
    let Some(project) = project(PLANTED) else {
        return;
    };
    femp(project.path(), &["prove", "-f", "halves"]);
    let output = femp(project.path(), &["prop", "--seed", "9", "-f", "halves"]);
    let report = stdout(&output);
    assert!(report.contains("fails on a stored regression"), "{report}");
    assert!(report.contains("      x = 250\n"), "{report}");
}

#[test]
fn lists_the_claims_property_testing_lists() {
    let Some(project) = project(PLANTED) else {
        return;
    };
    let proved = stdout(&femp(project.path(), &["prove", "--list"]));
    let tested = stdout(&femp(project.path(), &["prop", "--list"]));
    assert_eq!(proved, tested);
}

#[test]
fn the_demonstrator_claims_are_proved() {
    let Some(project) = project(DEMONSTRATOR) else {
        return;
    };
    let output = femp(project.path(), &["prove", "--strict"]);
    let report = stdout(&output);
    assert!(
        !output.status.success(),
        "the contract machine is not supported: {report}"
    );
    for name in [
        "vested_never_exceeds_total",
        "vesting_never_decreases",
        "vesting_waits_for_the_cliff",
        "vesting_completes_at_the_end",
        "claims_stay_within_what_vested",
        "schedule_error",
        "vested_amount",
        "VestingModel",
        "vested_at_spec",
        "claim_spec",
    ] {
        let line = entry(&report, &format!("core::{name}"));
        assert!(line.starts_with("PROVED"), "{report}");
        assert!(line.contains("on evm, native, wasm"), "{line}");
    }
    let contract = entry(&report, "contracts::VestingContractMachine");
    assert!(contract.starts_with("UNSUPPORTED"), "{contract}");
    assert!(contract.contains("at evm/src/machines.fe:"), "{contract}");
    let storage = entry(&report, "contracts::Vesting");
    assert!(storage.starts_with("PROVED"), "{report}");
    assert!(storage.contains("on evm"), "{storage}");
    assert!(
        report.contains("11 proved, 0 refuted, 0 unknown, 1 not supported"),
        "{report}"
    );
    let lenient = femp(project.path(), &["prove", "-f", "vest"]);
    assert!(lenient.status.success(), "{}", stdout(&lenient));
}

const SPEC_LIBRARY: &str = "../../ingots/spec";

/// Gives a copy of the planted fixture the specification library, as
/// `femp new` does.
fn with_specs(project: &std::path::Path, specs: &str) {
    common::copy_dir(&common::fixture(SPEC_LIBRARY), &project.join("spec"));
    let manifest = project.join("shared/fe.toml");
    let mut text = fs::read_to_string(&manifest).unwrap();
    text.push_str("\n[dependencies]\nfemp_spec = \"../spec\"\n");
    fs::write(&manifest, text).unwrap();
    fs::write(project.join("shared/src/specs.fe"), specs).unwrap();
}

#[test]
fn trusted_specs_stand_in_for_their_targets_and_are_listed() {
    let Some(project) = project(PLANTED) else {
        return;
    };
    with_specs(
        project.path(),
        r#"use femp_spec::{ensures, requires}
use ingot::doubled

#[spec(trusted)]
fn doubled_spec(x: u64) -> u64 {
    requires(x <= 1000)
    let result = doubled(x)
    ensures(result >= x)
    result
}

#[property]
#[gen(x = "0..=1000")]
pub fn doubling_grows(x: u64) -> bool {
    doubled(x) >= x
}

#[property]
pub fn doubling_anything(x: u64) -> bool {
    doubled(x) >= x
}
"#,
    );
    let output = femp(project.path(), &["prove", "-f", "doubl"]);
    let report = stdout(&output);
    let spec = entry(&report, "core::doubled_spec");
    assert!(spec.starts_with("SKIPPED"), "{spec}");
    assert!(spec.contains("trusted"), "{spec}");
    assert!(
        entry(&report, "core::doubling_grows").starts_with("PROVED"),
        "{report}"
    );
    let unbounded = entry(&report, "core::doubling_anything");
    assert!(unbounded.starts_with("UNKNOWN"), "{unbounded}");
    assert!(
        unbounded.contains("the precondition of `doubled_spec` may not hold"),
        "{unbounded}"
    );
    assert!(
        report.contains(
            "assumed\n  core::doubling_grows: the trusted spec `doubled_spec` stands for `doubled`"
        ),
        "{report}"
    );
}

#[test]
fn new_projects_prove_their_spec() {
    if !solver_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let created = femp(dir.path(), &["new", "fresh"]);
    assert!(created.status.success(), "{}", stdout(&created));
    let project = dir.path().join("fresh");
    if !femp(&project, &["doctor"]).status.success() {
        eprintln!("skipping: `femp doctor` reports that this machine cannot build the template");
        return;
    }
    let output = femp(&project, &["prove"]);
    let report = stdout(&output);
    assert!(output.status.success(), "{report}");
    assert!(
        entry(&report, "core::fee_spec").starts_with("PROVED"),
        "{report}"
    );
}

#[test]
fn public_specs_are_tested_and_refuted_alike() {
    let Some(project) = project(PLANTED) else {
        return;
    };
    with_specs(
        project.path(),
        r#"use femp_spec::{ensures, requires}
use ingot::doubled

/// Fails from 500 on, for even inputs only.
#[spec]
#[gen(x = "0..=1000")]
pub fn doubled_spec(x: u64) -> u64 {
    requires(x % 2 == 0)
    let result = doubled(x)
    ensures(result < 1000)
    result
}
"#,
    );
    let listed = stdout(&femp(project.path(), &["prop", "--list", "-f", "doubled"]));
    assert!(
        listed.contains("core::doubled_spec  spec  shared/src/specs.fe:"),
        "{listed}"
    );

    let output = femp(project.path(), &["prove", "-f", "doubled"]);
    let report = stdout(&output);
    let spec = entry(&report, "core::doubled_spec");
    assert!(spec.starts_with("REFUTED"), "{spec}");
    assert!(
        spec.contains("an `ensures` does not hold at shared/src/specs.fe:"),
        "{spec}"
    );
    assert!(spec.contains("            x = 500\n"), "{spec}");
    assert!(
        spec.contains("evm panicked (assertion failed) · native panicked (assertion failed)"),
        "{spec}"
    );
    assert!(spec.contains("not replayed on wasm"), "{spec}");
    assert!(
        spec.contains("saved to regressions/core/doubled_spec.json"),
        "{spec}"
    );

    let tested = femp(project.path(), &["prop", "--seed", "4", "-f", "doubled"]);
    let report = stdout(&tested);
    assert!(report.contains("fails on a stored regression"), "{report}");
    assert!(report.contains("      x = 500\n"), "{report}");
    assert!(report.contains("not run on wasm"), "{report}");
}
