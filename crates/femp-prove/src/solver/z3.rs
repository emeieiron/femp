//! A `z3 -in -smt2` process. Commands go to its standard input; after each
//! batch an `echo` marks the end of the answer, so errors are seen at once.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use super::sexp::{self, Sexp};
use super::{Answer, Solver};

const MARK: &str = "femp-done";

pub struct Z3 {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Z3 {
    pub fn start(program: &Path) -> Result<Self, String> {
        let mut child = Command::new(program)
            .args(["-in", "-smt2"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("could not start `{}`: {error}", program.display()))?;
        let input = child.stdin.take().ok_or("the solver has no input")?;
        let output = BufReader::new(child.stdout.take().ok_or("the solver has no output")?);
        let mut solver = Self {
            child,
            input,
            output,
        };
        solver.send("(set-option :print-success false)\n(set-option :random-seed 0)\n(set-option :smt.random_seed 0)")?;
        Ok(solver)
    }

    fn write(&mut self, text: &str) -> Result<(), String> {
        self.input
            .write_all(text.as_bytes())
            .and_then(|()| self.input.write_all(b"\n"))
            .and_then(|()| self.input.flush())
            .map_err(|error| format!("the solver stopped: {error}"))
    }

    fn read_line(&mut self) -> Result<String, String> {
        let mut line = String::new();
        let read = self
            .output
            .read_line(&mut line)
            .map_err(|error| format!("the solver stopped: {error}"))?;
        if read == 0 {
            return Err("the solver stopped unexpectedly".to_string());
        }
        Ok(line.trim_end().to_string())
    }

    /// Reads one complete answer, which may span several lines.
    fn read_answer(&mut self) -> Result<String, String> {
        let mut answer = String::new();
        loop {
            let line = self.read_line()?;
            if answer.is_empty() && line.trim().is_empty() {
                continue;
            }
            if !answer.is_empty() {
                answer.push('\n');
            }
            answer.push_str(&line);
            if !answer.starts_with('(') || sexp::complete(&answer) {
                return Ok(answer);
            }
        }
    }
}

impl Solver for Z3 {
    fn send(&mut self, commands: &str) -> Result<(), String> {
        self.write(commands)?;
        self.write(&format!("(echo \"{MARK}\")"))?;
        let mut errors = Vec::new();
        loop {
            let answer = self.read_answer()?;
            if answer == MARK {
                break;
            }
            if answer.starts_with("(error") {
                errors.push(answer);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "the solver rejected the encoding: {}",
                errors.join("; ")
            ))
        }
    }

    fn check(&mut self, timeout: Duration) -> Result<Answer, String> {
        let millis = timeout.as_millis().max(1);
        self.write(&format!("(set-option :timeout {millis})\n(check-sat)"))?;
        let answer = self.read_answer()?;
        match answer.as_str() {
            "sat" => Ok(Answer::Sat),
            "unsat" => Ok(Answer::Unsat),
            "unknown" => {
                self.write("(get-info :reason-unknown)")?;
                let reason = self.read_answer()?;
                let reason = sexp::parse(&reason)
                    .ok()
                    .and_then(|parsed| {
                        parsed
                            .list()
                            .and_then(|items| items.get(1))
                            .and_then(Sexp::atom)
                            .map(|text| text.trim_matches('"').to_string())
                    })
                    .unwrap_or(reason);
                Ok(Answer::Unknown(reason))
            }
            other => Err(format!("unexpected answer from the solver: {other}")),
        }
    }

    fn values(&mut self, terms: &[String]) -> Result<Vec<Sexp>, String> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        self.write(&format!("(get-value ({}))", terms.join(" ")))?;
        let answer = self.read_answer()?;
        if answer.starts_with("(error") {
            return Err(format!("the solver could not report a model: {answer}"));
        }
        let parsed = sexp::parse(&answer)?;
        let pairs = parsed
            .list()
            .ok_or_else(|| format!("unexpected model from the solver: {answer}"))?;
        pairs
            .iter()
            .map(|pair| {
                pair.list()
                    .and_then(|items| items.get(1))
                    .cloned()
                    .ok_or_else(|| format!("unexpected model from the solver: {answer}"))
            })
            .collect()
    }
}

impl Drop for Z3 {
    fn drop(&mut self) {
        let _ = self.write("(exit)");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
