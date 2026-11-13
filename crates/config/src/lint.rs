//! Configuration specific to the `forge lint` command and the `forge_lint` package

use clap::ValueEnum;
use core::fmt;
use serde::{Deserialize, Deserializer, Serialize};
use solar::{ast, interface::diagnostics::Level};
use std::str::FromStr;
use yansi::Paint;

/// Contains the config and rule set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinterConfig {
    /// Severities of the lints to run.
    ///
    /// Valid values (case-insensitive):
    ///
    /// - `high`
    /// - `medium` (or `med`)
    /// - `low`
    /// - `info`
    /// - `gas`
    /// - `code-size` (or `codesize`, `size`)
    ///
    /// An empty list applies no severity filter, which runs lints of every severity, including
    /// informational, gas and code-size lints. `forge lint --severity` overrides this setting.
    pub severity: Vec<Severity>,

    /// IDs of lints to skip, such as `"mixed-case-function"`.
    ///
    /// `forge lint` fails on an unknown lint ID.
    pub exclude_lints: Vec<String>,

    /// Glob patterns of files to skip when linting.
    ///
    /// Patterns are resolved relative to the project root.
    pub ignore: Vec<String>,

    /// Whether to run the linter during `forge build`.
    ///
    /// Set to `false` to stop linting on every build; `forge build --no-lint` skips it for a
    /// single run. Keeping it enabled and narrowing what runs with `severity`, `exclude_lints` or
    /// `ignore` catches issues earlier. Linting on build only runs when the build has no compiler
    /// errors.
    pub lint_on_build: bool,

    /// Settings for individual lints.
    pub lint_specific: LintSpecificConfig,
}

impl Default for LinterConfig {
    fn default() -> Self {
        Self {
            lint_on_build: true,
            severity: vec![Severity::High, Severity::Med, Severity::Low],
            exclude_lints: Vec::new(),
            ignore: Vec::new(),
            lint_specific: LintSpecificConfig::default(),
        }
    }
}

/// Contract types that can be exempted from the multi-contract-file lint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractException {
    /// Interfaces.
    Interface,
    /// Libraries.
    Library,
    /// Abstract contracts.
    AbstractContract,
}

/// Configuration specific to individual lints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LintSpecificConfig {
    /// Name fragments, usually acronyms, that the `mixedCase` and `PascalCase` lints accept.
    ///
    /// Applies to the `mixed-case-function`, `mixed-case-variable` and `pascal-case-struct` lints.
    /// The defaults allow common names like `rescueERC20`, `ERC721TokenReceiver`, `tokenURI` or
    /// `ERC20Data`.
    pub mixed_case_exceptions: Vec<String>,

    /// Contract kinds that the `multi-contract-file` lint ignores when a file declares more than
    /// one.
    ///
    /// - `interface`: interfaces.
    /// - `library`: libraries.
    /// - `abstract_contract`: abstract contracts.
    ///
    /// Regular contracts can't be exempted: a file with more than one is always flagged.
    pub multi_contract_file_exceptions: Vec<ContractException>,
}

impl Default for LintSpecificConfig {
    fn default() -> Self {
        Self {
            mixed_case_exceptions: vec![
                "ERC".to_string(),
                "URI".to_string(),
                "ID".to_string(),
                "URL".to_string(),
                "API".to_string(),
                "JSON".to_string(),
                "XML".to_string(),
                "HTML".to_string(),
                "HTTP".to_string(),
                "HTTPS".to_string(),
            ],
            multi_contract_file_exceptions: Vec::new(),
        }
    }
}

impl LintSpecificConfig {
    /// Checks if a given contract kind is included in the list of exceptions
    pub fn is_exempted(&self, contract_kind: &ast::ContractKind) -> bool {
        let exception_to_check = match contract_kind {
            ast::ContractKind::Interface => ContractException::Interface,
            ast::ContractKind::Library => ContractException::Library,
            ast::ContractKind::AbstractContract => ContractException::AbstractContract,
            // Regular contracts are always linted
            ast::ContractKind::Contract => return false,
        };

        self.multi_contract_file_exceptions.contains(&exception_to_check)
    }
}

/// Severity of a lint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Severity {
    High,
    Med,
    Low,
    Info,
    Gas,
    CodeSize,
}

impl Severity {
    const fn to_str(self) -> &'static str {
        match self {
            Self::High => "High",
            Self::Med => "Med",
            Self::Low => "Low",
            Self::Info => "Info",
            Self::Gas => "Gas",
            Self::CodeSize => "CodeSize",
        }
    }

    const fn to_str_kebab(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Med => "medium",
            Self::Low => "low",
            Self::Info => "info",
            Self::Gas => "gas",
            Self::CodeSize => "code-size",
        }
    }

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
        write!(f, "{}", self.color(self.to_str()))
    }
}

impl Serialize for Severity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_str_kebab().serialize(serializer)
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
