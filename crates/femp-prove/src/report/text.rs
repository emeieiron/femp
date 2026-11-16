use std::fmt::Write;

use femp_spec::{Arg, Counterexample};

use super::{ClaimReport, ClaimStatus, ProveReport};

const STATUS_WIDTH: usize = 12;

pub fn render(report: &ProveReport) -> String {
    let mut out = String::new();
    let width = report
        .claims
        .iter()
        .map(|claim| claim.label().len())
        .max()
        .unwrap_or_default();
    for claim in &report.claims {
        render_claim(&mut out, claim, width);
    }
    let assumed = report.assumed();
    if !assumed.is_empty() {
        out.push_str("\nassumed\n");
        for line in assumed {
            let _ = writeln!(out, "  {line}");
        }
    }
    let _ = writeln!(out, "\n{}", summary(report));
    out
}

fn status_name(status: &ClaimStatus) -> &'static str {
    match status {
        ClaimStatus::Proved => "PROVED",
        ClaimStatus::Refuted(_) => "REFUTED",
        ClaimStatus::Unknown { .. } => "UNKNOWN",
        ClaimStatus::NotSupported { .. } => "UNSUPPORTED",
        ClaimStatus::Skipped { .. } => "SKIPPED",
        ClaimStatus::Error { .. } => "ERROR",
    }
}

fn detail(out: &mut String, text: &str) {
    for line in text.lines() {
        let _ = writeln!(out, "{:STATUS_WIDTH$}{line}", "");
    }
}

fn render_claim(out: &mut String, claim: &ClaimReport, width: usize) {
    let label = claim.label();
    let status = status_name(&claim.status);
    let headline = match &claim.status {
        ClaimStatus::Proved => format!("on {}", claim.targets.join(", ")),
        ClaimStatus::Refuted(refuted) => refuted.failure.clone(),
        ClaimStatus::Unknown { reason } => reason.clone(),
        ClaimStatus::NotSupported { reason, .. } => reason.clone(),
        ClaimStatus::Skipped { reason } => reason.clone(),
        ClaimStatus::Error { message } => message.clone(),
    };
    let _ = writeln!(
        out,
        "{status:<STATUS_WIDTH$}{label:<width$}  {}",
        headline.lines().next().unwrap_or_default()
    );
    if let Some(rest) = headline.split_once('\n').map(|(_, rest)| rest) {
        detail(out, rest);
    }
    match &claim.status {
        ClaimStatus::Refuted(refuted) => {
            detail(out, &counterexample(&refuted.counterexample));
            if !refuted.replay.is_empty() {
                let outcomes: Vec<String> = refuted
                    .replay
                    .iter()
                    .map(|replay| format!("{} {}", replay.target, replay.outcome))
                    .collect();
                detail(out, &outcomes.join(" · "));
            } else if !refuted.refuted_on.is_empty() {
                detail(
                    out,
                    &format!("refuted on {}", refuted.refuted_on.join(", ")),
                );
            }
            if let Some(path) = &refuted.saved_to {
                detail(out, &format!("saved to {path}"));
            }
        }
        ClaimStatus::NotSupported { location, .. } if !location.is_empty() => {
            detail(out, &format!("at {location}"));
        }
        _ => {}
    }
    for note in &claim.notes {
        detail(out, &format!("note: {note}"));
    }
}

fn args(args: &[Arg]) -> String {
    args.iter()
        .map(|arg| format!("{} = {}", arg.name, arg.value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn counterexample(counterexample: &Counterexample) -> String {
    match counterexample {
        Counterexample::Args { args } => args
            .iter()
            .map(|arg| format!("{} = {}", arg.name, arg.value))
            .collect::<Vec<_>>()
            .join("\n"),
        Counterexample::Sequence {
            init,
            init_args,
            steps,
        } => {
            let mut lines = vec![format!("{init}({})", args(init_args))];
            for (index, step) in steps.iter().enumerate() {
                lines.push(format!(
                    "{}. {}({})",
                    index + 1,
                    step.command,
                    args(&step.args)
                ));
            }
            lines.join("\n")
        }
    }
}

fn summary(report: &ProveReport) -> String {
    let count = |matches: fn(&ClaimStatus) -> bool| {
        report
            .claims
            .iter()
            .filter(|claim| matches(&claim.status))
            .count()
    };
    let mut parts = vec![
        format!("{} proved", count(|s| matches!(s, ClaimStatus::Proved))),
        format!(
            "{} refuted",
            count(|s| matches!(s, ClaimStatus::Refuted(_)))
        ),
        format!(
            "{} unknown",
            count(|s| matches!(s, ClaimStatus::Unknown { .. }))
        ),
    ];
    let unsupported = count(|s| matches!(s, ClaimStatus::NotSupported { .. }));
    if unsupported > 0 {
        parts.push(format!("{unsupported} not supported"));
    }
    let skipped = count(|s| matches!(s, ClaimStatus::Skipped { .. }));
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    let errors = count(|s| matches!(s, ClaimStatus::Error { .. }));
    if errors > 0 {
        parts.push(format!("{errors} with errors"));
    }
    format!("{} ({})", parts.join(", "), report.targets.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{ClaimKind, Refuted, Replay};

    fn claim(name: &str, status: ClaimStatus) -> ClaimReport {
        ClaimReport {
            module: "core".into(),
            name: name.into(),
            kind: ClaimKind::Property,
            location: "shared/src/lib.fe:1".into(),
            targets: vec!["evm".into(), "native".into()],
            assumed: Vec::new(),
            notes: Vec::new(),
            status,
        }
    }

    #[test]
    fn renders_each_status() {
        let mut refuted = claim(
            "halves_stay_small",
            ClaimStatus::Refuted(Box::new(Refuted {
                failure: "returns false".into(),
                counterexample: Counterexample::Args {
                    args: vec![Arg {
                        name: "x".into(),
                        value: "250".into(),
                    }],
                },
                refuted_on: vec!["evm".into()],
                replay: vec![
                    Replay {
                        target: "evm".into(),
                        outcome: "returned false".into(),
                    },
                    Replay {
                        target: "native".into(),
                        outcome: "returned false".into(),
                    },
                ],
                saved_to: Some("regressions/core/halves_stay_small.json".into()),
            })),
        );
        refuted.assumed.push("`x` takes any `u64`".into());
        let report = ProveReport {
            targets: vec!["evm".into(), "native".into()],
            claims: vec![
                claim("bounded", ClaimStatus::Proved),
                refuted,
                claim(
                    "Counter",
                    ClaimStatus::Unknown {
                        reason: "no counterexample within 4 steps".into(),
                    },
                ),
            ],
        };
        let text = render(&report);
        assert!(text.contains("PROVED      core::bounded"), "{text}");
        assert!(
            text.contains("REFUTED     core::halves_stay_small  returns false"),
            "{text}"
        );
        assert!(text.contains("            x = 250\n"), "{text}");
        assert!(
            text.contains("evm returned false · native returned false"),
            "{text}"
        );
        assert!(
            text.contains("saved to regressions/core/halves_stay_small.json"),
            "{text}"
        );
        assert!(
            text.contains("assumed\n  core::halves_stay_small: `x` takes any `u64`"),
            "{text}"
        );
        assert!(
            text.ends_with("1 proved, 1 refuted, 1 unknown (evm, native)\n"),
            "{text}"
        );
        assert!(!report.succeeded(false));
    }
}
