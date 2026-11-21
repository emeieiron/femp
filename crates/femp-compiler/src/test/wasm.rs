//! WebAssembly tests: selected tests of a suite share one compiled module
//! (bisecting to isolate any test the backend refuses), and each test runs in
//! its own Node.js process with femp's host runtime.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use codegen::ExpectedRevert;
use hir::hir_def::HirIngot;
use mir::TestFunc;

use super::process::{self, OUTPUT_LIMIT, QUOTED_LIMIT};
use super::{Outcome, TestOptions, WasmRunner, failed, passed, reason, revert_mismatch};
use crate::error::CompileError;
use crate::guard::{opt_level, panic_message};
use crate::native::scratch_dir;
use crate::session::Session;

const RUNNER_SCRIPT: &str = include_str!("run_test.mjs");
const RUNNER_FILE: &str = "run_test.mjs";
/// The name the runner imports the host runtime by.
const HOST_FILE: &str = "host.js";
/// The runner's exit status for a test that panicked with a code or reverted.
const REVERT_STATUS: i32 = 3;

pub(super) fn run(
    session: &Session,
    options: &TestOptions<'_>,
) -> Result<Vec<Outcome>, CompileError> {
    let runner = options.wasm.as_ref().ok_or_else(|| {
        CompileError::Failed("WebAssembly tests need Node.js to run them".to_string())
    })?;
    let db = &session.db;
    let mut modules = session.ingot().all_modules(db).to_vec();
    modules.sort_by(|left, right| left.name(db).cmp(&right.name(db)));
    let tests = mir::collect_test_funcs(db, &modules, options.filter);
    if tests.is_empty() {
        return Ok(Vec::new());
    }
    let scratch = scratch_dir("femp-wasm-test-")?;
    let dir = scratch.path();
    for (file, contents) in [
        (HOST_FILE, runner.host_script),
        (RUNNER_FILE, RUNNER_SCRIPT),
    ] {
        fs::write(dir.join(file), contents)
            .map_err(|error| CompileError::Failed(format!("cannot write {file}: {error}")))?;
    }

    let level = opt_level(options.optimize);
    let mut groups: VecDeque<&[TestFunc<'_>]> = VecDeque::from([tests.as_slice()]);
    let mut module_index = 0usize;
    let mut outcomes = Vec::new();
    while let Some(group) = groups.pop_front() {
        let started = Instant::now();
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            codegen::emit_test_wasm(db, group, level, false)
                .map_err(|error| reason(&error.to_string()))
        }))
        .unwrap_or_else(|payload| {
            Err(format!(
                "the WebAssembly backend panicked: {}",
                panic_message(payload.as_ref())
            ))
        });
        let module = match (built, group) {
            (Ok(module), _) => module,
            (Err(error), [test]) => {
                outcomes.push(failed(
                    &test.name,
                    format!("cannot build `{}` for WebAssembly: {error}", test.name),
                    started.elapsed(),
                ));
                continue;
            }
            (Err(_), _) => {
                let (first, second) = group.split_at(group.len() / 2);
                groups.push_front(second);
                groups.push_front(first);
                continue;
            }
        };
        let file = format!("{module_index}.wasm");
        module_index += 1;
        fs::write(dir.join(&file), &module.wasm)
            .map_err(|error| CompileError::Failed(format!("cannot write {file}: {error}")))?;
        for (test, (entry, expected_revert)) in group.iter().zip(module.entries) {
            let case_started = Instant::now();
            let message = execute(runner, dir, &file, &entry, expected_revert.as_ref());
            outcomes.push(match message {
                None => passed(&test.name, case_started.elapsed()),
                Some(message) => failed(&test.name, message, case_started.elapsed()),
            });
        }
    }
    Ok(outcomes)
}

fn execute(
    runner: &WasmRunner<'_>,
    dir: &Path,
    file: &str,
    entry: &str,
    expected_revert: Option<&ExpectedRevert>,
) -> Option<String> {
    let ran = process::run(
        Command::new(runner.node)
            .current_dir(dir)
            .arg(RUNNER_FILE)
            .arg(file)
            .arg(entry)
            .arg(OUTPUT_LIMIT.to_string()),
    );
    let ran = match ran {
        Ok(ran) => ran,
        Err(error) => return Some(format!("failed to run the test in Node.js: {error}")),
    };
    let preview = |bytes: &[u8]| {
        String::from_utf8_lossy(&bytes[..bytes.len().min(QUOTED_LIMIT)])
            .trim_end()
            .to_string()
    };
    let succeeded = ran.status.success() && ran.stopped.is_none();
    let revert = (ran.stopped.is_none() && ran.status.code() == Some(REVERT_STATUS))
        .then(|| {
            let line = String::from_utf8_lossy(&ran.stderr);
            hex::decode(line.trim().strip_prefix("revert")?.trim()).ok()
        })
        .flatten();
    let failure = || match &ran.stopped {
        Some(reason) => format!("wasm test stopped: {reason}"),
        None if ran.stderr.is_empty() => format!("wasm test failed ({})", ran.status),
        None => format!("wasm test failed: {}", preview(&ran.stderr)),
    };
    let message = match (revert, expected_revert) {
        (Some(data), Some(expected)) => revert_mismatch(expected, &data),
        (Some(data), None) => Some(format!(
            "Test reverted: 0x{}{}",
            hex::encode(&data),
            revert_reason(&data)
                .map(|reason| format!(" ({reason})"))
                .unwrap_or_default()
        )),
        (None, Some(_)) if succeeded => {
            Some("Expected test to revert, but it succeeded".to_string())
        }
        (None, Some(_)) => Some(format!(
            "Expected test to revert, but it failed with: {}",
            failure()
        )),
        (None, None) => (!succeeded).then(failure),
    };
    message.map(|mut message| {
        if !ran.stdout.is_empty() {
            let _ = write!(message, "\n    logged:\n{}", preview(&ran.stdout));
        }
        message
    })
}

/// What Solidity `Panic(uint256)` or `Error(string)` revert data says.
fn revert_reason(data: &[u8]) -> Option<String> {
    let (selector, body) = data.split_first_chunk::<4>()?;
    let word = |index: usize| {
        let word = body.get(32 * index..32 * index + 32)?;
        let (high, low) = word.split_at(24);
        high.iter()
            .all(|&byte| byte == 0)
            .then(|| u64::from_be_bytes(low.try_into().unwrap()))
    };
    match selector {
        [0x4e, 0x48, 0x7b, 0x71] if body.len() == 32 => Some(
            match word(0)? {
                0x00 => "generic panic",
                0x01 => "assertion failed",
                0x11 => "arithmetic overflow",
                0x12 => "division by zero",
                0x21 => "invalid enum value",
                0x22 => "invalid storage encoding",
                0x31 => "pop from an empty array",
                0x32 => "index out of bounds",
                0x41 => "out of memory",
                0x51 => "call to a zero-initialized function",
                _ => return None,
            }
            .to_string(),
        ),
        [0x08, 0xc3, 0x79, 0xa0] if word(0) == Some(32) => {
            let message = body.get(64..)?.get(..usize::try_from(word(1)?).ok()?)?;
            Some(String::from_utf8_lossy(message).into_owned())
        }
        _ => None,
    }
}
