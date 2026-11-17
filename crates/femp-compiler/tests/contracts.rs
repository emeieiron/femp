//! Lowers the fixture contract into the functions its invariants are
//! proved with, and compares the printed VIR with the stored snapshot;
//! `FEMP_UPDATE_FIXTURES=1` rewrites it.

use std::path::Path;

use femp_compiler::{ContractRoot, Root, Session, Target};

fn ledger() -> ContractRoot {
    ContractRoot {
        path: Vec::new(),
        name: "Ledger".to_string(),
        invariants: vec![Root::function(&[], "within_cap")],
        messages: Vec::new(),
    }
}

#[test]
fn lowers_a_contract_as_a_machine() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ledger");
    let session = Session::open(&dir, Target::Evm).unwrap_or_else(|error| panic!("{error}"));
    let lowered = session.lower_with(&[], &[ledger()]);
    let contract = match &lowered.contracts[0] {
        Ok(contract) => contract,
        Err(error) => panic!("{error}"),
    };
    let mut text = String::new();
    text.push_str(&format!("fields: {:?}\n", contract.fields));
    for handler in std::iter::once(&contract.init).chain(&contract.arms) {
        let params: Vec<String> = handler
            .params
            .iter()
            .map(|param| format!("{}: {}", param.name, param.ty))
            .collect();
        text.push_str(&format!(
            "{} ({}) context={} payable={} selector={:?}\n",
            handler.name,
            params.join(", "),
            handler.context,
            handler.payable,
            handler.selector.map(|selector| format!("{selector:#010x}"))
        ));
    }
    text.push('\n');
    text.push_str(&femp_vir::print::program(&lowered.program));
    let text = text.replace(&format!("{}/", dir.canonicalize().unwrap().display()), "");
    let snapshot = dir.with_extension("vir");
    if std::env::var_os("FEMP_UPDATE_FIXTURES").is_some() {
        std::fs::write(&snapshot, &text).unwrap();
    }
    let expected = std::fs::read_to_string(&snapshot).unwrap_or_default();
    assert_eq!(text, expected);
}

fn lower_vault(source: Option<&str>, name: &str) -> femp_compiler::Lowered {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault");
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::copy(fixture.join("fe.toml"), dir.path().join("fe.toml")).unwrap();
    let mut text = std::fs::read_to_string(fixture.join("src/lib.fe")).unwrap();
    if let Some(source) = source {
        text.push_str(source);
    }
    std::fs::write(dir.path().join("src/lib.fe"), text).unwrap();
    let session = Session::open(dir.path(), Target::Evm).unwrap_or_else(|error| panic!("{error}"));
    session.lower_with(
        &[],
        &[ContractRoot {
            path: Vec::new(),
            name: name.to_string(),
            invariants: Vec::new(),
            messages: Vec::new(),
        }],
    )
}

#[test]
fn calls_to_other_contracts_are_noted_and_may_call_back() {
    let lowered = lower_vault(None, "Vault");
    let contract = lowered.contracts[0]
        .as_ref()
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(!contract.init.calls_out);
    let calls: Vec<(&str, bool)> = contract
        .arms
        .iter()
        .map(|arm| (arm.name.as_str(), arm.calls_out))
        .collect();
    assert_eq!(calls, [("Pay", true), ("Ready", true)]);
    // Only `Pay`'s call may change state, and goes through `reenter`.
    let reenter = contract.reenter.expect("a call that may call back");
    let text = femp_vir::print::program(&lowered.program);
    let calls = text.matches(&format!("call {reenter}(")).count();
    assert_eq!(calls, 1, "{text}");
}

#[test]
fn calls_out_where_the_storage_cannot_be_written_are_not_supported() {
    let helper = r#"
fn notify(payee: Address, amount: u256) -> bool uses (call: mut Call) {
    payee.call(PayeeMsg::Receive { amount })
}

pub contract Notifier uses (call: mut Call) {
    mut store: VaultStore,

    init() {}

    recv VaultMsg {
        Pay { amount } -> bool uses (store, mut call) {
            notify(payee: store.payee, amount)
        }

        Ready -> bool { true }
    }
}
"#;
    let lowered = lower_vault(Some(helper), "Notifier");
    let error = lowered.contracts[0].as_ref().expect_err("not supported");
    assert!(
        error
            .message
            .contains("cannot write the contract's storage"),
        "{error}"
    );
}
