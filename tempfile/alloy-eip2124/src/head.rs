use alloy_primitives::{BlockNumber, B256, U256};
use core::fmt;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Describes the current head block.
///
/// The head block is the highest fully synced block.
///
/// Note: This is a slimmed down version of Header, primarily for communicating the highest block
/// with the P2P network and the RPC.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Head {
    /// The number of the head block.
    pub number: BlockNumber,
    /// The hash of the head block.
    pub hash: B256,
    /// The difficulty of the head block.
    pub difficulty: U256,
    /// The total difficulty at the head block.
    pub total_difficulty: U256,
    /// The timestamp of the head block.
    pub timestamp: u64,
}

impl Head {
    /// Creates a new `Head` instance.
    pub const fn new(
        number: BlockNumber,
        hash: B256,
        difficulty: U256,
        total_difficulty: U256,
        timestamp: u64,
    ) -> Self {
        Self { number, hash, difficulty, total_difficulty, timestamp }
    }

    /// Updates the head block with new information.
    pub fn update(
        &mut self,
        number: BlockNumber,
        hash: B256,
        difficulty: U256,
        total_difficulty: U256,
        timestamp: u64,
    ) {
        *self = Self { number, hash, difficulty, total_difficulty, timestamp };
    }

    /// Checks if the head block is an empty block (i.e., has default values).
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl fmt::Display for Head {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Head Block:\n Number: {}\n Hash: {}\n Difficulty: {:?}\n Total Difficulty: {:?}\n Timestamp: {}",
            self.number, self.hash, self.difficulty, self.total_difficulty, self.timestamp
        )
    }
}
