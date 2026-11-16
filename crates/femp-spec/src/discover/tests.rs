use std::fs;

use femp_core::ingot::IngotManifest;
use femp_core::platform::PlatformId;
use femp_core::project::{Module, ModuleRole};
use femp_core::property::{PropertyKind, Ty};

use super::*;
use crate::claims::{ContractMode, MessageKind};

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
    discover(&module)
        .map(|claims| claims.suite)
        .map_err(|errors| errors.iter().map(|e| e.message.clone()).collect())
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
    let suite = discover(&module).unwrap().suite;
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

fn claims(files: &[(&str, &str)]) -> Result<Claims, Vec<String>> {
    let (_dir, module) = module(files, ModuleRole::Shared);
    discover(&module).map_err(|errors| errors.iter().map(|e| e.message.clone()).collect())
}

const POSITION: &str = r#"
pub struct Position { pub claimed: u64 }

impl Position {
    pub fn claim(self, amount: u64, now: u64) -> Position { self }
    pub fn step(self) -> Position { self }
}

pub struct Model {}

impl Model {
    pub fn step(self) -> Model { self }
}

pub fn open(total: u64) -> Position { Position { claimed: 0 } }
"#;

const SPECS: &str = r#"
use femp_spec::{requires, ensures}

#[spec]
#[gen(amount = "1..=10")]
fn claim_spec(position: Position, amount: u64, now: u64) -> Position {
    position.claim(amount, now)
}

#[spec(trusted, target = "position::open", timeout = 5)]
pub fn opening(_ total: u64) -> Position {
    open(total)
}

#[spec(prove, transparent, target = "Position::claim")]
fn claim_again(position: Position, amount: u64, now: u64) -> Position {
    position.claim(amount, now)
}

#[spec(target = "step")]
fn receiver_decides(model: Model) -> Model {
    model.step()
}
"#;

#[test]
fn finds_specs_and_resolves_their_targets() {
    let claims = claims(&[
        ("lib.fe", ""),
        ("position.fe", POSITION),
        ("specs.fe", SPECS),
    ])
    .unwrap();
    assert!(claims.suite.is_empty());
    let names: Vec<&str> = claims.contracts.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["claim_spec", "opening", "claim_again", "receiver_decides"]
    );
    assert_eq!(claims.contracts[3].target.to_string(), "Model::step");

    let claim = &claims.contracts[0];
    assert_eq!(claim.path, ["specs"]);
    assert_eq!(claim.mode, ContractMode::Prove);
    assert!(!claim.transparent);
    assert_eq!(claim.target.to_string(), "Position::claim");
    assert_eq!(claim.target.path, ["position"]);
    assert_eq!(claim.target.location.line, 5);
    assert_eq!(claim.params[0].ty, None);
    assert_eq!(claim.params[0].ty_text, "Position");
    assert_eq!(claim.params[1].ty, Some(Ty::Uint(64)));
    assert_eq!(claim.params[1].generator, "1..=10");

    let opening = &claims.contracts[1];
    assert_eq!(opening.mode, ContractMode::Trusted);
    assert_eq!(opening.timeout, Some(5));
    assert_eq!(opening.target.owner, None);
    assert_eq!(opening.target.name, "open");
    assert_eq!(opening.params[0].label, None);

    assert!(claims.contracts[2].transparent);
}

#[test]
fn reports_malformed_specs() {
    let source = r#"
#[spec]
fn lonely(x: u64) -> u64 { x }

#[spec(target = "missing")]
fn missing_target(x: u64) -> u64 { x }

#[spec(prove, trusted, fast)]
fn mixed_spec(x: u64) -> u64 { x }

#[spec(trusted, transparent, timeout = "soon", target = "helper")]
fn helper_spec<T>(x: T) -> u64 { 1 }

#[spec(target = "helper")]
#[gen(p = "1..=2")]
fn typed(p: Position) -> u64 { 1 }

#[spec(target = "step")]
fn ambiguous(x: u64) -> u64 { x }

#[spec(target = "helper")]
#[property]
pub fn twice(x: u64) -> bool { true }

pub fn helper() -> u64 { 1 }

pub fn mixed() -> u64 { 1 }

pub struct A {}
impl A {
    pub fn step(self) -> A { self }
    #[spec]
    fn inner_spec(self) -> A { self }
}
pub struct B {}
impl B {
    pub fn step(self) -> B { self }
}
"#;
    let errors = claims(&[("lib.fe", source)]).unwrap_err();
    for fragment in [
        "names no target",
        "`missing`, which is not a function of this module",
        "either `prove` or `trusted`",
        "`fast` is not one of them",
        "cannot be `transparent`",
        "`timeout` must be a whole number",
        "cannot be generic",
        "only `bool` and fixed-width integer parameters take generators",
        "could be `A::step` or `B::step`",
        "either `#[spec]` or `#[property]`",
        "belongs on a free function",
    ] {
        assert!(
            errors.iter().any(|error| error.contains(fragment)),
            "missing `{fragment}` in {errors:#?}"
        );
    }
}

