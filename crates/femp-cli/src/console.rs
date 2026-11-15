use std::io::{IsTerminal, Write, stderr, stdout};
use std::path::Path;
use std::process;

const LABEL_WIDTH: usize = 10;
const GREEN: &str = "32";
const YELLOW: &str = "33";
const RED: &str = "31";
/// The status a process ends with when it is killed for writing to a closed pipe.
const EXIT_BROKEN_PIPE: i32 = 141;

// `println!` panics when stdout is closed early, as in `femp doctor | head`.
// Ending quietly instead matches what other command-line tools do.
pub fn line(text: impl AsRef<str>) {
    if writeln!(stdout(), "{}", text.as_ref()).is_err() {
        process::exit(EXIT_BROKEN_PIPE);
    }
}

pub fn step(label: &str, message: &str) {
    line(format!(
        "{} {message}",
        paint(label, GREEN, stdout().is_terminal())
    ));
}

pub fn warning(message: &str) {
    eprintln!(
        "{} {message}",
        paint("warning", YELLOW, stderr().is_terminal())
    );
}

pub fn error(message: &str) {
    eprintln!("{} {message}", paint("error", RED, stderr().is_terminal()));
}

pub fn detail(text: &str) {
    for row in text.trim_end().lines() {
        line(format!("    {row}"));
    }
}

pub fn relative(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn paint(label: &str, color: &str, is_terminal: bool) -> String {
    let padded = format!("{label:>LABEL_WIDTH$}");
    if is_terminal && std::env::var_os("NO_COLOR").is_none() {
        format!("\x1b[1;{color}m{padded}\x1b[0m")
    } else {
        padded
    }
}
