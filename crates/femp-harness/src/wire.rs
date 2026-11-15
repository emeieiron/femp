//! The line protocol between the host and a harness process.
//!
//! A request is one decimal word per line: the entry id, then the
//! arguments (`1`/`0` for booleans). A machine request continues with the
//! initializer's arguments, the number of steps, and for each step the
//! command index and its arguments.
//!
//! A property answers with its result on one line. A machine answers with
//! one record per state, the initial one included: a `@` line, then each
//! invariant and observation on its own line. A line starting with `!`
//! reports an abnormal end (`!abort <code>`, `!trap <message>`,
//! `!error <message>`); a process that dies instead reports it on stderr.

use femp_core::property::{
    Halt, Machine, Outcome, PanicKind, Record, Sequence, StepStatus, Trace, Trap, Ty, Value,
};
use femp_core::{Error, Result};

use crate::session::Reply;

pub const RECORD_MARK: &str = "@";
const STATUS: char = '!';

pub fn word(value: &Value) -> String {
    match value.as_bool() {
        Some(flag) => (flag as u8).to_string(),
        None => value.to_string(),
    }
}

pub fn property_request(id: usize, args: &[Value]) -> String {
    let mut request = format!("{id}\n");
    for arg in args {
        request.push_str(&word(arg));
        request.push('\n');
    }
    request
}

pub fn sequence_request(id: usize, sequence: &Sequence) -> String {
    let mut request = property_request(id, &sequence.init);
    request.push_str(&format!("{}\n", sequence.steps.len()));
    for step in &sequence.steps {
        request.push_str(&property_request(step.command, &step.args));
    }
    request
}

/// The number of lines a complete reply to `sequence` has.
pub fn sequence_reply_len(machine: &Machine, sequence: &Sequence) -> usize {
    (sequence.steps.len() + 1) * (1 + machine.record_len())
}

pub fn is_status(line: &str) -> bool {
    line.starts_with(STATUS)
}

pub fn status(line: &str) -> Outcome {
    let body = line.trim_start_matches(STATUS);
    let (kind, detail) = body.split_once(' ').unwrap_or((body, ""));
    match kind {
        "abort" => match detail.trim().parse() {
            Ok(code) => Outcome::Aborted(code),
            Err(_) => Outcome::Trapped(Trap::Crashed(format!("malformed status `{line}`"))),
        },
        "trap" if detail.contains("unreachable") => Outcome::Panicked(PanicKind::Unknown),
        "trap" => Outcome::Trapped(Trap::Halted(detail.to_string())),
        _ => Outcome::Trapped(Trap::Crashed(detail.to_string())),
    }
}

/// Reads a value line. Signed values may also arrive as the 64-bit pattern
/// a WebAssembly host word carries.
pub fn decode_value(ty: Ty, text: &str) -> Option<Value> {
    Value::parse(ty, text).or_else(|| {
        let word: u64 = text.trim().parse().ok()?;
        (ty.bits() <= 64)
            .then(|| Value::from_u64_bits(ty, word))
            .flatten()
    })
}

/// How a process that died while answering ended.
pub fn ended(stderr: &str, status: &str, signal: Option<i32>) -> Outcome {
    Outcome::from_panic_output(stderr).unwrap_or_else(|| {
        let reason = match signal {
            Some(signal) => format!("signal {signal}"),
            None => status.to_string(),
        };
        Outcome::Trapped(Trap::Crashed(reason))
    })
}

pub fn property_outcome(result: Ty, reply: Reply) -> Outcome {
    match reply {
        Reply::Lines(lines) => match lines.first() {
            Some(line) if is_status(line) => status(line),
            Some(line) => decode_value(result, line).map_or_else(
                || Outcome::Trapped(Trap::Crashed(format!("unexpected output `{line}`"))),
                Outcome::Returned,
            ),
            None => Outcome::Trapped(Trap::Crashed("no output".to_string())),
        },
        Reply::Ended {
            stderr,
            status,
            signal,
            ..
        } => ended(&stderr, &status, signal),
        Reply::TimedOut { .. } => Outcome::Trapped(Trap::Timeout),
    }
}

/// Rebuilds the trace of a machine run from a reply.
pub fn trace(machine: &Machine, sequence: &Sequence, reply: Reply) -> Result<Trace> {
    let (lines, end) = match reply {
        Reply::Lines(lines) => (lines, None),
        Reply::Ended {
            lines,
            stderr,
            status,
            signal,
        } => (lines, Some(ended(&stderr, &status, signal))),
        Reply::TimedOut { lines } => (lines, Some(Outcome::Trapped(Trap::Timeout))),
    };
    let mut records = Vec::new();
    let mut halt = None;
    let mut rest = lines.iter().map(String::as_str);
    let expected = sequence.steps.len() + 1;
    while records.len() < expected {
        match rest.next() {
            Some(RECORD_MARK) => match read_record(machine, &mut rest)? {
                Ok(record) => records.push(record),
                Err(outcome) => {
                    halt = Some(outcome);
                    break;
                }
            },
            Some(line) if is_status(line) => {
                halt = Some(status(line));
                break;
            }
            Some(line) => {
                return Err(Error::Toolchain(format!(
                    "the harness answered `{line}` where a record was expected"
                )));
            }
            None => {
                halt = Some(end.clone().unwrap_or_else(|| {
                    Outcome::Trapped(Trap::Crashed("the reply ended early".to_string()))
                }));
                break;
            }
        }
    }
    Ok(Trace {
        halt: halt.map(|outcome| Halt {
            step: records.len(),
            outcome,
        }),
        records,
    })
}

