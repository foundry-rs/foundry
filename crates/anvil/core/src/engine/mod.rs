use reth_rpc_types::engine::{
    ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3, ForkchoiceState, PayloadId,
    TransitionConfiguration,
};

#[cfg(feature = "serde")]
use ethers_core::types::serde_helpers::*;

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "method", content = "params"))]
pub enum EngineRequest {
    /// Retrieves the payload corresponding to the specified PayloadId.
    #[cfg_attr(feature = "serde", serde(rename = "engine_getPayloadV3"))]
    GetPayloadV3(PayloadId),

    /// Submits a new payload of a block for validation and execution.
    #[cfg_attr(feature = "serde", serde(rename = "engine_newPayloadV1"))]
    NewPayloadV1(ExecutionPayloadV1),

    #[cfg_attr(feature = "serde", serde(rename = "engine_newPayloadV2"))]
    NewPayloadV2(ExecutionPayloadV2),

    #[cfg_attr(feature = "serde", serde(rename = "engine_newPayloadV3"))]
    NewPayloadV3(ExecutionPayloadV3),

    /// Informs the execution client about the current head of the chain and requests payload building.
    #[cfg_attr(feature = "serde", serde(rename = "engine_forkchoiceUpdatedV1"))]
    ForkchoiceUpdatedV1(ForkchoiceState),

    /// Exchanges transition configuration data between the consensus layer and the execution layer.
    #[cfg_attr(feature = "serde", serde(rename = "engine_exchangeTransitionConfigurationV1"))]
    ExchangeTransitionConfigurationV1(TransitionConfiguration),

    /// Retrieves the bodies for a set of payloads, identified by their hashes.
    #[cfg_attr(feature = "serde", serde(rename = "engine_getPayloadBodiesByHashV1"))]
    GetPayloadBodiesByHashV1(Vec<PayloadId>),

    /// Retrieves the bodies for a set of payloads, within a specified range.
    #[cfg_attr(feature = "serde", serde(rename = "engine_getPayloadBodiesByRangeV1"))]
    GetPayloadBodiesByRangeV1(PayloadId, u64), // Starting PayloadId and count

    /// Hello world request delete me
    #[cfg_attr(feature = "serde", serde(rename = "helloWorld"))]
    HelloWorld(),
}
