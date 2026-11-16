//! Drives the `femp` binary against a copy of the demonstrator project.
//! These tests need the Fe compiler and skip themselves when the machine
//! cannot build every platform the demonstrator targets.

mod common;

use std::fs;
use std::process::Command;

use common::{femp, stdout};

fn project() -> Option<tempfile::TempDir> {
    common::project("../../examples/vesting")
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

#[cfg(unix)]
#[test]
fn generates_once_per_command_and_compiles_the_staged_source() {
    let Some(project) = project() else { return };
    let root = project.path();
    let manifest = root.join("femp.toml");
    let mut config = fs::read_to_string(&manifest).unwrap();
    config.push_str(
        r#"
[modules.core.generate.fixture]
command = ["sh", "-c", "printf x >> ../generator-runs && cat generated-source"]
output = "src/generated.fe"
"#,
    );
    fs::write(manifest, config).unwrap();
    let shared = root.join("shared");
    let source = shared.join("generated-source");
    fs::write(&source, "pub fn generated_answer() -> u64 { 42 }\n").unwrap();
    fs::write(
        shared.join("src/generated_test.fe"),
        "use ingot::generated::generated_answer\n\n\
         #[test]\nfn consumes_generated_source() { assert(generated_answer() == 42) }\n",
    )
    .unwrap();

    let output = femp(
        root,
        &[
            "test",
            "-t",
            "evm",
            "-t",
            "native",
            "-t",
            "wasm",
            "-f",
            "consumes_generated_source",
        ],
    );
    let report = stdout(&output);
    assert!(
        output.status.success(),
        "{report}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        report.contains("core::consumes_generated_source"),
        "{report}"
    );
    assert_eq!(
        fs::read_to_string(root.join("generator-runs")).unwrap(),
        "x"
    );
    assert!(!shared.join("src/generated.fe").exists());
    assert!(fs::read_dir(root.join("build")).unwrap().next().is_none());

    fs::write(source, "pub fn generated_answer() -> u64 { 43 }\n").unwrap();
    let output = femp(
        root,
        &["test", "-t", "evm", "-f", "consumes_generated_source"],
    );
    assert!(!output.status.success(), "{}", stdout(&output));
    assert!(stdout(&output).contains("1 failed"), "{}", stdout(&output));
    assert_eq!(
        fs::read_to_string(root.join("generator-runs")).unwrap(),
        "xx"
    );
}

#[cfg(unix)]
#[test]
fn leaves_authored_sources_intact_and_reports_generator_errors() {
    let Some(project) = project() else { return };
    let root = project.path();
    let manifest = root.join("femp.toml");
    let mut config = fs::read_to_string(&manifest).unwrap();
    config.push_str(
        r#"
[modules.core.generate.fixture]
command = ["sh", "-c", "echo schema error >&2; exit 7"]
output = "src/generated.fe"
"#,
    );
    fs::write(manifest, config).unwrap();

    let authored = root.join("shared/src/generated.fe");
    let contents = "pub fn handwritten() -> u64 { 42 }\n";
    fs::write(&authored, contents).unwrap();
    let output = femp(root, &["check", "-t", "evm"]);
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        errors.contains("would replace an existing source"),
        "{errors}"
    );
    assert_eq!(fs::read_to_string(&authored).unwrap(), contents);

    fs::remove_file(&authored).unwrap();
    let output = femp(root, &["check", "-t", "evm"]);
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(errors.contains("core.fixture"), "{errors}");
    assert!(errors.contains("schema error"), "{errors}");
    assert!(!stdout(&output).contains("Checking"));
    assert!(!authored.exists());
    assert!(fs::read_dir(root.join("build")).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn generation_preserves_workspace_dependencies_and_skips_unselected_targets() {
    let Some(project) = project() else { return };
    let root = project.path();
    let manifest = root.join("femp.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    fs::create_dir_all(root.join("vendor/support/src")).unwrap();
    fs::write(
        root.join("vendor/support/fe.toml"),
        "[ingot]\nname = \"support\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        root.join("vendor/support/src/lib.fe"),
        "pub fn value() -> u64 { 42 }\n",
    )
    .unwrap();
    fs::write(
        root.join("fe.toml"),
        r#"
[workspace]
name = "consumer"
version = "0.1.0"
members = ["shared", "evm", "native", "wasm"]
[dependencies]
support = { path = "vendor/support" }
"#,
    )
    .unwrap();
    let ingot = root.join("shared/fe.toml");
    let shared_manifest = fs::read_to_string(&ingot).unwrap();
    let with_support = if shared_manifest.contains("[dependencies]") {
        shared_manifest.replace("[dependencies]\n", "[dependencies]\nsupport = true\n")
    } else {
        format!("{shared_manifest}\n[dependencies]\nsupport = true\n")
    };
    fs::write(ingot, with_support).unwrap();
    let authored = root.join("shared/src/fixture.fe");
    fs::write(
        &authored,
        "use support::value\npub fn answer() -> u64 { value() }\n",
    )
    .unwrap();
    fs::write(
        &manifest,
        format!(
            r#"{original}
[modules.core.generate.fixture]
command = ["printf", "pub fn value() -> u64 {{ 1 }}\n"]
output = "src/generated.fe"

[modules.web.generate.fixture]
command = ["femp-nonexistent-generator-for-test"]
output = "src/generated.fe"
"#
        ),
    )
    .unwrap();

    let doctor = femp(root, &["doctor"]);
    assert!(!doctor.status.success());
    assert!(
        String::from_utf8_lossy(&doctor.stderr).contains("femp-nonexistent-generator-for-test")
    );
    let check = femp(root, &["check", "-t", "evm"]);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(!stdout(&check).contains("Generating web"));

    fs::write(&authored, "pub fn answer() -> u64 { false }\n").unwrap();
    let check = femp(root, &["check", "-t", "evm"]);
    assert!(!check.status.success());
    let errors = String::from_utf8_lossy(&check.stderr);
    assert!(
        errors.contains(&authored.canonicalize().unwrap().display().to_string()),
        "{errors}"
    );
}

#[cfg(unix)]
#[test]
fn editor_generation_is_refreshed_by_normal_checks() {
    let Some(project) = project() else { return };
    let root = project.path();
    let manifest = root.join("femp.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    fs::write(
        manifest,
        format!(
            r#"{original}
[modules.core.generate.fixture]
command = ["cat", "editor-input"]
output = "src/generated.fe"
"#
        ),
    )
    .unwrap();
    let input = root.join("shared/editor-input");
    let output = root.join("shared/src/generated.fe");
    fs::write(&input, "pub fn number() -> u64 { 41 }\n").unwrap();
    let result = femp(root, &["generate"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fs::read_to_string(&output).unwrap().contains("41"));

    fs::write(input, "pub fn number() -> u64 { 42 }\n").unwrap();
    let result = femp(root, &["check", "-t", "evm"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(fs::read_to_string(output).unwrap().contains("42"));
}
