use std::fmt;

use super::value::Value;

const PANIC_SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];
const ERROR_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];
const PANIC_PREFIX: &str = "panicked";
const REVERT_PREFIX: &str = "reverted with 0x";

/// Why a computation stopped abnormally, in terms every backend can report.
/// `Unknown` is a panic whose reason the backend does not expose, such as a
/// WebAssembly trap.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PanicKind {
    Assertion,
    Overflow,
    DivisionByZero,
    Bounds,
    Memory,
    /// `core::panic()` and other panics that carry no reason.
    Explicit,
    Message(String),
    /// A Solidity panic code without a portable meaning.
    Code(u64),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Trap {
    OutOfGas,
    Timeout,
    /// The process running the case ended without reporting a result.
    Crashed(String),
    /// The execution engine stopped the case for another reason.
    Halted(String),
}

/// The normalized result of running one case on one target.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Outcome {
    Returned(Value),
    Panicked(PanicKind),
    /// EVM revert data that is not a panic encoding, such as a custom error.
    Reverted(Vec<u8>),
    /// A WebAssembly module ended the call with `abort(code)`.
    Aborted(u32),
    Trapped(Trap),
}

impl Outcome {
    pub fn is_abnormal(&self) -> bool {
        !matches!(self, Self::Returned(_))
    }

    /// Normalizes EVM revert data. Fe reverts with `Panic(uint256)` for
    /// checked failures, `Error(string)` for `assert!` with a message, and no
    /// data for `core::panic()`.
    pub fn from_revert(data: &[u8]) -> Self {
        if data.is_empty() {
            return Self::Panicked(PanicKind::Explicit);
        }
        if data.len() == 36
            && data[..4] == PANIC_SELECTOR
            && let Some(code) = small_word(&data[4..], 0)
        {
            return Self::Panicked(panic_code(code as u64));
        }
        if data.len() >= 4
            && data[..4] == ERROR_SELECTOR
            && let Some(message) = decode_error_string(&data[4..])
        {
            return Self::Panicked(message_kind(&message));
        }
        Self::Reverted(data.to_vec())
    }

    /// Normalizes what a native executable printed to standard error before
    /// it aborted. The last `panicked` line wins.
    pub fn from_panic_output(stderr: &str) -> Option<Self> {
        let line = stderr
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| line.starts_with(PANIC_PREFIX))?;
        let reason = line[PANIC_PREFIX.len()..].trim_start_matches(':').trim();
        if reason.is_empty() {
            return Some(Self::Panicked(PanicKind::Explicit));
        }
        if let Some(hex) = reason.strip_prefix(REVERT_PREFIX)
            && let Some(data) = decode_hex(hex)
        {
            return Some(Self::from_revert(&data));
        }
        Some(Self::Panicked(message_kind(reason)))
    }
}

fn panic_code(code: u64) -> PanicKind {
    match code {
        0x01 => PanicKind::Assertion,
        0x11 => PanicKind::Overflow,
        0x12 => PanicKind::DivisionByZero,
        0x32 => PanicKind::Bounds,
        0x41 => PanicKind::Memory,
        other => PanicKind::Code(other),
    }
}

fn message_kind(message: &str) -> PanicKind {
    match message {
        "assertion failed" => PanicKind::Assertion,
        "arithmetic overflow" => PanicKind::Overflow,
        "division by zero" => PanicKind::DivisionByZero,
        "index out of bounds" => PanicKind::Bounds,
        "out of memory" => PanicKind::Memory,
        other => PanicKind::Message(other.to_string()),
    }
}

// ABI encoding of a single `string`: offset word, length word, padded bytes.
fn decode_error_string(payload: &[u8]) -> Option<String> {
    let offset = small_word(payload, 0)?;
    let length = small_word(payload, offset)?;
    let start = offset.checked_add(32)?;
    let text = payload.get(start..start.checked_add(length)?)?;
    String::from_utf8(text.to_vec()).ok()
}

