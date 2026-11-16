//! Builds the fixture ingots and compares what the library emits with what
//! `fe build` wrote for them, byte for byte (the files under
//! `tally.golden`).

use std::path::{Path, PathBuf};
use std::process::Command;

use femp_compiler::{Session, Target};
use femp_core::environment::OptLevel;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tally")
        .join(name)
}

fn golden(file: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tally.golden")
        .join(file);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn text(file: &str) -> String {
    String::from_utf8(golden(file)).expect("a text file")
}

fn open(name: &str, target: Target) -> Session {
    Session::open(&fixture(name), target).unwrap_or_else(|error| panic!("{error}"))
}

#[test]
fn evm_contracts_match_the_compiler() {
    let built = open("contract", Target::Evm)
        .build_evm(OptLevel::default())
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(built.warnings.is_empty(), "{:?}", built.warnings);
    let [contract] = built.contracts.as_slice() else {
        panic!("one contract, not {}", built.contracts.len());
    };
    assert_eq!(contract.name, "Tally");
    assert_eq!(contract.bytecode, text("Tally.bin"));
    assert_eq!(contract.runtime, text("Tally.runtime.bin"));
    assert_eq!(
        contract.abi.as_deref(),
        Some(text("Tally.abi.json").as_str())
    );
    assert_eq!(
        contract.metadata.as_deref(),
        Some(text("Tally.metadata.json").as_str())
    );
}

#[test]
fn wasm_modules_match_the_compiler() {
    let module = open("web", Target::Wasm)
        .build_wasm(OptLevel::default())
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(module.file_name, "tally_web.wasm");
    assert_eq!(module.wasm, golden("tally_web.wasm"));
    assert_eq!(module.manifest, text("tally_web.manifest.json"));
}

#[test]
#[cfg(any(
    all(target_arch = "x86_64", target_os = "linux"),
    all(target_arch = "aarch64", target_os = "macos")
))]
fn native_executables_run() {
    let session = open("cli", Target::Native);
    assert_eq!(session.artifact_stem(), "tally_cli");
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("tally_cli");
    session
        .build_native(OptLevel::default(), &executable)
        .unwrap_or_else(|error| panic!("{error}"));
    let output = Command::new(&executable)
        .args(["1", "2", "3"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "6\n");
    let rejected = Command::new(&executable).arg("x").output().unwrap();
    assert_eq!(rejected.status.code(), Some(64));
}

#[test]
fn a_contract_build_needs_a_contract() {
    let error = open("core", Target::Evm)
        .build_evm(OptLevel::default())
        .err()
        .expect("an error");
    assert!(error.to_string().contains("no contract"), "{error}");
}

#[test]
fn errors_are_reported_as_the_compiler_prints_them() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("fe.toml"),
        "[ingot]\nname = \"broken\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/lib.fe"),
        "pub fn f() -> u8 {\n    true\n}\n",
    )
    .unwrap();
    let Err(femp_compiler::CompileError::Rejected(text)) = Session::open(dir.path(), Target::Wasm)
    else {
        panic!("the ingot is rejected");
    };
    assert!(text.contains("error"), "{text}");
    assert!(text.contains("src/lib.fe:2:5"), "{text}");
}
