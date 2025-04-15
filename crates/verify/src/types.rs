use eyre::Result;
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Enum to represent the type of verification: `full` or `partial`.
/// Ref: <https://docs.sourcify.dev/docs/full-vs-partial-match/>
#[derive(Debug, Clone, clap::ValueEnum, Default, PartialEq, Eq, Serialize, Deserialize, Copy)]
pub enum VerificationType {
    #[default]
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "partial")]
    Partial,
}

impl FromStr for VerificationType {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "full" => Ok(Self::Full),
            "partial" => Ok(Self::Partial),
            _ => eyre::bail!("Invalid verification type"),
        }
    }
}

impl From<VerificationType> for String {
    fn from(v: VerificationType) -> Self {
        match v {
            VerificationType::Full => "full".to_string(),
            VerificationType::Partial => "partial".to_string(),
        }
    }
}

impl fmt::Display for VerificationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => write!(f, "full"),
            Self::Partial => write!(f, "partial"),
        }
    }
}
