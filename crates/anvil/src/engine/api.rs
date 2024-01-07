use crate::eth::{backend::fork::ClientFork, EthApi};
use std::{time::Instant, collections::HashMap};
use anvil_core::engine::EngineRequest;
use anvil_rpc::response::ResponseResult;
use reth_rpc_types::engine::{
    CancunPayloadFields, ExecutionPayload, ExecutionPayloadBodiesV1, ExecutionPayloadEnvelopeV2,
    ExecutionPayloadEnvelopeV3, ExecutionPayloadInputV2, ExecutionPayloadV1, ExecutionPayloadV3,
    ForkchoiceUpdated, PayloadAttributes, PayloadId, PayloadStatus, TransitionConfiguration,
    CAPABILITIES,
};

use crate::engine::error::Result;

// use crate::core::engine::EngineRequest;
// use reth_rpc_types::error::{EngineApiError, EngineApiResult};

pub const HELLO_WORLD: &str = "hello world";

#[derive(Clone)]
pub struct EngineApi {
    pub eth_api: EthApi,
    pub payload_store: HashMap<PayloadId, ExecutionPayloadEnvelopeV3>,
    pub payload_attribute_store: HashMap<PayloadId, PayloadAttributes>,
}

use crate::engine::error::ToRpcResponseResult;

// === impl Engine API ===

impl EngineApi {
    /// Creates a new instance
    #[allow(clippy::too_many_arguments)]
    pub fn new(eth_api: EthApi) -> Self {
        Self { eth_api, payload_store: HashMap::new(), payload_attribute_store: HashMap::new() }
    }

    pub async fn execute(&self, request: EngineRequest) -> ResponseResult {
        self.hello_world();
        match request {
            EngineRequest::GetPayloadV3(payload_id) => self.get_payload_v3(payload_id).await.to_rpc_result()
        }
    }

    pub fn hello_world(&self) -> Result<String> {
        print!("hello world");
        Ok(HELLO_WORLD.to_string())
    }

    pub fn get_fork(&self) -> Option<ClientFork> {
        None
    }

    /// Returns the most recent version of the payload that is available in the corresponding
    /// payload build process at the time of receiving this call.
    ///
    /// See also <https://github.com/ethereum/execution-apis/blob/fe8e13c288c592ec154ce25c534e26cb7ce0530d/src/engine/cancun.md#engine_getpayloadv3>
    ///
    /// Note:
    /// > Provider software MAY stop the corresponding build process after serving this call.
    pub async fn get_payload_v3(
        &self,
        payload_id: PayloadId,
    ) -> Result<ExecutionPayloadEnvelopeV3> {
        // For now we're skipping validation
            // First we fetch the payload attributes to check the timestamp
            // validate timestamp according to engine rules
            // self.validate_payload_timestamp(EngineApiMessageVersion::V3, attributes.timestamp)?;

        // Now resolve the payload
        Ok(self
            .payload_store.get(&payload_id).unwrap().clone())
    }
    
}