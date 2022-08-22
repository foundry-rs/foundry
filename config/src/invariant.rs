//! Configuration for invariant testing

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

/// Contains for invariant testing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantConfig {
    /// The number of runs that must execute for each invariant test group.
    pub runs: u32,
    /// The number of calls executed to attempt to break invariants in one run.
    pub depth: u32,
    /// Fails the invariant fuzzing if a revert occurs
    pub fail_on_revert: bool,
    /// Allows overriding an unsafe external call when running invariant tests. eg. reentrancy
    /// checks
    pub call_override: bool,
    ///
    pub include_stack: bool,
    ///
    pub include_memory: bool,
    ///
    pub include_storage_keys: bool,
    ///
    pub dict_weight: u32, // TODO: validation
}

impl Default for InvariantConfig {
    fn default() -> Self {
        InvariantConfig {
            runs: 256,
            depth: 15,
            fail_on_revert: false,
            call_override: false,
            include_stack: true,
            include_memory: true,
            include_storage_keys: true,
            dict_weight: 80,
        }
    }
}
