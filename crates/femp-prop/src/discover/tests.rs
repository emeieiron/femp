use std::fs;

use femp_core::ingot::IngotManifest;
use femp_core::platform::PlatformId;
use femp_core::project::{Module, ModuleRole};
use femp_core::property::{PropertyKind, Ty};

use super::*;

fn module(files: &[(&str, &str)], role: ModuleRole) -> (tempfile::TempDir, Module) {
    let dir = tempfile::tempdir().unwrap();
    for (path, source) in files {
        let path = dir.path().join("src").join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let module = Module {
        name: "core".to_string(),
        dir: dir.path().to_path_buf(),
        source_dir: dir.path().to_path_buf(),
        ingot: IngotManifest {
            name: "vesting_core".to_string(),
            version: None,
            target: None,
            dependencies: Vec::new(),
        },
        role,
    };
    (dir, module)
}

fn shared(files: &[(&str, &str)]) -> Result<Suite, Vec<String>> {
    let (_dir, module) = module(files, ModuleRole::Shared);
    discover(&module).map_err(|errors| errors.iter().map(|e| e.message.clone()).collect())
}

const PROPERTIES: &str = r#"
use ingot::schedule::Schedule

/// Vesting never exceeds the total.
#[property(cases = 500)]
#[gen(cliff = "0..=1_000", duration = "1..=1_000")]
pub fn bounded(total: u64, cliff: u64, duration: u64, now: u64) -> bool {
    true
}

#[differential]
pub fn code(_ total: u64, signed: i32) -> u32 {
    0
}

pub fn helper() -> u64 { 1 }

#[test]
fn ordinary_test() {}
"#;

const MODEL: &str = r#"
use core::marker::Copy

#[machine(steps = "1..=8", runs = 50, clock = "0..=400")]
pub struct Model { pub claimed: u64 }

impl Copy for Model {}

impl Model {
    #[init]
    #[gen(total = "1..=1000")]
    pub fn open(total: u64) -> Model { Model { claimed: 0 } }

    #[command(weight = 3)]
    pub fn claim(self, amount: u64) -> Model { self }

    #[command]
    pub fn wait(self) -> Self { self }

    #[invariant]
    pub fn sane(self) -> bool { true }

    #[observe]
    pub fn claimed(self) -> u64 { self.claimed }
}
"#;

#[test]
fn finds_properties_with_their_generators() {
    let suite = shared(&[("lib.fe", ""), ("properties.fe", PROPERTIES)]).unwrap();
    assert_eq!(suite.ingot, "vesting_core");
    let names: Vec<&str> = suite.properties.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["bounded", "code"]);

    let bounded = &suite.properties[0];
    assert_eq!(bounded.kind, PropertyKind::Holds);
    assert_eq!(bounded.path, ["properties"]);
    assert_eq!(bounded.cases, Some(500));
    assert_eq!(bounded.location.line, 7);
    let generators: Vec<&str> = bounded
        .params
        .iter()
        .map(|p| p.generator.as_str())
        .collect();
    assert_eq!(generators, ["any", "0..=1_000", "1..=1_000", "any"]);

    let code = &suite.properties[1];
    assert_eq!(code.kind, PropertyKind::Agrees);
    assert_eq!(code.result, Ty::Uint(32));
    assert_eq!(code.params[0].label, None);
    assert_eq!(code.params[1].label.as_deref(), Some("signed"));
    assert_eq!(code.params[1].ty, Ty::Int(32));
    assert_eq!(
        suite.item_path(&code.path, &code.name),
        "vesting_core::properties::code"
    );
}

#[test]
fn finds_machines_and_their_methods() {
    let suite = shared(&[("lib.fe", ""), ("model/state.fe", MODEL)]).unwrap();
    let machine = &suite.machines[0];
    assert_eq!(machine.name, "Model");
    assert_eq!(machine.path, ["model", "state"]);
    assert_eq!(machine.steps, "1..=8");
    assert_eq!(machine.runs, Some(50));
    assert_eq!(machine.clock.as_deref(), Some("0..=400"));
    assert_eq!(machine.init.name, "open");
    assert_eq!(machine.init.params[0].generator, "1..=1000");
    let commands: Vec<(&str, u32)> = machine
        .commands
        .iter()
        .map(|c| (c.method.name.as_str(), c.weight))
        .collect();
    assert_eq!(commands, [("claim", 3), ("wait", 1)]);
    assert_eq!(machine.invariants[0].name, "sane");
    assert_eq!(machine.observations[0].ty, Ty::Uint(64));
    assert!(!machine.uses_effects());
}

