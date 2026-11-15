use anyhow::Result;
use femp_core::environment::OptLevel;
use femp_core::outcome::{TestRun, TestStatus};
use femp_core::project::Project;
use femp_testing::matrix::ParityMatrix;
use femp_testing::{plan, render, run};

use crate::cli::TestArgs;
use crate::console;
use crate::session::Session;

pub fn run(args: &TestArgs) -> Result<bool> {
    let session = Session::open()?;
    let platforms = session.select(&args.target.targets)?;
    let inputs = session.prepare(&platforms, true)?;
    let cx = inputs.context(OptLevel::default());

    let jobs = plan::plan(cx.project, &platforms);
    let runs = run::run(&cx, &jobs, args.filter.as_deref(), &mut |finished| {
        console::step("Tested", &summary_line(finished));
    });

    print_parity(cx.project, &runs);
    print_logs(&runs, args.verbose);
    Ok(runs.iter().all(TestRun::succeeded))
}

fn summary_line(run: &TestRun) -> String {
    let passed = count(run, TestStatus::Passed);
    let failed = count(run, TestStatus::Failed);
    let outcome = match &run.suite_error {
        Some(error) => format!("could not run: {error}"),
        None if run.cases.is_empty() => "no tests".to_string(),
        None => format!("{passed} passed, {failed} failed"),
    };
    format!("{} [{}]: {outcome}", run.module, run.platform)
}

fn count(run: &TestRun, status: TestStatus) -> usize {
    run.cases
        .iter()
        .filter(|case| case.status == status)
        .count()
}

fn print_parity(project: &Project, runs: &[TestRun]) {
    let shared = runs.iter().filter(|run| {
        project
            .module(&run.module)
            .is_some_and(|module| module.is_shared())
    });
    let matrix = ParityMatrix::from_runs(shared);
    if matrix.rows.is_empty() {
        return;
    }

    console::line("\nShared tests across platforms\n");
    console::detail(&render::table(&matrix));
    console::line("");

    let divergent: Vec<String> = matrix
        .divergent()
        .map(|row| render::divergence(&matrix, row))
        .collect();
    for message in &divergent {
        console::error(&format!("divergence: {message}"));
    }
    let partial = matrix.partial().count();
    if partial > 0 {
        console::warning(&format!(
            "{partial} shared test(s) exist on only some platforms (shown as `-`)"
        ));
    }
    if divergent.is_empty() {
        console::step(
            "Parity",
            "every shared test behaves the same on all platforms it ran on",
        );
    }
}

fn print_logs(runs: &[TestRun], verbose: bool) {
    for run in runs.iter().filter(|run| verbose || !run.succeeded()) {
        if !run.log.is_empty() {
            console::line(format!("\n{} [{}]\n", run.module, run.platform));
            console::detail(&run.log);
        }
    }
}
