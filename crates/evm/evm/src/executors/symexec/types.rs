//! Shared data types for the symbolic-assist worker.

use alloy_primitives::{Address, B256};
use foundry_evm_fuzz::BasicTxDetails;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// A specific branch in a specific contract that the symbolic worker can try
/// to flip. Computed from a [`BranchObservation`](crate::inspectors::BranchObservation)
/// and the index of the transaction in the seed sequence that exercised it.
///
/// Two observations with the same `FrontierKey` represent "the same branch in
/// the same call slot" and should share attempt bookkeeping so the worker
/// doesn't repeatedly hammer an unflippable guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrontierKey {
    /// Contract whose bytecode contains the branch.
    pub address: Address,
    /// Program counter of the `JUMPI`.
    pub pc: usize,
    /// Destination of the side that the symbolic worker is trying to reach
    /// (i.e. the side currently *not* covered).
    pub other_dest_lo: u64,
    /// Index of the call in the seed `Vec<BasicTxDetails>` that produced this
    /// observation. For single-call (stateless) fuzz this is always 0; for
    /// invariant tests it identifies which tx in the sequence to mutate.
    pub tx_index: u32,
    /// First 4 bytes of the calldata of that call (selector). Lets us
    /// distinguish frontiers reached through different entrypoints.
    pub selector: [u8; 4],
}

/// Per-frontier attempt bookkeeping, kept separately from [`super::corpus`]
/// stats so the symbolic worker doesn't pollute the fuzzer's counters.
#[derive(Clone, Debug, Default)]
pub struct FrontierStats {
    pub attempts: u16,
    pub successes: u16,
    /// UUID of the corpus entry the last attempt was generated from.
    pub last_source_uuid: Option<Uuid>,
}

/// Hard cap on attempts per frontier per campaign.
pub const MAX_FRONTIER_ATTEMPTS: u16 = 3;

/// Hard cap on candidates evaluated per frontier per cycle.
pub const MAX_CANDIDATES_PER_FRONTIER: usize = 8;

/// Hard cap on seeds processed per assist cycle.
pub const MAX_SEEDS_PER_CYCLE: usize = 1;

/// In-process state for the symbolic worker. Owned by the master worker; not
/// persisted to disk (regenerated fresh each campaign).
#[derive(Clone, Debug, Default)]
pub struct SymExecState {
    /// Bookkeeping per frontier we have already tried to flip.
    pub frontiers: HashMap<FrontierKey, FrontierStats>,
    /// Hashes (e.g. `keccak256(serialize(seq))`) of candidate sequences we
    /// have already proposed, to avoid re-validating duplicates.
    pub seen_candidate_hashes: HashSet<B256>,
}

impl SymExecState {
    /// Whether we should attempt this frontier again.
    pub fn should_try(&self, key: &FrontierKey) -> bool {
        self.frontiers.get(key).map(|s| s.attempts < MAX_FRONTIER_ATTEMPTS).unwrap_or(true)
    }

    pub fn record_attempt(&mut self, key: FrontierKey, source: Uuid, success: bool) {
        let entry = self.frontiers.entry(key).or_default();
        entry.attempts = entry.attempts.saturating_add(1);
        if success {
            entry.successes = entry.successes.saturating_add(1);
        }
        entry.last_source_uuid = Some(source);
    }
}

/// One generated candidate to validate.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// The mutated transaction sequence (same length as the source seed).
    pub tx_seq: Vec<BasicTxDetails>,
    /// The frontier the candidate is trying to flip; used for bookkeeping
    /// after validation.
    pub frontier: FrontierKey,
    /// UUID of the corpus entry the candidate was derived from.
    pub source_uuid: Uuid,
}
