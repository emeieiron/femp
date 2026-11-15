//! A long-lived child process that answers requests one line at a time.
//! When the child dies, say because a case panicked, the session reports how
//! it ended and starts a new child for the next request.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use femp_core::{Error, Result};

const STDERR_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// Every expected line arrived, or a line that ends the reply early.
    Lines(Vec<String>),
    /// The child exited before answering in full.
    Ended {
        lines: Vec<String>,
        status: String,
        signal: Option<i32>,
        stderr: String,
    },
    /// The child did not answer in time and was stopped.
    TimedOut { lines: Vec<String> },
}

struct Running {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    stderr: Arc<Mutex<String>>,
    stderr_reader: Option<JoinHandle<()>>,
}

pub struct Session {
    program: PathBuf,
    args: Vec<OsString>,
    timeout: Duration,
    running: Option<Running>,
}

impl Session {
    pub fn new(program: impl Into<PathBuf>, args: Vec<OsString>, timeout: Duration) -> Self {
        Self {
            program: program.into(),
            args,
            timeout,
            running: None,
        }
    }

    /// Sends `request` and collects up to `expected` lines, stopping early
    /// at a line for which `ends` holds.
    pub fn exchange(
        &mut self,
        request: &str,
        expected: usize,
        ends: impl Fn(&str) -> bool,
    ) -> Result<Reply> {
        if self.running.is_none() {
            self.running = Some(self.spawn()?);
        }
        let running = self.running.as_mut().expect("the child was just started");
        // A child that already died closes its stdin; its exit is reported
        // below when its output ends.
        let _ = running
            .stdin
            .write_all(request.as_bytes())
            .and_then(|()| running.stdin.flush());

        let deadline = Instant::now() + self.timeout;
        let mut lines = Vec::new();
        while lines.len() < expected {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match running.lines.recv_timeout(remaining) {
                Ok(line) => {
                    let done = ends(&line);
                    lines.push(line);
                    if done {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.stop();
                    return Ok(Reply::TimedOut { lines });
                }
                Err(RecvTimeoutError::Disconnected) => return self.ended(lines),
            }
        }
        Ok(Reply::Lines(lines))
    }

    fn spawn(&self) -> Result<Running> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                Error::Toolchain(format!(
                    "could not start `{}`: {error}",
                    self.program.display()
                ))
            })?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr is piped");

        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let stderr = Arc::new(Mutex::new(String::new()));
        let collected = Arc::clone(&stderr);
        let stderr_reader = thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            while let Ok(read) = stderr_pipe.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let mut text = collected
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                text.push_str(&String::from_utf8_lossy(&buffer[..read]));
                if text.len() > STDERR_LIMIT {
                    let cut = text.len() - STDERR_LIMIT / 2;
                    let cut = (cut..text.len())
                        .find(|i| text.is_char_boundary(*i))
                        .unwrap_or(cut);
                    text.drain(..cut);
                }
            }
        });
        Ok(Running {
            child,
            stdin,
            lines,
            stderr,
            stderr_reader: Some(stderr_reader),
        })
    }

    fn ended(&mut self, lines: Vec<String>) -> Result<Reply> {
        let mut running = self.running.take().expect("a child is running");
        let status = running.child.wait().map_err(|error| {
            Error::Toolchain(format!(
                "could not wait for `{}`: {error}",
                self.program.display()
            ))
        })?;
        if let Some(reader) = running.stderr_reader.take() {
            let _ = reader.join();
        }
        let stderr = running
            .stderr
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        Ok(Reply::Ended {
            lines,
            status: status.to_string(),
            signal: signal(&status),
            stderr,
        })
    }

    fn stop(&mut self) {
        if let Some(mut running) = self.running.take() {
            let _ = running.child.kill();
            let _ = running.child.wait();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(unix)]
fn signal(status: &ExitStatus) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(status)
}

#[cfg(not(unix))]
fn signal(_status: &ExitStatus) -> Option<i32> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    // Echoes each request line back doubled; `boom` aborts the process after
    // a panic message, and `hang` never answers.
    const SCRIPT: &str = r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    boom) echo "panicked: arithmetic overflow" >&2; kill -ABRT $$ ;;
    hang) exec sleep 30 ;;
    *) echo "$line$line"; echo "!end" ;;
  esac
done
"#;

    fn session(timeout: Duration) -> (tempfile::TempDir, Session) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("child.sh");
        fs::write(&script, SCRIPT).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        (dir, Session::new(script, Vec::new(), timeout))
    }

    #[test]
    fn exchanges_lines_and_stops_at_the_end_marker() {
        let (_dir, mut session) = session(Duration::from_secs(5));
        let reply = session
            .exchange("ab\n", 5, |line| line.starts_with('!'))
            .unwrap();
        assert_eq!(reply, Reply::Lines(vec!["abab".into(), "!end".into()]));
        let reply = session.exchange("c\n", 1, |_| false).unwrap();
        assert_eq!(reply, Reply::Lines(vec!["cc".into()]));
    }

    #[test]
    fn reports_a_crash_and_restarts() {
        let (_dir, mut session) = session(Duration::from_secs(5));
        let Reply::Ended {
            lines,
            signal,
            stderr,
            ..
        } = session.exchange("boom\n", 1, |_| false).unwrap()
        else {
            panic!("expected the child to end");
        };
        assert!(lines.is_empty());
        assert_eq!(signal, Some(6));
        assert!(stderr.contains("panicked: arithmetic overflow"));
        let reply = session.exchange("x\n", 1, |_| false).unwrap();
        assert_eq!(reply, Reply::Lines(vec!["xx".into()]));
    }

    #[test]
    fn stops_a_child_that_hangs() {
        let (_dir, mut session) = session(Duration::from_secs(1));
        let reply = session.exchange("hang\n", 1, |_| false).unwrap();
        assert_eq!(reply, Reply::TimedOut { lines: Vec::new() });
        let reply = session.exchange("y\n", 1, |_| false).unwrap();
        assert_eq!(reply, Reply::Lines(vec!["yy".into()]));
    }
}
