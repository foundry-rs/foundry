//! Configuration for fuzz testing

use ethers_core::types::U256;
use serde::{Deserialize, Serialize};

// TODO:
// exposing config items for
// - percent of time dict vs. random vs. edge is used (edge + dict should be merged)
// - include-stack
// - include-memory
// - include-storage-keys
// - include-storage-values
// - include-push-bytes (constants, immutables)
// could help people fine tune that trade off, but also may be overkill

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
    // TODO:
    ///
    pub include_stack: bool,
    ///
    pub include_memory: bool,
    ///
    pub include_storage: bool,
    ///
    pub dict_weight: u32, // TODO: validation
}

impl Default for FuzzConfig {
    fn default() -> Self {
        FuzzConfig {
            runs: 256,
            max_local_rejects: 1024,
            max_global_rejects: 65536,
            seed: None,
            include_stack: true,
            include_memory: true,
            include_storage: true,
            dict_weight: 80,
        }
    }
}
