use std::fmt;

use femp_vir::Span;

/// Code the prover has no model for, and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported {
    pub span: Span,
    pub message: String,
}

impl Unsupported {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.span.is_known() {
            write!(f, "{}: {}", self.span, self.message)
        } else {
            f.write_str(&self.message)
        }
    }
}

/// Why the compiler could not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    /// The directory holds no ingot the compiler can load.
    NotAnIngot(String),
    /// The compiler reported errors, formatted as it prints them.
    Rejected(String),
    /// Code generation, linking or running compiled code failed.
    Failed(String),
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnIngot(message) | Self::Failed(message) => f.write_str(message),
            Self::Rejected(text) => write!(f, "the compiler rejected the ingot:\n{text}"),
        }
    }
}

impl std::error::Error for CompileError {}

impl From<CompileError> for femp_core::Error {
    fn from(error: CompileError) -> Self {
        match error {
            CompileError::Rejected(output) => Self::Rejected {
                output: output.trim_end().to_string(),
            },
            other => Self::Compiler(other.to_string()),
        }
    }
}
