//! Proves the invariants of the compiler's fixture contract with Z3, when
//! it is installed: one holds after every message, and one breaks after a
//! sequence the bounded search finds from deployment.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use femp_compiler::{ContractRoot, MessageRoot, Root, Session, Target};
use femp_core::property::Location;
use femp_prove::obligation::machine::{self, ContractHandler, MessageCheck, MessageFn};
use femp_prove::{Config, Dumps, Task, Verdict};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../femp-compiler/tests/fixtures")
        .join(name)
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

fn prove(fixture_name: &str, name: &str, invariant: &str, config: &Config) -> Proof {
    prove_all(fixture_name, name, &[invariant], config)
}

/// Proves the contract's `invariants` together, as `femp prove` does.
fn prove_all(fixture_name: &str, name: &str, invariants: &[&str], config: &Config) -> Proof {
    let dir = fixture(fixture_name);
    let session = Session::open(&dir, Target::Evm).unwrap_or_else(|error| panic!("{error}"));
    let root = ContractRoot {
        path: Vec::new(),
        name: name.to_string(),
        invariants: invariants
            .iter()
            .map(|invariant| Root::function(&[], invariant))
            .collect(),
        messages: Vec::new(),
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
        file: dir.join("src/lib.fe"),
        line: 1,
    };
    let lowered_invariants: Vec<_> = invariants
        .iter()
        .zip(&contract.invariants)
        .map(|(invariant, lowered)| (*invariant, lowered.params.as_slice(), lowered.func))
        .collect();
    let (declared, fns) = machine::contract_machine(
        name,
        &[],
        location,
        handler(&contract.init),
        &arms,
        &lowered_invariants,
        contract.reenter,
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
    let search = (1..=4).find_map(|steps| {
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
    let proof = prove("ledger", "Ledger", "within_cap", &config);
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
    let proof = prove("ledger", "Ledger", "below_hundred", &config);
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

#[test]
fn proves_an_invariant_of_every_account_in_a_map() {
    let Some(config) = config() else {
        return;
    };
    let proof = prove("token", "Token", "balances_stay_under_the_cap", &config);
    assert!(proof.init.is_proved(), "{:?}", proof.init);
    for (name, verdict) in &proof.steps {
        assert!(verdict.is_proved(), "{name}: {verdict:?}");
    }
}

#[test]
fn refutes_a_bound_the_owner_can_mint_past() {
    let Some(config) = config() else {
        return;
    };
    let proof = prove("token", "Token", "supply_stays_small", &config);
    let (steps, verdict, declared) = proof.search.expect("a sequence from deployment");
    let Verdict::Refuted(refutation) = verdict else {
        unreachable!()
    };
    let sequence = machine::decode(&declared, steps, &refutation.values).unwrap();
    assert_eq!(steps, 1);
    let step = &sequence.steps[0];
    let mint = &declared.commands[step.command].method;
    assert_eq!(mint.name, "Mint");
    // Only the owner, who deployed the contract, may mint.
    let caller = mint
        .params
        .iter()
        .position(|param| param.name == "ctx.caller")
        .unwrap();
    assert_eq!(step.args[caller], sequence.init[0]);
}

#[test]
fn finds_a_sequence_of_two_claims_where_wide_values_stall_the_solver() {
    let Some(config) = config() else {
        return;
    };
    // `Claim` forgets earlier claims, so claiming everything twice breaks
    // the invariant. Among every `u64` the solver needs well over the
    // timeout to find those two steps through the vesting arithmetic;
    // among small values it finds them at once.
    let proof = prove_all(
        "schedule",
        "Schedule",
        &["claims_stay_within_total", "schedule_stays_valid"],
        &config,
    );
    assert!(proof.init.is_proved(), "{:?}", proof.init);
    let (steps, verdict, declared) = proof.search.expect("a sequence from deployment");
    let Verdict::Refuted(refutation) = verdict else {
        unreachable!()
    };
    assert_eq!(steps, 2);
    let sequence = machine::decode(&declared, steps, &refutation.values).unwrap();
    let shown = |values: &[femp_core::property::Value]| -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    };
    // init(total = 1, start = 0, cliff = 0, duration = 1), then twice
    // Claim(ctx.timestamp = 1, amount = 1): the minimum `femp prop` finds.
    assert_eq!(shown(&sequence.init), ["1", "0", "0", "1"]);
    for step in &sequence.steps {
        let claim = &declared.commands[step.command].method;
        assert_eq!(claim.name, "Claim");
        let names: Vec<&str> = claim.params.iter().map(|param| param.name.as_str()).collect();
        assert_eq!(names, ["ctx.timestamp", "amount"]);
        assert_eq!(shown(&step.args), ["1", "1"]);
    }
}

/// Proves one message claim of the token fixture: each of its checks from
/// any state the invariant allows, and, for the first that fails, a search
/// for a real sequence that ends in the message.
fn prove_message(
    claim: &str,
    message: &str,
    after: bool,
    checks: &[MessageCheck],
    config: &Config,
) -> (Vec<Verdict>, Option<femp_core::property::Sequence>) {
    let dir = fixture("token");
    let session = Session::open(&dir, Target::Evm).unwrap_or_else(|error| panic!("{error}"));
    let root = ContractRoot {
        path: Vec::new(),
        name: "Token".to_string(),
        invariants: vec![Root::function(&[], "balances_stay_under_the_cap")],
        messages: vec![MessageRoot {
            root: Root::function(&[], claim),
            message: message.to_string(),
            after,
        }],
    };
    let lowered = session.lower_with(&[], &[root]);
    let contract = lowered.contracts[0]
        .as_ref()
        .unwrap_or_else(|error| panic!("{error}"));
    let lowered_message = contract.messages[0]
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
    let invariant = &contract.invariants[0];
    let (declared, fns) = machine::contract_machine(
        "Token",
        &[],
        Location {
            file: dir.join("src/lib.fe"),
            line: 1,
        },
        handler(&contract.init),
        &arms,
        &[(
            "balances_stay_under_the_cap",
            &invariant.params,
            invariant.func,
        )],
        contract.reenter,
    );
    let claim_fn = MessageFn {
        name: claim,
        func: lowered_message.func,
        command: lowered_message.arm,
        params: &lowered_message.params,
    };
    let program = &lowered.program;
    let run = |task: Task| femp_prove::prove(task, config, &Dumps::default()).verdict;
    let verdicts: Vec<Verdict> = checks
        .iter()
        .map(|check| {
            run(machine::message_task(program, &fns, &declared, &claim_fn, *check).unwrap())
        })
        .collect();
    let failed = checks
        .iter()
        .zip(&verdicts)
        .find(|(_, verdict)| !verdict.is_proved())
        .map(|(check, _)| *check);
    let sequence = failed.and_then(|check| {
        (0..3).find_map(|before| {
            let task =
                machine::message_search_task(program, &fns, &declared, &claim_fn, check, before)
                    .unwrap();
            match run(task) {
                Verdict::Refuted(refutation) => machine::decode_message(
                    &declared,
                    lowered_message.arm,
                    before,
                    &refutation.values,
                ),
                _ => None,
            }
        })
    });
    (verdicts, sequence)
}

#[test]
fn proves_what_a_message_ensures_of_every_account() {
    let Some(config) = config() else {
        return;
    };
    let (verdicts, _) = prove_message(
        "transfer_moves_tokens",
        "Transfer",
        true,
        &[MessageCheck::Ensures],
        &config,
    );
    assert!(verdicts[0].is_proved(), "{:?}", verdicts[0]);
}

#[test]
fn proves_exactly_when_a_message_reverts() {
    let Some(config) = config() else {
        return;
    };
    let (verdicts, _) = prove_message(
        "mint_reverts",
        "Mint",
        false,
        &[MessageCheck::Reverts, MessageCheck::GoesThrough],
        &config,
    );
    for verdict in &verdicts {
        assert!(verdict.is_proved(), "{verdict:?}");
    }
}

#[test]
fn refutes_a_revert_condition_with_a_real_sequence() {
    let Some(config) = config() else {
        return;
    };
    let (verdicts, sequence) = prove_message(
        "transfer_reverts_when_short",
        "Transfer",
        false,
        &[MessageCheck::Reverts, MessageCheck::GoesThrough],
        &config,
    );
    assert!(
        matches!(verdicts[0], Verdict::Refuted(_)),
        "{:?}",
        verdicts[0]
    );
    // A transfer by someone who holds nothing returns `false`.
    let sequence = sequence.expect("a sequence from deployment");
    assert_eq!(sequence.steps.len(), 1);
}

fn step<'p>(proof: &'p Proof, message: &str) -> &'p Verdict {
    proof
        .steps
        .iter()
        .find(|(name, _)| name == message)
        .map(|(_, verdict)| verdict)
        .unwrap_or_else(|| panic!("no message `{message}`"))
}

#[test]
fn a_contract_that_counts_before_it_calls_out_keeps_its_invariant() {
    let Some(config) = config() else {
        return;
    };
    let proof = prove("vault", "Vault", "paid_within_limit", &config);
    assert!(proof.init.is_proved(), "{:?}", proof.init);
    for (name, verdict) in &proof.steps {
        assert!(verdict.is_proved(), "{name}: {verdict:?}");
    }
}

#[test]
fn a_callee_that_calls_back_sees_storage_the_invariants_allow() {
    let Some(config) = config() else {
        return;
    };
    // Paid only after the call: a payment made while the callee called
    // back pushes it past the limit.
    let late = prove("vault", "LateVault", "late_paid_within_limit", &config);
    let Verdict::Refuted(refutation) = step(&late, "Pay") else {
        panic!("{:?}", step(&late, "Pay"));
    };
    assert!(
        matches!(&refutation.check, femp_vir::Check::Claim(text) if text.contains("fails after")),
        "{:?}",
        refutation.check
    );
    assert!(
        step(&late, "Ready").is_proved(),
        "a static call cannot call back"
    );
    // No sequence whose callees do not call back breaks it.
    assert!(late.search.is_none());

    // Past the limit at the call, which a callee calling back would see.
    let eager = prove("vault", "EagerVault", "eager_paid_within_limit", &config);
    let Verdict::Refuted(refutation) = step(&eager, "Pay") else {
        panic!("{:?}", step(&eager, "Pay"));
    };
    assert_eq!(
        refutation.check,
        femp_vir::Check::Reentry("eager_paid_within_limit".to_string())
    );
    assert!(eager.search.is_none());
}
