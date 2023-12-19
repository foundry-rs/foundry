use reth_rpc_types::engine::{
    payload::{ExecutionPayloadBodyV1, ExecutionPayloadFieldV2, ExecutionPayloadInputV2},
    ExecutionPayload, ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3, PayloadError, PayloadId,
};

#[cfg(feature = "serde")]
use ethers_core::types::serde_helpers::*;

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "method", content = "params"))]
pub enum EngineRequest {
    GetPayloadV3(PayloadId),
    HelloWorld()
}