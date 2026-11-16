//! Runs a compiled test in its own process, with a time limit and a cap on
//! the output it keeps.

use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const TIME_LIMIT: Duration = Duration::from_secs(60);
pub const OUTPUT_LIMIT: usize = 1024 * 1024;
/// How much of a failed test's output its message quotes.
pub const QUOTED_LIMIT: usize = 4096;

pub struct Ran {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Why the process was stopped before it ended, if it was.
    pub stopped: Option<String>,
}

pub fn run(command: &mut Command) -> io::Result<Ran> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let overflowed = Arc::new(AtomicBool::new(false));
    let stdout = capture(child.stdout.take(), &overflowed);
    let stderr = capture(child.stderr.take(), &overflowed);
    let (status, stopped) = wait(&mut child, &overflowed)?;
    Ok(Ran {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
        stopped,
    })
}

fn wait(child: &mut Child, overflowed: &AtomicBool) -> io::Result<(ExitStatus, Option<String>)> {
    let deadline = Instant::now() + TIME_LIMIT;
    let mut pause = Duration::from_millis(1);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status, None));
        }
        let stopped = if overflowed.load(Ordering::Relaxed) {
            Some(format!(
                "the test wrote more than {} KiB of output",
                OUTPUT_LIMIT / 1024
            ))
        } else if Instant::now() >= deadline {
            Some(format!(
                "the test exceeded the time limit of {} seconds",
                TIME_LIMIT.as_secs()
            ))
        } else {
            None
        };
        if stopped.is_some() {
            let _ = child.kill();
            return Ok((child.wait()?, stopped));
        }
        thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(10));
    }
}

// Keeps reading past the limit, so that the child never blocks on a full
// pipe, and flags the overflow for `wait` to stop it.
fn capture(
    pipe: Option<impl Read + Send + 'static>,
    overflowed: &Arc<AtomicBool>,
) -> JoinHandle<Vec<u8>> {
    let overflowed = Arc::clone(overflowed);
    thread::spawn(move || {
        let mut kept = Vec::new();
        let Some(mut pipe) = pipe else {
            return kept;
        };
        let mut buffer = [0u8; 8192];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let room = OUTPUT_LIMIT.saturating_sub(kept.len());
                    kept.extend_from_slice(&buffer[..read.min(room)]);
                    if read > room {
                        overflowed.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
        kept
    })
}

/// The start of `bytes` as text, for a failure message.
pub fn quoted(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(QUOTED_LIMIT)]);
    if bytes.len() > QUOTED_LIMIT {
        format!("{text}\n[output truncated]")
    } else {
        text.into_owned()
    }
}