// A record, or the outcome that interrupted it.
fn read_record<'a>(
    machine: &Machine,
    lines: &mut impl Iterator<Item = &'a str>,
) -> Result<std::result::Result<Record, Outcome>> {
    let mut invariants = Vec::new();
    let mut observations = Vec::new();
    let types = machine
        .invariants
        .iter()
        .map(|_| Ty::Bool)
        .chain(machine.observations.iter().map(|o| o.ty));
    for (index, ty) in types.enumerate() {
        let Some(line) = lines.next() else {
            return Ok(Err(Outcome::Trapped(Trap::Crashed(
                "the reply ended inside a record".to_string(),
            ))));
        };
        if is_status(line) {
            return Ok(Err(status(line)));
        }
        let value = decode_value(ty, line).ok_or_else(|| {
            Error::Toolchain(format!("the harness answered `{line}` for a `{ty}`"))
        })?;
        if index < machine.invariants.len() {
            invariants.push(value.as_bool().unwrap_or(false));
        } else {
            observations.push(value);
        }
    }
    Ok(Ok(Record {
        status: StepStatus::Done,
        invariants,
        observations,
    }))
}

#[cfg(test)]
mod tests {
    use femp_core::property::{Command, Location, Method, Observation, Step};

    use super::*;

    fn value(ty: Ty, text: &str) -> Value {
        Value::parse(ty, text).unwrap()
    }

    fn method(name: &str) -> Method {
        Method {
            name: name.into(),
            params: Vec::new(),
            effects: Vec::new(),
            location: Location {
                file: "m.fe".into(),
                line: 1,
            },
        }
    }

    fn machine() -> Machine {
        Machine {
            name: "M".into(),
            path: Vec::new(),
            init: method("new"),
            commands: vec![Command {
                method: method("step"),
                weight: 1,
            }],
            invariants: vec![method("ok")],
            observations: vec![Observation {
                method: method("value"),
                ty: Ty::Int(64),
            }],
            steps: "1..=4".into(),
            runs: None,
            clock: None,
            location: Location {
                file: "m.fe".into(),
                line: 1,
            },
        }
    }

    fn sequence(steps: usize) -> Sequence {
        Sequence {
            init: vec![value(Ty::Uint(8), "3")],
            steps: (0..steps)
                .map(|_| Step {
                    command: 0,
                    args: vec![value(Ty::Bool, "true")],
                    advance: 0,
                })
                .collect(),
        }
    }

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn encodes_requests() {
        assert_eq!(
            property_request(2, &[value(Ty::Bool, "true"), value(Ty::Int(8), "-4")]),
            "2\n1\n-4\n"
        );
        assert_eq!(sequence_request(5, &sequence(2)), "5\n3\n2\n0\n1\n0\n1\n");
        assert_eq!(sequence_reply_len(&machine(), &sequence(2)), 9);
    }

    #[test]
    fn decodes_values_and_statuses() {
        assert_eq!(
            decode_value(Ty::Int(64), "-5"),
            Some(value(Ty::Int(64), "-5"))
        );
        assert_eq!(
            decode_value(Ty::Int(8), "18446744073709551615"),
            Some(value(Ty::Int(8), "-1"))
        );
        assert_eq!(decode_value(Ty::Bool, "true"), Some(Value::boolean(true)));
        assert_eq!(status("!abort 6"), Outcome::Aborted(6));
        assert_eq!(
            status("!trap RuntimeError: unreachable"),
            Outcome::Panicked(PanicKind::Unknown)
        );
        assert!(matches!(
            status("!trap stack overflow"),
            Outcome::Trapped(Trap::Halted(_))
        ));
    }

    #[test]
    fn maps_replies_to_outcomes() {
        assert_eq!(
            property_outcome(Ty::Bool, Reply::Lines(lines(&["false"]))),
            Outcome::Returned(Value::boolean(false))
        );
        let crashed = Reply::Ended {
            lines: Vec::new(),
            status: "signal: 6".into(),
            signal: Some(6),
            stderr: "panicked: division by zero\n".into(),
        };
        assert_eq!(
            property_outcome(Ty::Bool, crashed),
            Outcome::Panicked(PanicKind::DivisionByZero)
        );
        assert_eq!(
            property_outcome(Ty::Bool, Reply::TimedOut { lines: Vec::new() }),
            Outcome::Trapped(Trap::Timeout)
        );
    }

    #[test]
    fn rebuilds_traces() {
        let complete = lines(&["@", "true", "3", "@", "true", "-4", "@", "false", "5"]);
        let trace = trace(&machine(), &sequence(2), Reply::Lines(complete)).unwrap();
        assert_eq!(trace.records.len(), 3);
        assert_eq!(trace.records[2].invariants, [false]);
        assert_eq!(trace.records[1].observations, [value(Ty::Int(64), "-4")]);
        assert_eq!(trace.halt, None);

        let crashed = Reply::Ended {
            lines: lines(&["@", "true", "3"]),
            status: "signal: 6".into(),
            signal: Some(6),
            stderr: "panicked: arithmetic overflow".into(),
        };
        let trace = super::trace(&machine(), &sequence(2), crashed).unwrap();
        assert_eq!(trace.records.len(), 1);
        assert_eq!(
            trace.halt,
            Some(Halt {
                step: 1,
                outcome: Outcome::Panicked(PanicKind::Overflow)
            })
        );

        let trapped = lines(&["@", "true", "3", "@", "!trap RuntimeError: unreachable"]);
        let trace = super::trace(&machine(), &sequence(2), Reply::Lines(trapped)).unwrap();
        assert_eq!(trace.halt.unwrap().step, 1);

        let garbage = lines(&["?"]);
        assert!(super::trace(&machine(), &sequence(1), Reply::Lines(garbage)).is_err());
    }
}
