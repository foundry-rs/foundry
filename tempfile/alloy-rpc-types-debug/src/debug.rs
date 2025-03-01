//! Types for the `debug` API.

use alloy_primitives::{map::B256HashMap, Bytes};
use serde::{Deserialize, Serialize};

/// Represents the execution witness of a block. Contains an optional map of state preimages.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionWitness {
    /// Map of all hashed trie nodes to their preimages that were required during the execution of
    /// the block, including during state root recomputation.
    ///
    /// `keccak(rlp(node)) => rlp(node)`
    pub state: B256HashMap<Bytes>,
    /// Map of all contract codes (created / accessed) to their preimages that were required during
    /// the execution of the block, including during state root recomputation.
    ///
    /// `keccak(bytecodes) => bytecodes`
    pub codes: B256HashMap<Bytes>,
    /// Map of all hashed account and storage keys (addresses and slots) to their preimages
    /// (unhashed account addresses and storage slots, respectively) that were required during
    /// the execution of the block.
    ///
    /// `keccak(address|slot) => address|slot`
    pub keys: B256HashMap<Bytes>,
}
