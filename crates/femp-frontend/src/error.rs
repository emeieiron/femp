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

/// Why an ingot could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    NotAnIngot(String),
    /// The compiler reported errors, formatted as it prints them.
    Diagnostics(String),
    UnknownTarget(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnIngot(message) => f.write_str(message),
            Self::Diagnostics(text) => write!(f, "the compiler rejected the ingot:\n{text}"),
            Self::UnknownTarget(target) => write!(f, "the Fe compiler has no target `{target}`"),
        }
    }
}

impl std::error::Error for LoadError {}
