//! Lowers the fixture ingot for every target and compares the printed VIR
//! with the stored snapshots; `FEMP_UPDATE_FIXTURES=1` rewrites them.

use std::path::{Path, PathBuf};

use femp_compiler::{Root, Session, Target};

const ROOTS: [(&str, Option<&str>); 10] = [
    ("ignores", None),
    ("spread", Some("Pair")),
    ("wraps", None),
    ("widened", None),
    ("area", None),
    ("sum_to", None),
    ("third", None),
    ("width", None),
    ("bump", None),
    ("greeting", None),
];

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basics")
}

fn roots() -> Vec<Root> {
    ROOTS
        .iter()
        .map(|(name, owner)| match owner {
            Some(owner) => Root::method(&[], owner, name),
            None => Root::function(&[], name),
        })
        .collect()
}

fn lowered(target: &str) -> String {
    let dir = fixture_dir();
    let loaded = Session::open(&dir, Target::parse(target).expect("a known target"))
        .unwrap_or_else(|error| panic!("{error}"));
    let lowered = loaded.lower(&roots());
    let mut text = String::new();
    for ((name, _), result) in ROOTS.iter().zip(&lowered.roots) {
        match result {
            Ok(id) => text.push_str(&format!("// {name}: {id}\n")),
            Err(error) => text.push_str(&format!("// {name}: not supported: {error}\n")),
        }
    }
    text.push_str(&femp_vir::print::program(&lowered.program));
    let prefix = format!("{}/", dir.canonicalize().unwrap().display());
    text.replace(&prefix, "")
}

fn check(target: &str) {
    let text = lowered(target);
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("basics.{target}.vir"));
    if std::env::var_os("FEMP_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(text, std::fs::read_to_string(path).unwrap());
}

#[test]
fn lowers_for_evm() {
    check("evm");
}

#[test]
fn lowers_for_native() {
    check("native");
}

#[test]
fn lowers_for_wasm() {
    check("wasm");
}

#[test]
fn targets_differ_where_the_code_does() {
    let evm = lowered("evm");
    let wasm = lowered("wasm");
    assert!(evm.contains("v1: u64 = 256"), "{evm}");
    assert!(wasm.contains("v1: u64 = 32"), "{wasm}");
    assert!(
        wasm.contains("u32 = 2"),
        "wasm indexes with a 32-bit usize: {wasm}"
    );
}

#[test]
fn reports_what_it_cannot_lower_with_its_location() {
    let text = lowered("evm");
    assert!(
        text.contains("// greeting: not supported: src/lib.fe:85: values of type"),
        "{text}"
    );
    // A mutable borrow is lent to the callee and written back.
    assert!(text.contains("// bump: f"), "{text}");
}

#[test]
fn rejects_ingots_with_errors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("fe.toml"),
        "[ingot]\nname = \"broken\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/lib.fe"), "pub fn f() -> u8 { true }\n").unwrap();
    let error = Session::open(dir.path(), Target::Evm)
        .err()
        .expect("an error");
    assert!(error.to_string().contains("rejected"), "{error}");
}
