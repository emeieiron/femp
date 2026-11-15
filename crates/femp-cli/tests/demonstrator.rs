//! Drives the `femp` binary against a copy of the demonstrator project.
//! These tests need the Fe compiler and skip themselves when the machine
//! cannot build every platform the demonstrator targets.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const GENERATED_DIRS: &[&str] = &["build", "dist"];

fn demonstrator() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/vesting")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if GENERATED_DIRS.iter().any(|generated| name == *generated) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn femp(project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_femp"))
        .args(args)
        .current_dir(project)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn project() -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&demonstrator(), dir.path());
    if femp(dir.path(), &["doctor"]).status.success() {
        Some(dir)
    } else {
        eprintln!(
            "skipping: `femp doctor` reports that this machine cannot build the demonstrator"
        );
        None
    }
}

#[test]
fn builds_every_platform_and_its_bindings() {
    let Some(project) = project() else { return };

    let output = femp(project.path(), &["build"]);
    assert!(output.status.success(), "{}", stdout(&output));

    for artifact in [
        "build/evm/contracts/Vesting.bin",
        "build/evm/contracts/Vesting.abi.json",
        "build/evm/contracts/Vesting.ts",
        "build/native/cli/vesting_cli",
        "build/wasm/web/vesting_web.wasm",
        "build/wasm/web/vesting_web.manifest.json",
        "build/wasm/web/vesting_web.js",
        "build/wasm/web/vesting_web.d.ts",
        "build/wasm/web/host.js",
        "build/wasm/web/package.json",
    ] {
        assert!(
            project.path().join(artifact).is_file(),
            "missing {artifact}"
        );
    }

    let vested = Command::new(project.path().join("build/native/cli/vesting_cli"))
        .args(["vested", "1000", "100", "50", "200", "200"])
        .output()
        .unwrap();
    assert_eq!(stdout(&vested).trim(), "500");
}

#[test]
fn shared_tests_agree_on_every_platform() {
    let Some(project) = project() else { return };

    let output = femp(project.path(), &["test"]);
    let report = stdout(&output);
    assert!(output.status.success(), "{report}");
    assert!(report.contains("evm     native  wasm"), "{report}");
    assert!(
        report.contains("core::nothing_vests_before_the_cliff"),
        "{report}"
    );
    assert!(!report.contains("FAIL"), "{report}");
}

#[test]
fn a_platform_dependent_shared_test_is_reported_as_a_divergence() {
    let Some(project) = project() else { return };
    fs::write(
        project.path().join("shared/src/divergent.fe"),
        "#[cfg(target = \"native\")]\nfn answer() -> u64 {\n    1\n}\n\n\
         #[cfg(target = \"evm\")]\nfn answer() -> u64 {\n    2\n}\n\n\
         #[cfg(target = \"wasm\")]\nfn answer() -> u64 {\n    2\n}\n\n\
         #[test]\nfn answers_two() {\n    assert(answer() == 2)\n}\n",
    )
    .unwrap();

    let output = femp(project.path(), &["test", "--filter", "answers_two"]);
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        errors.contains("core::answers_two passes on evm, wasm but fails on native"),
        "{errors}"
    );
}

#[test]
fn packages_every_platform_with_an_index() {
    let Some(project) = project() else { return };

    let output = femp(project.path(), &["package"]);
    assert!(output.status.success(), "{}", stdout(&output));

    let index = fs::read_to_string(project.path().join("dist/manifest.json")).unwrap();
    for expected in [
        "\"evm/contracts/Vesting.bin\"",
        "\"wasm/web/vesting_web.wasm\"",
        "\"wasm/web/package.json\"",
        "vesting_cli\"",
        "\"sha256\"",
    ] {
        assert!(index.contains(expected), "missing {expected} in {index}");
    }
}

#[test]
fn a_broken_layout_is_refused() {
    let Some(project) = project() else { return };
    let ingot = project.path().join("shared/fe.toml");
    let declared = fs::read_to_string(&ingot).unwrap();
    fs::write(
        &ingot,
        declared.replace(
            "version = \"0.1.0\"",
            "version = \"0.1.0\"\ntarget = \"evm\"",
        ),
    )
    .unwrap();

    let output = femp(project.path(), &["check"]);
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        errors.contains("shared module `core` sets `target = \"evm\"`"),
        "{errors}"
    );
}
