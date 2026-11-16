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
