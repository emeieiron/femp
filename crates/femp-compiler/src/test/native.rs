//! Native tests: every selected test is compiled into one object, linked
//! with a launcher that calls the test its first argument names, and run in
//! a process of its own.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use codegen::NativeTestEntry;
use hir::hir_def::HirIngot;

use super::process::{self, quoted};
use super::{Outcome, TestOptions, failed, passed, revert_mismatch};
use crate::error::CompileError;
use crate::guard::{generate, opt_level};
use crate::native::{link, scratch_dir};
use crate::session::Session;

/// The launcher's exit status after it records a test's revert data.
const REVERT_STATUS: i32 = 101;

pub(super) fn run(
    session: &Session,
    options: &TestOptions<'_>,
) -> Result<Vec<Outcome>, CompileError> {
    if !cfg!(any(
        all(target_arch = "x86_64", target_os = "linux"),
        all(target_arch = "aarch64", target_os = "macos")
    )) {
        return Err(CompileError::Failed(
            "native tests run on x86-64 Linux and AArch64 macOS only".to_string(),
        ));
    }
    let db = &session.db;
    let mut modules = session.ingot().all_modules(db).to_vec();
    modules.sort_by(|left, right| left.name(db).cmp(&right.name(db)));
    let level = opt_level(options.optimize);
    let suite = generate("native", || {
        codegen::emit_test_suite_native(db, &modules, level, options.filter, false, false)
    })?;
    let Some(suite) = suite else {
        return Ok(Vec::new());
    };

    let scratch = scratch_dir("femp-native-test-")?;
    let dir = scratch.path();
    let object = dir.join("tests.o");
    let launcher = dir.join("launcher.c");
    let executable = dir.join("tests");
    write(&object, &suite.object)?;
    write(&launcher, launcher_source(&suite.tests).as_bytes())?;
    link(&[&launcher, &object], &executable)?;

    Ok(suite
        .tests
        .iter()
        .enumerate()
        .map(|(index, test)| run_case(&executable, dir, index, test))
        .collect())
}

fn write(path: &Path, contents: &[u8]) -> Result<(), CompileError> {
    fs::write(path, contents)
        .map_err(|error| CompileError::Failed(format!("cannot write {}: {error}", path.display())))
}

// A revert ends the test in `__fe_revert`, which records the revert data in
// the file the runner names and exits with `REVERT_STATUS`.
fn launcher_source(tests: &[NativeTestEntry]) -> String {
    let mut source = format!(
        r#"#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
static const char *revert_path;
void __fe_revert(const unsigned char *data, int64_t len) {{
fflush(NULL);
FILE *file = fopen(revert_path, "wb");
if (!file || fwrite(data, 1, (size_t)len, file) != (size_t)len || fclose(file) != 0) abort();
_exit({REVERT_STATUS});
}}
"#
    );
    for test in tests {
        let _ = writeln!(source, "extern void {}(void);", test.symbol);
    }
    source.push_str(
        "int main(int argc, char **argv) {\nif (argc != 3) return 2;\nrevert_path = argv[2];\n",
    );
    for (index, test) in tests.iter().enumerate() {
        let _ = writeln!(
            source,
            "if (strcmp(argv[1], \"{index}\") == 0) {{ {}(); return 0; }}",
            test.symbol
        );
    }
    source.push_str("return 2;\n}\n");
    source
}

fn run_case(executable: &Path, dir: &Path, index: usize, test: &NativeTestEntry) -> Outcome {
    let started = Instant::now();
    let revert_path = dir.join(format!("revert-{index}"));
    let ran = process::run(
        Command::new(executable)
            .arg(index.to_string())
            .arg(&revert_path),
    );
    let message = match ran {
        Err(error) => Some(format!("failed to execute native test: {error}")),
        Ok(ran) => {
            let describe = |summary: String| {
                format!(
                    "{summary}\nstdout:\n{}\nstderr:\n{}",
                    quoted(&ran.stdout),
                    quoted(&ran.stderr)
                )
            };
            let failure = || {
                describe(format!(
                    "native test failed ({}){}",
                    ran.status,
                    ran.stopped
                        .as_ref()
                        .map(|reason| format!(": {reason}"))
                        .unwrap_or_default()
                ))
            };
            // Only the launcher's record makes an exit with the revert
            // status a revert.
            let revert = (ran.stopped.is_none() && ran.status.code() == Some(REVERT_STATUS))
                .then(|| fs::read(&revert_path).ok())
                .flatten();
            let succeeded = ran.status.success() && ran.stopped.is_none();
            match (revert, &test.expected_revert) {
                (Some(data), Some(expected)) => revert_mismatch(expected, &data),
                (Some(data), None) => {
                    Some(describe(format!("Test reverted: 0x{}", hex::encode(data))))
                }
                (None, Some(_)) if succeeded => {
                    Some("Expected test to revert, but it succeeded".to_string())
                }
                (None, Some(_)) => Some(format!(
                    "Expected test to revert, but it failed with: {}",
                    failure()
                )),
                (None, None) => (!succeeded).then(failure),
            }
        }
    };
    match message {
        None => passed(&test.name, started.elapsed()),
        Some(message) => failed(&test.name, message, started.elapsed()),
    }
}
