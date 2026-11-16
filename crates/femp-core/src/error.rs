use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no `{file}` found in `{}` or any parent directory", .start.display())]
    ProjectNotFound { file: &'static str, start: PathBuf },

    #[error("failed to {action} `{}`: {source}", .path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid `{}`: {message}", .path.display())]
    InvalidManifest { path: PathBuf, message: String },

    #[error("project layout is invalid:{}", bullet_list(.0))]
    Layout(Vec<String>),

    #[error("unknown platform `{name}` (known platforms: {known})")]
    UnknownPlatform { name: String, known: String },

    #[error("platform `{platform}` is unavailable: {reason}")]
    Unavailable { platform: String, reason: String },

    #[error("toolchain: {0}")]
    Toolchain(String),

    #[error("`{command}` failed\n{output}")]
    Command { command: String, output: String },

    /// The compiler's errors, formatted as it prints them.
    #[error("the Fe compiler reported errors:\n{output}")]
    Rejected { output: String },

    #[error("compiler: {0}")]
    Compiler(String),

    #[error("{0}")]
    Artifact(String),
}

impl Error {
    pub fn io(action: &'static str, path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            action,
            path: path.to_path_buf(),
            source,
        }
    }

    pub fn invalid_manifest(path: &Path, message: impl Into<String>) -> Self {
        Self::InvalidManifest {
            path: path.to_path_buf(),
            message: message.into(),
        }
    }
}

fn bullet_list(items: &[String]) -> String {
    items.iter().map(|item| format!("\n  - {item}")).collect()
}
