use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Environment {
    pub toolchain: Toolchain,
    pub host: Host,
}

/// The Fe compiler femp is built with.
#[derive(Debug, Clone)]
pub struct Toolchain {
    pub version: String,
    pub commit: String,
    /// The backends it compiles with, as it names them.
    pub backends: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Host {
    pub os: String,
    pub arch: String,
    /// The Node.js that runs WebAssembly code, and its major version.
    pub node: Option<PathBuf>,
    pub node_major: Option<u32>,
    pub has_c_compiler: bool,
    /// The Z3 solver that proves claims, and its version.
    pub z3: Option<PathBuf>,
    pub z3_version: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OptLevel {
    None,
    #[default]
    Balanced,
    Aggressive,
    Size,
}

impl OptLevel {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "0" => Some(Self::None),
            "1" => Some(Self::Balanced),
            "2" => Some(Self::Aggressive),
            "s" => Some(Self::Size),
            _ => None,
        }
    }

    pub fn as_arg(self) -> &'static str {
        match self {
            Self::None => "0",
            Self::Balanced => "1",
            Self::Aggressive => "2",
            Self::Size => "s",
        }
    }
}