#[test]
fn platform_machines_may_declare_effects() {
    let source = r#"
#[machine]
pub struct Driver { pub at: Address }
impl Driver {
    #[init]
    pub fn deploy(start: u64) -> Driver uses (create: mut Create) { Driver { at: x } }
    #[command]
    pub fn put(self, value: u64) -> Driver uses (call: mut Call) { self }
    #[invariant]
    pub fn agrees(self) -> bool uses (call: Call) { true }
}
"#;
    let (_dir, module) = module(
        &[("lib.fe", source)],
        ModuleRole::Platform(PlatformId::new("evm")),
    );
    let suite = discover(&module).unwrap();
    let machine = &suite.machines[0];
    assert_eq!(machine.steps, DEFAULT_STEPS);
    assert!(machine.uses_effects());
    assert_eq!(machine.init.effects[0].ty, "Create");
    assert!(machine.init.effects[0].mutable);
    assert!(!machine.invariants[0].effects[0].mutable);

    let errors = shared(&[("lib.fe", source)]).unwrap_err();
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert!(errors[0].contains("shared module"));
}

#[test]
fn reports_malformed_declarations() {
    let source = r#"
#[property]
fn private(x: u64) -> bool { true }

#[property]
pub fn wrong_result(x: u64) -> u64 { x }

#[property]
#[gen(y = "1..=2", "loose")]
pub fn bad_gen(x: u64) -> bool { true }

#[property(tries = 3)]
pub fn bad_option(x: usize) -> bool { true }

#[differential]
pub fn generic<T>(x: u64) -> u64 { x }

#[property]
#[differential]
pub fn both(x: u64) -> bool { true }

#[property]
#[cfg(target = "evm")]
pub fn conditional() -> bool { true }

#[init]
pub fn loose() {}

#[differential]
pub fn bounded() -> u8 { 1 }

#[property]
pub fn bounded() -> bool { true }
"#;
    let errors = shared(&[("lib.fe", source)]).unwrap_err();
    let expected = [
        "must be `pub`",
        "must return `bool`",
        "not `loose`",
        "`y`, which is not a parameter",
        "`tries` is not one of them",
        "is a `usize`",
        "cannot be generic",
        "not both",
        "cannot be conditionally compiled",
        "belongs on a method",
        "must be unique",
    ];
    for fragment in expected {
        assert!(
            errors.iter().any(|error| error.contains(fragment)),
            "missing `{fragment}` in {errors:#?}"
        );
    }
}

#[test]
fn reports_malformed_machines() {
    let source = r#"
#[machine(depth = 3)]
struct Hidden { x: u64 }
impl Hidden {
    #[command]
    pub fn step(x: u64) -> u64 { x }
    #[invariant]
    pub fn check(self, x: u64) -> bool { true }
    #[observe]
    pub fn look(self) -> Hidden { self }
}

pub struct Plain {}
impl Plain {
    #[command]
    pub fn step(self) -> Plain { self }
}
"#;
    let errors = shared(&[("lib.fe", source)]).unwrap_err();
    for fragment in [
        "`depth` is not one of them",
        "must be `pub`",
        "takes `self` and returns `Hidden`",
        "takes only `self` and returns `bool`",
        "an `#[observe]` takes only `self`",
        "exactly one `#[init]`",
        "not a `#[machine]` struct",
    ] {
        assert!(
            errors.iter().any(|error| error.contains(fragment)),
            "missing `{fragment}` in {errors:#?}"
        );
    }
}

#[test]
fn rejects_entries_in_conditional_modules() {
    let source = "#![cfg(target = \"wasm\")]\n#[property]\npub fn p() -> bool { true }\n";
    let errors = shared(&[("lib.fe", ""), ("only_wasm.fe", source)]).unwrap_err();
    assert!(errors[0].contains("some targets only"), "{errors:?}");
}

#[test]
fn an_empty_module_has_an_empty_suite() {
    assert!(shared(&[("lib.fe", "pub fn f() {}\n")]).unwrap().is_empty());
}
