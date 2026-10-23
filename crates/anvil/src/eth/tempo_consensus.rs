//! Native Tempo consensus RPC response types.
//!
//! Local Tempo mining produces a development 1-of-1 finalization over the complete native header.
//! `consensus_getFinalization` accepts `"latest"` or `{"height": N}`; its certificate is unprefixed
//! hex, and its block uses Tempo's native header and transaction types. `consensus_getLatest` has
//! nullable `finalized` and `notarized` fields; local mining only populates `finalized`.
//!
//! Genesis, imported heads, and remote fork anchors remain unsigned. Reset and successful state
//! import start a new local signing history; remote certificates are not forwarded. Snapshots
//! restore pending rotations exactly, while rollback restores the retained block's key schedule.
//! Rotation takes effect in the epoch after the next block's epoch, allowing its predecessor's
//! boundary block to announce the new key before it signs.
//!
//! The latest finalization retains its complete body. Historical queries return the native 503
//! unavailable error if the certificate or a body matching its commitments is unavailable,
//! including after transaction-body pruning. The identity methods expose public development keys
//! only.

use super::{
    backend::tempo_consensus::TempoFinalization,
    error::{BlockchainError, Result},
};
use alloy_primitives::{B256, hex};
use anvil_core::eth::block::Block;
use foundry_primitives::{FoundryHeader, FoundryTxEnvelope};
use serde::Serialize;
use tempo_primitives::TempoTxEnvelope;

/// A finalization certificate and the complete native Tempo block it authenticates.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TempoCertifiedBlock {
    /// Consensus epoch as a JSON number.
    pub epoch: u64,
    /// Consensus view as a JSON number.
    pub view: u64,
    /// Hash of the complete RLP-encoded Tempo header.
    pub digest: B256,
    /// Native finalization bytes encoded as hex, without a `0x` prefix or length framing.
    pub certificate: String,
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
        Ok(Self {
            epoch: finalization.epoch,
            view: finalization.view,
            digest: finalization.digest,
            certificate: hex::encode(finalization.certificate),
            block,
        })
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