#[test]
fn spec_names_share_the_namespace_of_entry_points() {
    let source = "pub fn f() -> bool { true }\n\
                  #[property]\npub fn f_spec() -> bool { true }\n\
                  #[spec(target = \"f\")]\nfn f_spec() -> bool { f() }\n";
    let errors = claims(&[("lib.fe", source)]).unwrap_err();
    assert!(errors[0].contains("must be unique"), "{errors:?}");
}

const LEDGER: &str = r#"
msg LedgerMsg {
    #[selector = sol("deposit(uint64)")]
    Deposit { amount: u64 } -> u64,
}

pub struct LedgerStore { pub total: u64, pub cap: u64 }

pub contract Ledger uses (ctx: Ctx) {
    mut store: LedgerStore,

    recv LedgerMsg {
        Deposit { amount } -> u64 uses (mut store) {
            store.total
        }
    }
}

#[invariant]
fn within_cap(store: LedgerStore) -> bool {
    store.total <= store.cap
}

#[invariant(contract = "Ledger")]
fn cap_is_set(store: LedgerStore) -> bool {
    true
}
"#;

#[test]
fn finds_the_invariants_of_a_contract() {
    let claims = claims(&[("lib.fe", LEDGER)]).unwrap();
    let [claim] = claims.storage.as_slice() else {
        panic!("one contract, not {:?}", claims.storage);
    };
    assert_eq!(claim.name, "Ledger");
    assert!(claim.path.is_empty());
    assert_eq!(claim.location.line, 9);
    let names: Vec<&str> = claim.invariants.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["within_cap", "cap_is_set"]);
    assert!(claims.suite.machines.is_empty());
}

#[test]
fn an_invariant_needs_its_contract() {
    let lone = "#[invariant]\nfn holds(store: u64) -> bool { true }\n";
    let errors = claims(&[("lib.fe", lone)]).unwrap_err();
    assert!(errors[0].contains("declares no contract"), "{errors:?}");

    let named = format!(
        "{LEDGER}\n#[invariant(contract = \"Vault\")]\nfn other(store: LedgerStore) -> bool {{ true }}\n"
    );
    let errors = claims(&[("lib.fe", &named)]).unwrap_err();
    assert!(errors[0].contains("no contract `Vault`"), "{errors:?}");

    let two = format!("{LEDGER}\npub contract Vault {{}}\n");
    let errors = claims(&[("lib.fe", &two)]).unwrap_err();
    assert!(errors[0].contains("several contracts"), "{errors:?}");

    let shaped = format!("{LEDGER}\n#[invariant]\nfn wrong(store: LedgerStore) -> u64 {{ 0 }}\n");
    let errors = claims(&[("lib.fe", &shaped)]).unwrap_err();
    assert!(errors[0].contains("returns `bool`"), "{errors:?}");
}

#[test]
fn finds_what_messages_ensure_and_when_they_revert() {
    let source = format!(
        "{LEDGER}\n#[ensures(message = \"Deposit\")]\nfn deposit_adds(store: LedgerStore, store_after: LedgerStore, amount: u64) -> bool {{ true }}\n\n#[reverts(message = \"Deposit\")]\nfn deposit_reverts(store: LedgerStore, amount: u64) -> bool {{ false }}\n"
    );
    let found_claims = claims(&[("lib.fe", &source)]).unwrap();
    let [claim] = found_claims.storage.as_slice() else {
        panic!("one contract, not {:?}", found_claims.storage);
    };
    let found: Vec<(&str, MessageKind, &str)> = claim
        .messages
        .iter()
        .map(|message| {
            (
                message.name.as_str(),
                message.kind,
                message.message.as_str(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("deposit_adds", MessageKind::Ensures, "Deposit"),
            ("deposit_reverts", MessageKind::Reverts, "Deposit"),
        ]
    );

    let unnamed =
        format!("{LEDGER}\n#[ensures]\nfn loose(store: LedgerStore) -> bool {{ true }}\n");
    let errors = claims(&[("lib.fe", &unnamed)]).unwrap_err();
    assert!(errors[0].contains("names its message"), "{errors:?}");
}
