//! Configuration specific to the `forge mutate` command

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::{RegexWrapper, from_opt_glob};

/// Contains the mutation test config
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutationConfig {
    /// path to where mutation artifacts should be written to
    pub out: PathBuf,

    /// Only run test functions matching the specified regex pattern.
    #[serde(rename = "match_test")]
    pub test_pattern: Option<RegexWrapper>,

    /// Only run test functions that do not match the specified regex pattern.
    #[serde(rename = "no_match_test")]
    pub test_pattern_inverse: Option<RegexWrapper>,

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
}

impl Default for MutationConfig {
    fn default() -> Self {
        MutationConfig {
            out: "mutant".into(),
            test_pattern: None,
            test_pattern_inverse: None,
            function_pattern: None,
            function_pattern_inverse: None,
            contract_pattern: None,
            contract_pattern_inverse: None,
            path_pattern: None,
            path_pattern_inverse: None
        }
    }
}
