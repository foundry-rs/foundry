//! Tempo's `tempo_simulateV1` response and the TIP-20 tokens it reports metadata for.

use alloy_network::AnyRpcBlock;
use alloy_primitives::Address;
use alloy_rpc_types::{
    TransactionRequest,
    simulate::{SimulatePayload, SimulatedBlock},
};
use alloy_serde::WithOtherFields;
use alloy_sol_types::SolEvent;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tempo_precompiles::tip20::{ITIP20, Tip20TokenMetadata};
use tempo_primitives::{TempoAddressExt, transaction::Call};

/// Response of `tempo_simulateV1`, matching a Tempo node's.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TempoSimulateV1Response {
    /// The simulated blocks, as `eth_simulateV1` returns them.
    pub blocks: Vec<SimulatedBlock<AnyRpcBlock>>,
    /// Metadata of the deployed TIP-20 tokens that are called, used as a fee token, or emit a
    /// `Transfer` log.
    pub token_metadata: BTreeMap<Address, Tip20TokenMetadata>,
}

/// Returns the TIP-20 addresses the calls of `payload` target, directly or in a Tempo batch, or
/// pay fees with.
pub(crate) fn tip20_targets(
    payload: &SimulatePayload<WithOtherFields<TransactionRequest>>,
) -> BTreeSet<Address> {
    payload
        .block_state_calls
        .iter()
        .flat_map(|block| &block.calls)
        .flat_map(|request| {
            let batch = request.other.get_deserialized::<Vec<Call>>("calls").and_then(Result::ok);
            let fee_token =
                request.other.get_deserialized::<Address>("feeToken").and_then(Result::ok);
            request
                .to
                .and_then(|to| to.into_to())
                .into_iter()
                .chain(batch.into_iter().flatten().filter_map(|call| call.to.into_to()))
                .chain(fee_token)
        })
        .filter(|address| address.is_tip20())
        .collect()
}

/// Returns the TIP-20 tokens that emit a `Transfer` log in `blocks`.
pub(crate) fn transferred_tip20s(
    blocks: &[SimulatedBlock<AnyRpcBlock>],
) -> impl Iterator<Item = Address> + '_ {
    blocks
        .iter()
        .flat_map(|block| &block.calls)
        .flat_map(|call| &call.logs)
        .filter(|log| {
            log.address().is_tip20() && log.topic0() == Some(&ITIP20::Transfer::SIGNATURE_HASH)
        })
        .map(|log| log.address())
}
