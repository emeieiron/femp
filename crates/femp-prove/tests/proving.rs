//! Lowers the fixture ingot's properties, snapshots their tasks after each
//! pass and their SMT-LIB scripts, and proves them with Z3 when it is
//! installed. `FEMP_UPDATE_FIXTURES=1` rewrites the snapshots.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use femp_core::ingot::IngotManifest;
use femp_core::project::{Module, ModuleRole};
use femp_core::property::{PanicKind, Property};
use femp_frontend::{Loaded, Root};
use femp_prove::obligation::property;
use femp_prove::{Config, Dumps, Task, Verdict, passes, vc};
use femp_vir::Check;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claims")
}

fn properties() -> Vec<Property> {
    let dir = fixture();
    let module = Module {
        name: "core".to_string(),
        dir: dir.clone(),
        source_dir: dir.clone(),
        ingot: IngotManifest::read(&dir).unwrap(),
        role: ModuleRole::Shared,
    };
    femp_spec::discover(&module).unwrap().suite.properties
}

fn tasks(target: &str) -> Vec<(Property, Task)> {
    let properties = properties();
    let loaded = Loaded::open(&fixture(), target).unwrap_or_else(|error| panic!("{error}"));
    let roots: Vec<Root> = properties
        .iter()
        .map(|p| Root::function(&p.path, &p.name))
        .collect();
    let lowered = loaded.lower(&roots);
    properties
        .into_iter()
        .zip(lowered.roots)
        .map(|(declared, root)| {
            let func = root.unwrap_or_else(|error| panic!("{}: {error}", declared.name));
            let task = property::task(&lowered.program, func, &declared).unwrap();
            (declared, task)
        })
        .collect()
}

fn local(text: &str) -> String {
    let prefix = format!("{}/", fixture().canonicalize().unwrap().display());
    text.replace(&prefix, "")
}

fn check_snapshot(name: &str, text: &str) {
    let path = fixture().with_file_name(name);
    if std::env::var_os("FEMP_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, text).unwrap();
    }
    assert_eq!(text, std::fs::read_to_string(path).unwrap());
}

fn task_named(name: &str) -> Task {
    tasks("evm")
        .into_iter()
        .find(|(declared, _)| declared.name == name)
        .map(|(_, task)| task)
        .unwrap()
}

#[test]
fn snapshots_each_pass() {
    for name in ["width_is_bounded", "index_in_bounds"] {
        let mut task = task_named(name);
        let mut text = String::new();
        passes::run(&mut task, &passes::standard(), &mut |pass, task| {
            text.push_str(&format!("// after {pass}\n"));
            text.push_str(&femp_vir::print::function(
                &task.program.types,
                task.entry(),
            ));
            text.push('\n');
        })
        .unwrap();
        check_snapshot(&format!("{name}.passes.vir"), &local(&text));
    }
}

#[test]
fn snapshots_the_smt_script() {
    let mut task = task_named("sum_fits");
    passes::run(&mut task, &passes::standard(), &mut |_, _| {}).unwrap();
    let script = vc::encode(&task).unwrap();
    check_snapshot("sum_fits.smt2", &local(&script.text()));
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

#[test]
fn proves_and_refutes_the_fixture() {
    let Some(solver) = solver() else {
        return;
    };
    let config = Config {
        solver,
        timeout: Duration::from_secs(20),
        search_budget: Duration::from_secs(60),
        unroll: 8,
    };
    for target in ["evm", "native", "wasm"] {
        for (declared, task) in tasks(target) {
            let verdict = femp_prove::prove(task, &config, &Dumps::default()).verdict;
            let name = declared.name.as_str();
            match (name, &verdict) {
                ("width_is_bounded" | "total_is_small", Verdict::Proved) => {}
                ("sum_fits", Verdict::Refuted(refutation)) => {
                    assert_eq!(refutation.check, Check::Panic(PanicKind::Overflow));
                    assert_eq!(refutation.shown, [("x".to_string(), "200".to_string())]);
                }
                ("index_in_bounds", Verdict::Refuted(refutation)) => {
                    assert_eq!(refutation.check, Check::Panic(PanicKind::Bounds));
                    assert_eq!(refutation.shown, [("i".to_string(), "3".to_string())]);
                }
                ("division_is_safe", Verdict::Refuted(refutation)) => {
                    assert_eq!(refutation.check, Check::Panic(PanicKind::Overflow));
                    let shown: Vec<&str> = refutation
                        .shown
                        .iter()
                        .map(|(_, value)| value.as_str())
                        .collect();
                    assert_eq!(shown, ["-128", "-1"]);
                }
                (
                    "counts_to_n" | "counts_a_few" | "largest_bounds_every_element",
                    Verdict::Proved,
                ) => {}
                ("counts_many", Verdict::Refuted(refutation)) => {
                    assert_eq!(refutation.check, Check::Unwind);
                }
                ("counts_to_zero", Verdict::Refuted(refutation)) => {
                    assert_eq!(refutation.shown, [("n".to_string(), "1".to_string())]);
                }
                ("keeps_a_wrong_invariant", Verdict::Refuted(refutation)) => {
                    // The invariant is checked at the loop's head, so it
                    // must hold even when the body never runs.
                    assert_eq!(
                        refutation.check,
                        Check::Claim(
                            "the loop invariant does not hold when the loop starts".into()
                        )
                    );
                    assert_eq!(refutation.shown, [("n".to_string(), "0".to_string())]);
                }
                _ => panic!("{name} on {target}: unexpected {verdict:?}"),
            }
        }
    }
}
