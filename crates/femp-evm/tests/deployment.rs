//! Deploys the compiler's fixture contract and sends it messages, as a
//! contract's refuting sequence is replayed.

use std::path::Path;

use femp_core::environment::{Environment, Host, OptLevel};
use femp_core::platform::{Context, Platform};
use femp_core::project::Project;
use femp_core::property::{Delivery, Outcome, Transaction, Transactions, Ty, Value};
use femp_evm::EvmPlatform;
use femp_vir::eval::Val;

fn value(ty: Ty, number: u128) -> Value {
    Value::from_u128(ty, number).unwrap()
}

fn call(selector: u32, name: &str, args: Vec<Value>, timestamp: u128) -> Transaction {
    Transaction {
        selector: Some(selector),
        name: name.to_string(),
        args,
        context: vec![("ctx.timestamp".to_string(), value(Ty::Uint(256), timestamp))],
    }
}

#[test]
fn replays_a_sequence_of_messages() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../femp-compiler/tests/fixtures/ledger");
    let dir = tempfile::tempdir().unwrap();
    let module = dir.path().join("evm");
    std::fs::create_dir_all(module.join("src")).unwrap();
    for file in ["fe.toml", "src/lib.fe"] {
        std::fs::copy(fixture.join(file), module.join(file)).unwrap();
    }
    std::fs::write(
        dir.path().join("femp.toml"),
        "[project]\nname = \"ledger\"\nversion = \"0.1.0\"\n\n[modules.contracts]\npath = \"evm\"\nplatform = \"evm\"\n",
    )
    .unwrap();
    let project = Project::load(dir.path()).unwrap();
    let environment = Environment {
        toolchain: femp_compiler::toolchain(),
        host: Host {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            node: None,
            node_major: None,
            has_c_compiler: false,
            z3: None,
            z3_version: None,
        },
    };
    let cx = Context {
        project: &project,
        environment: &environment,
        optimize: OptLevel::default(),
    };
    let deposit = 0x1376_5838;
    // The storage, as the prover's state, from the contract's layout.
    let session = femp_compiler::Session::open(&module, femp_compiler::Target::Evm).unwrap();
    let lowered = session.lower_with(
        &[],
        &[femp_compiler::ContractRoot {
            path: Vec::new(),
            name: "Ledger".to_string(),
            invariants: Vec::new(),
            messages: Vec::new(),
        }],
    );
    let contract = lowered.contracts[0].as_ref().unwrap();
    let layout = session.storage_layout(&[], "Ledger").unwrap();
    let table = &lowered.program.types;
    let reads = layout.slots(&contract.state, table, &[]);
    let transactions = Transactions {
        contract: "Ledger".to_string(),
        deploy: Transaction {
            selector: None,
            name: "init".to_string(),
            args: vec![value(Ty::Uint(64), 100)],
            context: Vec::new(),
        },
        calls: vec![
            call(deposit, "Deposit", vec![value(Ty::Uint(64), 60)], 5),
            call(deposit, "Deposit", vec![value(Ty::Uint(64), 41)], 6),
            call(deposit, "Deposit", vec![value(Ty::Uint(64), 40)], 7),
        ],
        reads: reads.clone(),
    };
    let run = EvmPlatform::new()
        .run_contract(
            &cx,
            &project.modules[0],
            &transactions,
            &dir.path().join("build"),
        )
        .unwrap_or_else(|error| panic!("{error}"));
    let deliveries = &run.deliveries;
    // The second deposit would take the total past the cap of 100.
    assert_eq!(deliveries.len(), 4);
    assert_eq!(deliveries[0], Delivery::Accepted);
    assert_eq!(deliveries[1], Delivery::Accepted);
    assert!(
        matches!(&deliveries[2], Delivery::Rejected(Outcome::Reverted(_))),
        "{:?}",
        deliveries[2]
    );
    assert_eq!(deliveries[3], Delivery::Accepted);

    // The storage after the last deposit: total 100 of a cap of 100, last
    // deposited at time 7.
    let words = &run.storage[3];
    let read = |slot: &femp_compiler::Word| {
        reads
            .iter()
            .position(|read| read == slot)
            .map_or([0; 32], |index| words[index])
    };
    let state = layout.decode(&contract.state, table, &[], &read).unwrap();
    let int = |number: u32| Val::int(number);
    let expected = Val::Tuple(vec![Val::Adt {
        variant: 0,
        fields: vec![int(100), int(100), int(7)],
    }]);
    assert_eq!(state, expected);
}
