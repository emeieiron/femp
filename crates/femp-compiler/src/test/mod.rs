//! Runs an ingot's `#[test]` functions on the session's target, as
//! `fe test` does: on the EVM in an in-process interpreter, natively as one
//! process per test, and in WebAssembly via Node.js.

mod evm;
mod native;
mod process;
mod wasm;

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use codegen::ExpectedRevert;
use femp_core::environment::OptLevel;
use hir::hir_def::HirIngot;

use crate::error::CompileError;
use crate::session::Session;
use crate::target::Target;

pub struct TestOptions<'a> {
    pub optimize: OptLevel,
    /// Only the tests whose name contains this.
    pub filter: Option<&'a str>,
    /// What runs WebAssembly tests; without it they cannot run.
    pub wasm: Option<WasmRunner<'a>>,
}

/// The Node.js that runs WebAssembly tests, and the host runtime the test
/// modules are instantiated with, which exports `instantiate`.
pub struct WasmRunner<'a> {
    pub node: &'a Path,
    pub host_script: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseResult {
    pub name: String,
    pub passed: bool,
    /// Why the test failed.
    pub message: Option<String>,
}

/// Each test's result, and the report the compiler's runner prints for
/// them.
pub struct TestReport {
    pub cases: Vec<CaseResult>,
    pub log: String,
}

struct Outcome {
    case: CaseResult,
    elapsed: Duration,
}

impl Session {
    pub fn test(&self, options: &TestOptions<'_>) -> Result<TestReport, CompileError> {
        let ingot = self.ingot();
        if !ingot
            .all_funcs(&self.db)
            .iter()
            .any(|func| func.is_test(&self.db))
        {
            return Ok(TestReport {
                cases: Vec::new(),
                log: String::new(),
            });
        }
        let outcomes = match self.target {
            Target::Evm => evm::run(self, options)?,
            Target::Native => native::run(self, options)?,
            Target::Wasm => wasm::run(self, options)?,
        };
        let mut log = String::new();
        for outcome in &outcomes {
            write_status(&mut log, outcome);
        }
        Ok(TestReport {
            cases: outcomes.into_iter().map(|outcome| outcome.case).collect(),
            log,
        })
    }
}

fn write_status(out: &mut String, outcome: &Outcome) {
    let label = if outcome.case.passed {
        "PASS "
    } else {
        "FAIL "
    };
    let _ = writeln!(
        out,
        "{label} [{}s] {}",
        seconds(outcome.elapsed),
        outcome.case.name
    );
    if let Some(message) = &outcome.case.message {
        let _ = writeln!(out, "    {message}");
    }
}

// Six columns, with as many decimals as fit, at most four.
fn seconds(elapsed: Duration) -> String {
    const WIDTH: usize = 6;
    let seconds = elapsed.as_secs_f64();
    let whole = if seconds >= 1.0 {
        seconds.log10().floor() as usize + 1
    } else {
        1
    };
    let decimals = WIDTH.saturating_sub(whole + 1).min(4);
    let rendered = if decimals == 0 {
        format!("{seconds:.0}")
    } else {
        format!("{seconds:.decimals$}")
    };
    format!("{rendered:>WIDTH$}")
}

fn passed(name: &str, elapsed: Duration) -> Outcome {
    Outcome {
        case: CaseResult {
            name: name.to_string(),
            passed: true,
            message: None,
        },
        elapsed,
    }
}

fn failed(name: &str, message: String, elapsed: Duration) -> Outcome {
    Outcome {
        case: CaseResult {
            name: name.to_string(),
            passed: false,
            message: Some(message),
        },
        elapsed,
    }
}

/// How revert data differs from what a `should_revert` test expects, if it
/// does.
fn revert_mismatch(expected: &ExpectedRevert, data: &[u8]) -> Option<String> {
    match expected {
        ExpectedRevert::Any => None,
        ExpectedRevert::Selector(selector) => (data.get(..4) != Some(&selector[..])).then(|| {
            format!(
                "Expected revert with selector 0x{}, but got revert data: 0x{}",
                hex::encode(selector),
                hex::encode(data),
            )
        }),
        ExpectedRevert::PanicCode(expected) => (data != &expected[..]).then(|| {
            format!(
                "Expected revert data 0x{}, but got: 0x{}",
                hex::encode(expected),
                hex::encode(data),
            )
        }),
    }
}

/// The part of a lowering error that says what is wrong; the compiler
/// appends the function's runtime IR after a blank line.
fn reason(message: &str) -> String {
    message
        .split("\n\nrMIR context:")
        .next()
        .unwrap_or(message)
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_elapsed_time_in_six_columns() {
        assert_eq!(seconds(Duration::from_micros(400)), "0.0004");
        assert_eq!(seconds(Duration::from_millis(12_345)), "12.345");
        assert_eq!(seconds(Duration::from_secs(1234)), "1234.0");
        assert_eq!(seconds(Duration::from_secs(123_456)), "123456");
    }

    #[test]
    fn compares_revert_data_with_the_expectation() {
        assert_eq!(revert_mismatch(&ExpectedRevert::Any, b"x"), None);
        let selector = ExpectedRevert::Selector([1, 2, 3, 4]);
        assert_eq!(revert_mismatch(&selector, &[1, 2, 3, 4, 5]), None);
        assert!(revert_mismatch(&selector, &[1, 2]).is_some());
    }
}
