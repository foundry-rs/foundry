//! Configuration for fuzz testing

use ethers_core::types::U256;
use serde::{Deserialize, Serialize};

/// Contains for fuzz testing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuzzConfig {
    /// The number of test cases that must execute for each property test
    pub runs: u32,
    /// The maximum number of local test case rejections allowed
    /// by proptest, to be encountered during usage of `vm.assume`
    /// cheatcode.
    pub max_local_rejects: u32,
    /// The maximum number of global test case rejections allowed
    /// by proptest, to be encountered during usage of `vm.assume`
    /// cheatcode.
    pub max_global_rejects: u32,
    /// Optional seed for the fuzzing RNG algorithm
    #[serde(
        deserialize_with = "ethers_core::types::serde_helpers::deserialize_stringified_numeric_opt"
    )]
    pub seed: Option<U256>,
    /// The weight of the dictionary
    #[serde(deserialize_with = "crate::deserialize_stringified_percent")]
    pub dictionary_weight: u32,
    /// The flag indicating whether to include values from storage
    pub include_storage: bool,
    /// The flag indicating whether to include push bytes values
    pub include_push_bytes: bool,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        FuzzConfig {
            runs: 256,
            max_local_rejects: 1024,
            max_global_rejects: 65536,
            seed: None,
            dictionary_weight: 40,
            include_storage: true,
            include_push_bytes: true,
        }
    }
}
