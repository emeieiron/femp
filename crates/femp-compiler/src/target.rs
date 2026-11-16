use std::fmt;

use common::compilation::Target as FeTarget;

/// A machine the compiler emits code for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Evm,
    Native,
    Wasm,
}

impl Target {
    pub const ALL: [Self; 3] = [Self::Evm, Self::Native, Self::Wasm];

    /// Reads a target as `fe.toml` names it.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|target| target.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Evm => "evm",
            Self::Native => "native",
            Self::Wasm => "wasm",
        }
    }

    /// The backend that compiles for the target, as the compiler names it.
    pub fn backend(self) -> &'static str {
        match self {
            Self::Evm => "sonatina",
            Self::Native => "native",
            Self::Wasm => "wasm",
        }
    }

    /// The width of `usize` and `isize`.
    pub fn word_bits(self) -> u16 {
        match self {
            Self::Evm => 256,
            Self::Native => 64,
            Self::Wasm => 32,
        }
    }

    pub(crate) fn fe(self) -> FeTarget {
        match self {
            Self::Evm => FeTarget::Evm,
            Self::Native => FeTarget::Native,
            Self::Wasm => FeTarget::Wasm,
        }
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
