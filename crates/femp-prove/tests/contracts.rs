//! Proves the invariants of the compiler's fixture contract with Z3, when
//! it is installed: one holds after every message, and one breaks after a
//! sequence the bounded search finds from deployment.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use femp_compiler::{ContractRoot, Root, Session, Target};
use femp_core::property::Location;
use femp_prove::obligation::machine::{self, ContractHandler};
use femp_prove::{Config, Dumps, Task, Verdict};

fn ledger() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../femp-compiler/tests/fixtures/ledger")
}

fn solver() -> Option<PathBuf> {
    let program = std::env::var_os("FEMP_Z3").map_or_else(|| PathBuf::from("z3"), PathBuf::from);
    let found = Command::new(&program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !found {
        eprintln!("skipping: Z3 is not installed");
    }
    found.then_some(program)
}

struct Proof {
    init: Verdict,
    steps: Vec<(String, Verdict)>,
    /// The first refuted search, with its number of steps.
    search: Option<(usize, Verdict, femp_core::property::Machine)>,
}

fn prove(invariant: &str, config: &Config) -> Proof {
    let session = Session::open(&ledger(), Target::Evm).unwrap_or_else(|error| panic!("{error}"));
    let root = ContractRoot {
        path: Vec::new(),
        name: "Ledger".to_string(),
        invariants: vec![Root::function(&[], invariant)],
    };
    let lowered = session.lower_with(&[], &[root]);
    let contract = lowered.contracts[0]
        .as_ref()
        .unwrap_or_else(|error| panic!("{error}"));
    fn handler(handler: &femp_compiler::Handler) -> ContractHandler<'_> {
        ContractHandler {
            name: &handler.name,
            params: &handler.params,
            func: handler.func,
        }
    }
    let arms: Vec<ContractHandler<'_>> = contract.arms.iter().map(handler).collect();
    let location = Location {
        file: ledger().join("src/lib.fe"),
        line: 1,
    };
    let (declared, fns) = machine::contract_machine(
        "Ledger",
        &[],
        location,
        handler(&contract.init),
        &arms,
        &[(invariant, contract.invariants[0])],
    );
    let program = &lowered.program;
    let run = |task: Task| femp_prove::prove(task, config, &Dumps::default()).verdict;
    let init = run(machine::init_task(program, &fns, &declared).unwrap());
    let steps = (0..declared.commands.len())
        .map(|command| {
            let task = machine::step_task(program, &fns, &declared, command).unwrap();
            (declared.commands[command].method.name.clone(), run(task))
        })
        .collect();
    let search = (1..=3).find_map(|steps| {
        let task = machine::search_task(program, &fns, &declared, steps).unwrap();
        match run(task) {
            verdict @ Verdict::Refuted(_) => Some((steps, verdict, declared.clone())),
            _ => None,
        }
    });
    Proof {
        init,
        steps,
        search,
    }
}

fn config() -> Option<Config> {
    Some(Config {
        solver: solver()?,
        timeout: Duration::from_secs(20),
        search_budget: Duration::from_secs(60),
        unroll: 8,
    })
}

#[test]
fn proves_an_invariant_every_message_keeps() {
    let Some(config) = config() else {
        return;
    };
    let proof = prove("within_cap", &config);
    assert!(proof.init.is_proved(), "{:?}", proof.init);
    for (name, verdict) in &proof.steps {
        assert!(verdict.is_proved(), "{name}: {verdict:?}");
    }
}

#[test]
fn refutes_an_invariant_with_a_sequence_of_messages() {
    let Some(config) = config() else {
        return;
    };
    let proof = prove("below_hundred", &config);
    assert!(proof.init.is_proved(), "{:?}", proof.init);
    let deposit = proof
        .steps
        .iter()
        .find(|(name, _)| name == "Deposit")
        .map(|(_, verdict)| verdict)
        .unwrap();
    assert!(matches!(deposit, Verdict::Refuted(_)), "{deposit:?}");
    let (steps, verdict, declared) = proof.search.expect("a sequence from deployment");
    let Verdict::Refuted(refutation) = verdict else {
        unreachable!()
    };
    let sequence = machine::decode(&declared, steps, &refutation.values).unwrap();
    // `init(cap)` lets the total reach 100, which one deposit then does.
    assert_eq!(steps, 1);
    assert!(sequence.init[0].bits() >= femp_core::property::U256::from(100u8));
    let step = &sequence.steps[0];
    assert_eq!(declared.commands[step.command].method.name, "Deposit");
}
