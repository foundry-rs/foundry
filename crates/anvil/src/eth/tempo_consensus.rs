//! Native Tempo consensus RPC response types.
//!
//! `consensus_getFinalization` returns a [`TempoCertifiedBlock`]. `consensus_getLatest` returns a
//! [`TempoConsensusState`] with nullable `finalized` and `notarized`; local mining only populates
//! `finalized`. The certificate bytes are unprefixed hex.
//!
//! Identity rotation takes effect in the epoch after the next block's epoch, allowing its
//! predecessor's boundary block to announce the new key before it signs. Snapshots restore pending
//! rotations exactly; rollback restores the retained block's key schedule.
//!
//! Locally mined fork blocks commit to a partial trie of the accounts and storage slots cached
//! when the block was mined, rather than the complete upstream state. Non-pending `eth_getProof`
//! queries use that immutable trie, including at the current head after development state edits.
//! Accounts and slots absent from it have exclusion proofs, even if they exist upstream. Later
//! upstream reads cannot extend an already certified trie. Proof snapshots follow the configured
//! state-history cache limits; evicted snapshots are unavailable rather than rebuilt from live
//! state. The latest snapshot is retained even when historical state retention is disabled.

use super::{
    backend::tempo_consensus::TempoFinalization,
    error::{BlockchainError, Result},
};
use anvil_core::eth::block::Block;
use foundry_primitives::{FoundryHeader, FoundryTxEnvelope};
use serde::{Deserialize, Serialize};
use tempo_primitives::TempoTxEnvelope;

/// A finalization certificate and the complete native Tempo block it authenticates.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TempoCertifiedBlock {
    /// Certificate metadata and bytes for this block.
    #[serde(flatten)]
    pub finalization: TempoFinalization,
    /// Native header and body, without Anvil transaction wrappers.
    pub block: tempo_primitives::Block,
}

impl TryFrom<(TempoFinalization, Block)> for TempoCertifiedBlock {
    type Error = BlockchainError;

    fn try_from((finalization, block): (TempoFinalization, Block)) -> Result<Self> {
        let block = block
            .try_map_header(|header| match header {
                FoundryHeader::Tempo(header) => Ok(header),
                FoundryHeader::Ethereum(_) => {
                    Err(BlockchainError::Internal("expected a native Tempo header".into()))
                }
            })?
            .try_map_transactions(|tx| match tx.into_inner() {
                FoundryTxEnvelope::Legacy(tx) => Ok(TempoTxEnvelope::Legacy(tx)),
                FoundryTxEnvelope::Eip2930(tx) => Ok(TempoTxEnvelope::Eip2930(tx)),
                FoundryTxEnvelope::Eip1559(tx) => Ok(TempoTxEnvelope::Eip1559(tx)),
                FoundryTxEnvelope::Eip7702(tx) => Ok(TempoTxEnvelope::Eip7702(tx)),
                FoundryTxEnvelope::Tempo(tx) => Ok(TempoTxEnvelope::AA(tx)),
                _ => Err(BlockchainError::Internal("expected a native Tempo transaction".into())),
            })?;
        Ok(Self { finalization, block })
    }
}

/// Current consensus state, matching Tempo's `consensus_getLatest` response.
#[derive(Clone, Debug, Serialize)]
pub struct TempoConsensusState {
    /// Latest local finalization, or the pinned upstream finalization before local mining.
    pub finalized: Option<TempoCertifiedBlock>,
    /// Reserved by Tempo; local mining only produces finalizations.
    pub notarized: Option<TempoCertifiedBlock>,
}

/// A locally finalized block, matching Tempo's consensus subscription event.
///
/// The live stream is bounded and skips older events when a subscriber falls behind, matching
/// Tempo's RPC behavior. Clients needing a complete canonical chain must reconcile block heights
/// and hashes and query `consensus_getFinalization` while the missing history is still retained.
/// Pruned or rewound history may be unavailable; subscriptions do not provide durable replay.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TempoConsensusEvent {
    /// A complete block and its finalization certificate at canonical publication.
    Finalized {
        /// The certified block, independent of subsequent pruning or rewinds.
        #[serde(flatten)]
        block: TempoCertifiedBlock,
        /// Unix timestamp in milliseconds when this finalization was observed.
        seen: u64,
    },
}
