//! Native Tempo consensus RPC response types.
//!
//! Local Tempo mining produces a development 1-of-1 finalization over the complete native header.
//! `consensus_getFinalization` accepts `"latest"` or `{"height": N}`; its certificate is unprefixed
//! hex, and its block uses Tempo's native header and transaction types. `consensus_getLatest` has
//! nullable `finalized` and `notarized` fields; local mining only populates `finalized`.
//!
//! Genesis and remote fork anchors remain unsigned. Reset and state imports that change canonical
//! history start a new local signing history; imports preserving canonical history retain the
//! local signer. Remote certificates are not forwarded. Snapshots restore the signer exactly within
//! one signing history; cross-import reverts start unsigned. Rollback restores retained key
//! schedules.
//!
//! The latest mined finalization retains its complete body across pruning and snapshot restores.
//! Rolling back to an older certified block whose pruned body no longer matches its header makes
//! both latest queries return the native 503 unavailable error. Mining a successor or restoring a
//! snapshot with a complete body restores availability. Historical queries also return 503 if the
//! certificate or a body matching its commitments is unavailable. The identity methods expose
//! public development keys only.

use super::{
    backend::tempo_consensus::TempoFinalization,
    error::{BlockchainError, Result},
};
use anvil_core::eth::block::Block;
use foundry_primitives::{FoundryHeader, FoundryTxEnvelope};
use serde::Serialize;
use tempo_primitives::TempoTxEnvelope;

/// A finalization certificate and the complete native Tempo block it authenticates.
#[derive(Clone, Debug, Serialize)]
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

/// Current local consensus state, matching Tempo's `consensus_getLatest` response.
#[derive(Clone, Debug, Serialize)]
pub struct TempoConsensusState {
    /// Latest locally finalized block, if any.
    pub finalized: Option<TempoCertifiedBlock>,
    /// Reserved by Tempo; local mining only produces finalizations.
    pub notarized: Option<TempoCertifiedBlock>,
}
