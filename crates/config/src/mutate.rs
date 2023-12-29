//! Configuration specific to the `forge mutate` command

use serde::{Deserialize, Serialize};
use std::{path::PathBuf, str::FromStr};
use crate::{RegexWrapper, from_opt_glob};

/// Contains the mutation test config
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MutateConfig {
    /// path to where mutation artifacts should be written to
    #[serde(default = "default_out_path")]
    pub out: PathBuf,

    /// Flag to write out mutants
    #[serde(default = "default_export")]
    pub export: bool,

    /// Only run mutations for functions matching the specified regex pattern.
    #[serde(rename = "match_function")]
    pub function_pattern: Option<RegexWrapper>,

    /// Only run mutations on functions that do not match the specified regex pattern.
    #[serde(rename = "no_match_function")]
    pub function_pattern_inverse: Option<RegexWrapper>,

    /// Only run mutations on functions in contracts matching the specified regex pattern.
    #[serde(rename = "match_contract")]
    pub contract_pattern: Option<RegexWrapper>,

    /// Only run mutations in contracts that do not match the specified regex pattern.
    #[serde(rename = "no_match_contract")]
    pub contract_pattern_inverse: Option<RegexWrapper>,

    /// Only run mutations on source files matching the specified glob pattern.
    #[serde(rename = "match_path", with = "from_opt_glob")]
    pub path_pattern: Option<globset::Glob>,

    /// Only run mutations on source files that do not match the specified glob pattern.
    #[serde(rename = "no_match_path", with = "from_opt_glob")]
    pub path_pattern_inverse: Option<globset::Glob>,

    /// Only run test functions matching the specified regex pattern.
    #[serde(rename = "match_test")]
    pub test_pattern: Option<RegexWrapper>,

    /// Only run test functions that do not match the specified regex pattern.
    #[serde(rename = "no_match_test")]
    pub test_pattern_inverse: Option<RegexWrapper>,

    /// Only run tests in contracts matching the specified regex pattern.
    #[serde(rename = "match_test_contract")]
    pub test_contract_pattern: Option<RegexWrapper>,

    /// Only run tests in contracts that do not match the specified regex pattern.
    #[serde(rename = "no_match_test_contract")]
    pub test_contract_pattern_inverse: Option<RegexWrapper>,

    /// Only run tests in source files matching the specified glob pattern.
    #[serde(rename = "match_test_path", with = "from_opt_glob")]
    pub test_path_pattern: Option<globset::Glob>,

    /// Only run tests in source files that do not match the specified glob pattern.
    #[serde(rename = "no_match_test_path", with = "from_opt_glob")]
    pub test_path_pattern_inverse: Option<globset::Glob>,
}

fn default_out_path() -> PathBuf {
    PathBuf::from_str("mutate").unwrap()
}

fn default_export() -> bool {
    false
}