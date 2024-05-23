use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Compilers supported by foundry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Language {
    /// Solidity
    Solidity,
    /// Vyper
    Vyper,
}

impl Language {
    /// Returns the language name as a string.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Solidity => "solidity",
            Self::Vyper => "vyper",
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Language {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "solidity" => Ok(Self::Solidity),
            "vyper" => Ok(Self::Vyper),
            s => Err(format!("Unknown language: {s}")),
        }
    }
}
