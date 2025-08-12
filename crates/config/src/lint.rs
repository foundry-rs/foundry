//! Configuration specific to the `forge lint` command and the `forge_lint` package

use clap::ValueEnum;
use core::fmt;
use serde::{Deserialize, Deserializer, Serialize};
use solar_interface::diagnostics::Level;
use std::str::FromStr;
use yansi::Paint;

/// Contains the config and rule set
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinterConfig {
    /// Specifies which lints to run based on severity.
    ///
    /// If uninformed, all severities are checked.
    pub severity: Vec<Severity>,

    /// Deny specific lints based on their ID (e.g. "mixed-case-function").
    pub exclude_lints: Vec<String>,

    /// Globs to ignore
    pub ignore: Vec<String>,

    /// Whether to run linting during `forge build`.
    ///
    /// Defaults to true. Set to false to disable automatic linting during builds.
    pub lint_on_build: bool,
}
impl Default for LinterConfig {
    fn default() -> Self {
        Self {
            lint_on_build: true,
            severity: Vec::new(),
            exclude_lints: Vec::new(),
            ignore: Vec::new(),
        }
    }
}

/// Severity of a lint
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
pub enum Severity {
    High,
    Med,
    Low,
    Info,
    Gas,
    CodeSize,
}

impl Severity {
    pub fn color(&self, message: &str) -> String {
        match self {
            Self::High => Paint::red(message).bold().to_string(),
            Self::Med => Paint::rgb(message, 255, 135, 61).bold().to_string(),
            Self::Low => Paint::yellow(message).bold().to_string(),
            Self::Info => Paint::cyan(message).bold().to_string(),
            Self::Gas => Paint::green(message).bold().to_string(),
            Self::CodeSize => Paint::green(message).bold().to_string(),
        }
    }
}

impl From<Severity> for Level {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::High | Severity::Med | Severity::Low => Self::Warning,
            Severity::Info | Severity::Gas | Severity::CodeSize => Self::Note,
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let colored = match self {
            Self::High => self.color("High"),
            Self::Med => self.color("Med"),
            Self::Low => self.color("Low"),
            Self::Info => self.color("Info"),
            Self::Gas => self.color("Gas"),
            Self::CodeSize => self.color("CodeSize"),
        };
        write!(f, "{colored}")
    }
}

// Custom deserialization to make `Severity` parsing case-insensitive
impl<'de> Deserialize<'de> for Severity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl FromStr for Severity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "high" => Ok(Self::High),
            "med" | "medium" => Ok(Self::Med),
            "low" => Ok(Self::Low),
            "info" => Ok(Self::Info),
            "gas" => Ok(Self::Gas),
            "size" | "codesize" | "code-size" => Ok(Self::CodeSize),
            _ => Err(format!(
                "unknown variant: found `{s}`, expected `one of `High`, `Med`, `Low`, `Info`, `Gas`, `CodeSize`"
            )),
        }
    }
}
