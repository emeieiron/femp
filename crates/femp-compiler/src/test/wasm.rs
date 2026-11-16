//! WebAssembly tests: each test is a module of its own, run by Node.js with
//! femp's host runtime in a process of its own.

use std::fmt::Write as _;
use std::fs;
use std::process::Command;
use std::time::Instant;

use hir::hir_def::HirIngot;

use super::process::{self, QUOTED_LIMIT};
use super::{Outcome, TestOptions, WasmRunner, failed, passed, reason};
use crate::error::CompileError;
use crate::guard::{opt_level, panic_message};
use crate::native::scratch_dir;
use crate::session::Session;

const RUNNER_SCRIPT: &str = include_str!("run_test.mjs");
const RUNNER_FILE: &str = "run_test.mjs";
/// The name the runner imports the host runtime by.
const HOST_FILE: &str = "host.js";

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
    let mut outcomes = Vec::new();
    for (index, test) in tests.iter().enumerate() {
        let started = Instant::now();
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            codegen::emit_test_wasm(db, test, level, false)
                .map_err(|error| reason(&error.to_string()))
        }))
        .unwrap_or_else(|payload| {
            Err(format!(
                "the WebAssembly backend panicked: {}",
                panic_message(payload.as_ref())
            ))
        });
        let module = match built {
            Ok(module) => module,
            Err(error) => {
                outcomes.push(failed(
                    &test.name,
                    format!("cannot build `{}` for WebAssembly: {error}", test.name),
                    started.elapsed(),
                ));
                continue;
            }
        };
        let file = format!("{index}.wasm");
        fs::write(dir.join(&file), &module.wasm)
            .map_err(|error| CompileError::Failed(format!("cannot write {file}: {error}")))?;
        let message = execute(runner, dir, &file, &module.entry);
        outcomes.push(match message {
            None => passed(&test.name, started.elapsed()),
            Some(message) => failed(&test.name, message, started.elapsed()),
        });
    }
    Ok(outcomes)
}

fn execute(
    runner: &WasmRunner<'_>,
    dir: &std::path::Path,
    file: &str,
    entry: &str,
) -> Option<String> {
    let ran = process::run(
        Command::new(runner.node)
            .current_dir(dir)
            .arg(RUNNER_FILE)
            .arg(file)
            .arg(entry),
    );
    let ran = match ran {
        Ok(ran) => ran,
        Err(error) => return Some(format!("failed to run the test in Node.js: {error}")),
    };
    if ran.status.success() && ran.stopped.is_none() {
        return None;
    }
    let preview = |bytes: &[u8]| {
        String::from_utf8_lossy(&bytes[..bytes.len().min(QUOTED_LIMIT)])
            .trim_end()
            .to_string()
    };
    let mut message = match &ran.stopped {
        Some(reason) => format!("wasm test stopped: {reason}"),
        None if ran.stderr.is_empty() => format!("wasm test failed ({})", ran.status),
        None => format!("wasm test failed: {}", preview(&ran.stderr)),
    };
    if !ran.stdout.is_empty() {
        let _ = write!(message, "\n    logged:\n{}", preview(&ran.stdout));
    }
    Some(message)
}