fn small_word(payload: &[u8], at: usize) -> Option<usize> {
    let bytes = payload.get(at..at.checked_add(32)?)?;
    let (high, low) = bytes.split_at(24);
    high.iter()
        .all(|byte| *byte == 0)
        .then(|| u64::from_be_bytes(low.try_into().unwrap_or_default()) as usize)
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(text.get(index..index + 2)?, 16).ok())
        .collect()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl fmt::Display for PanicKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Assertion => f.write_str("assertion failed"),
            Self::Overflow => f.write_str("arithmetic overflow"),
            Self::DivisionByZero => f.write_str("division by zero"),
            Self::Bounds => f.write_str("index out of bounds"),
            Self::Memory => f.write_str("out of memory"),
            Self::Explicit => f.write_str("explicit panic"),
            Self::Message(message) => write!(f, "{message:?}"),
            Self::Code(code) => write!(f, "panic code {code:#04x}"),
            Self::Unknown => f.write_str("reason not reported"),
        }
    }
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfGas => f.write_str("out of gas"),
            Self::Timeout => f.write_str("timed out"),
            Self::Crashed(detail) => write!(f, "crashed ({detail})"),
            Self::Halted(detail) => write!(f, "halted ({detail})"),
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Returned(value) => write!(f, "returned {value}"),
            Self::Panicked(kind) => write!(f, "panicked ({kind})"),
            Self::Reverted(data) => write!(f, "reverted with 0x{}", hex(data)),
            Self::Aborted(code) => write!(f, "aborted with code {code}"),
            Self::Trapped(trap) => write!(f, "trapped: {trap}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panic_data(code: u8) -> Vec<u8> {
        let mut data = PANIC_SELECTOR.to_vec();
        data.extend([0; 31]);
        data.push(code);
        data
    }

    #[test]
    fn decodes_solidity_panics() {
        assert_eq!(
            Outcome::from_revert(&panic_data(0x11)),
            Outcome::Panicked(PanicKind::Overflow)
        );
        assert_eq!(
            Outcome::from_revert(&panic_data(0x12)),
            Outcome::Panicked(PanicKind::DivisionByZero)
        );
        assert_eq!(
            Outcome::from_revert(&panic_data(0x21)),
            Outcome::Panicked(PanicKind::Code(0x21))
        );
        assert_eq!(
            Outcome::from_revert(&[]),
            Outcome::Panicked(PanicKind::Explicit)
        );
    }

    #[test]
    fn decodes_error_strings_and_keeps_custom_errors() {
        let mut data = ERROR_SELECTOR.to_vec();
        let mut word = |value: u8| {
            data.extend([0; 31]);
            data.push(value);
        };
        word(32);
        word(4);
        data.extend(b"nope");
        data.extend([0; 28]);
        assert_eq!(
            Outcome::from_revert(&data),
            Outcome::Panicked(PanicKind::Message("nope".to_string()))
        );

        let custom = vec![0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 6];
        assert_eq!(
            Outcome::from_revert(&custom),
            Outcome::Reverted(custom.clone())
        );
    }

    #[test]
    fn reads_native_panic_lines() {
        assert_eq!(
            Outcome::from_panic_output("noise\npanicked: arithmetic overflow\n"),
            Some(Outcome::Panicked(PanicKind::Overflow))
        );
        assert_eq!(
            Outcome::from_panic_output("panicked\n"),
            Some(Outcome::Panicked(PanicKind::Explicit))
        );
        assert_eq!(
            Outcome::from_panic_output("panicked: schedule too long"),
            Some(Outcome::Panicked(PanicKind::Message(
                "schedule too long".to_string()
            )))
        );
        let reverted = format!("panicked: reverted with 0x{}", hex(&panic_data(0x01)));
        assert_eq!(
            Outcome::from_panic_output(&reverted),
            Some(Outcome::Panicked(PanicKind::Assertion))
        );
        assert_eq!(Outcome::from_panic_output("segfault"), None);
    }
}
