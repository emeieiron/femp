use std::fmt::Write;

use super::{
    Counterexample, EntryKind, EntryReport, FailureKindName, FailureReport, RunReport, Status,
};

const DETAIL: &str = "      ";

pub fn render(report: &RunReport) -> String {
    let mut out = String::new();
    let width = report
        .entries
        .iter()
        .map(|entry| entry.label().len())
        .max()
        .unwrap_or_default();
    for entry in &report.entries {
        render_entry(&mut out, report, entry, width);
    }
    let _ = writeln!(out, "\n{}", summary(report));
    out
}

fn unit(entry: &EntryReport, count: usize) -> String {
    let word = match (entry.kind, count) {
        (EntryKind::Machine, 1) => "run",
        (EntryKind::Machine, _) => "runs",
        (_, 1) => "case",
        _ => "cases",
    };
    format!("{count} {word}")
}

fn render_entry(out: &mut String, report: &RunReport, entry: &EntryReport, width: usize) {
    let label = entry.label();
    match &entry.status {
        Status::Passed => {
            let mut what = unit(entry, entry.cases);
            if entry.rejected > 0 {
                let _ = write!(what, " ({} rejected)", entry.rejected);
            }
            if entry.exhaustive {
                what.push_str(", exhaustive");
            }
            if entry.regressions > 0 {
                let _ = write!(
                    what,
                    ", {} replayed",
                    plural(entry.regressions, "regression")
                );
            }
            let _ = writeln!(
                out,
                "PASS  {label:<width$}  {what} on {}",
                entry.ran_on.join(", ")
            );
        }
        Status::Failed(failure) => {
            let what = match failure.kind {
                FailureKindName::Violated => "fails",
                FailureKindName::Diverged => "diverges across targets",
            };
            let when = match failure.case {
                Some(_) => format!("after {}", unit(entry, entry.cases)),
                None => "on a stored regression".to_string(),
            };
            let _ = writeln!(out, "FAIL  {label:<width$}  {what} {when}");
            render_failure(out, report, entry, failure);
        }
        Status::Skipped { reason } => {
            let _ = writeln!(out, "SKIP  {label:<width$}  {reason}");
        }
        Status::Invalid { message } => {
            let _ = writeln!(out, "ERROR {label:<width$}  invalid declaration");
            let _ = writeln!(out, "{DETAIL}{message}");
        }
        Status::Error { message } => {
            let _ = writeln!(out, "ERROR {label:<width$}  could not run");
            for line in message.lines() {
                let _ = writeln!(out, "{DETAIL}{line}");
            }
        }
    }
    for skip in &entry.skipped {
        let _ = writeln!(out, "{DETAIL}not run on {}: {}", skip.target, skip.reason);
    }
    for note in &entry.notes {
        let _ = writeln!(out, "{DETAIL}note: {note}");
    }
}

fn render_failure(
    out: &mut String,
    report: &RunReport,
    entry: &EntryReport,
    failure: &FailureReport,
) {
    match &failure.counterexample {
        Counterexample::Args { args } => {
            for arg in args {
                let _ = writeln!(out, "{DETAIL}{} = {}", arg.name, arg.value);
            }
        }
        Counterexample::Sequence {
            init,
            init_args,
            steps,
        } => {
            let call = |name: &str, args: &[super::Arg]| {
                let args: Vec<String> = args
                    .iter()
                    .map(|arg| format!("{} = {}", arg.name, arg.value))
                    .collect();
                format!("{name}({})", args.join(", "))
            };
            let _ = writeln!(out, "{DETAIL}{}", call(init, init_args));
            for (index, step) in steps.iter().enumerate() {
                let clock = if step.advance > 0 {
                    format!("  (+{}s)", step.advance)
                } else {
                    String::new()
                };
                let _ = writeln!(
                    out,
                    "{DETAIL}{}. {}{clock}",
                    index + 1,
                    call(&step.command, &step.args)
                );
            }
        }
    }
    let width = failure
        .outcomes
        .iter()
        .map(|outcome| outcome.target.len())
        .max()
        .unwrap_or_default();
    for outcome in &failure.outcomes {
        let _ = writeln!(
            out,
            "{DETAIL}{:<width$}  {}",
            outcome.target, outcome.outcome
        );
    }
    let mut provenance = format!("seed {}", failure.seed);
    if let Some(case) = failure.case {
        let _ = write!(provenance, ", case {case}");
    }
    if let Some(shrink) = &failure.shrink {
        let _ = write!(
            provenance,
            ", minimized in {}",
            plural(shrink.attempts, "run")
        );
    }
    let _ = writeln!(out, "{DETAIL}{provenance}");
    if let Some(path) = &failure.saved_to {
        let _ = writeln!(out, "{DETAIL}saved to {path}");
    }
    let _ = writeln!(
        out,
        "{DETAIL}reproduce: femp prop -f {} --seed {}",
        entry.name, report.seed
    );
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        format!("1 {word}")
    } else {
        format!("{count} {word}s")
    }
}

fn summary(report: &RunReport) -> String {
    let count = |matches: fn(&Status) -> bool| {
        report
            .entries
            .iter()
            .filter(|entry| matches(&entry.status))
            .count()
    };
    let mut parts = vec![
        format!(
            "{} passed",
            count(|status| matches!(status, Status::Passed))
        ),
        format!(
            "{} failed",
            count(|status| matches!(status, Status::Failed(_)))
        ),
    ];
    let skipped = count(|status| matches!(status, Status::Skipped { .. }));
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    let errors = count(|status| matches!(status, Status::Invalid { .. } | Status::Error { .. }));
    if errors > 0 {
        parts.push(format!("{} with errors", errors));
    }
    format!(
        "{} (seed {}, {} comparison)",
        parts.join(", "),
        report.seed,
        report.comparison
    )
}
