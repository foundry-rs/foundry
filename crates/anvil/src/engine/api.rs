use crate::eth::{backend::fork::ClientFork, EthApi};
use std::time::Instant;
use reth::reth_rpc_types::engine::{
    CancunPayloadFields, ExecutionPayload, ExecutionPayloadBodiesV1, ExecutionPayloadEnvelopeV2,
    ExecutionPayloadEnvelopeV3, ExecutionPayloadInputV2, ExecutionPayloadV1, ExecutionPayloadV3,
    ForkchoiceUpdated, PayloadAttributes, PayloadId, PayloadStatus, TransitionConfiguration,
    CAPABILITIES,
};

pub const HELLO_WORLD: &str = "hello world";

#[derive(Clone)]
pub struct EngineApi {
    pub eth_api: EthApi
}

// === impl Engine API ===

impl EngineApi {
    /// Creates a new instance
    #[allow(clippy::too_many_arguments)]
    pub fn new(eth_api: EthApi) -> Self {
        Self { eth_api }
    }

    pub async fn execute(&self) {
        self.hello_world();
        match request {
            EngineRequest::GetPayloadV3(payload_id) => self.get_payload_v3(payload_id).await.to_rpc_result()
        }
    }

    pub fn hello_world(&self) -> Result<String,String> {
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
    ) -> EngineApiResult<ExecutionPayloadEnvelopeV3> {
        // First we fetch the payload attributes to check the timestamp
        let attributes = self.get_payload_attributes(payload_id).await?;

        // validate timestamp according to engine rules
        self.validate_payload_timestamp(EngineApiMessageVersion::V3, attributes.timestamp)?;

        // Now resolve the payload
        Ok(self
            .inner
            .payload_store
            .resolve(payload_id)
            .await
            .ok_or(EngineApiError::UnknownPayload)?
            .map(|payload| (*payload).clone().into_v3_payload())?)
    }

    /// Handler for `engine_getPayloadV3`
    ///
    /// Returns the most recent version of the payload that is available in the corresponding
    /// payload build process at the time of receiving this call.
    ///
    /// See also <https://github.com/ethereum/execution-apis/blob/fe8e13c288c592ec154ce25c534e26cb7ce0530d/src/engine/cancun.md#engine_getpayloadv3>
    ///
    /// Note:
    /// > Provider software MAY stop the corresponding build process after serving this call.
    async fn get_payload_v3(&self, payload_id: PayloadId) -> RpcResult<ExecutionPayloadEnvelopeV3> {
        trace!(target: "rpc::engine", "Serving engine_getPayloadV3");
        let start = Instant::now();
        let res = EngineApi::get_payload_v3(self, payload_id).await;
        self.inner.metrics.get_payload_v3.record(start.elapsed());
        Ok(res?)
    }

    /// Fetches the attributes for the payload with the given id.
    async fn get_payload_attributes(
        &self,
        payload_id: PayloadId,
    ) -> EngineApiResult<PayloadBuilderAttributes> {
        Ok(self
            .inner
            .payload_store
            .payload_attributes(payload_id)
            .await
            .ok_or(EngineApiError::UnknownPayload)??)
    }

    /// Validates the timestamp depending on the version called:
    ///
    /// * If V2, this ensure that the payload timestamp is pre-Cancun.
    /// * If V3, this ensures that the payload timestamp is within the Cancun timestamp.
    ///
    /// Otherwise, this will return [EngineApiError::UnsupportedFork].
    fn validate_payload_timestamp(
        &self,
        version: EngineApiMessageVersion,
        timestamp: u64,
    ) -> EngineApiResult<()> {
        let is_cancun = self.inner.chain_spec.is_cancun_active_at_timestamp(timestamp);
        if version == EngineApiMessageVersion::V2 && is_cancun {
            // From the Engine API spec:
            //
            // ### Update the methods of previous forks
            //
            // This document defines how Cancun payload should be handled by the [`Shanghai
            // API`](https://github.com/ethereum/execution-apis/blob/ff43500e653abde45aec0f545564abfb648317af/src/engine/shanghai.md).
            //
            // For the following methods:
            //
            // - [`engine_forkchoiceUpdatedV2`](https://github.com/ethereum/execution-apis/blob/ff43500e653abde45aec0f545564abfb648317af/src/engine/shanghai.md#engine_forkchoiceupdatedv2)
            // - [`engine_newPayloadV2`](https://github.com/ethereum/execution-apis/blob/ff43500e653abde45aec0f545564abfb648317af/src/engine/shanghai.md#engine_newpayloadV2)
            // - [`engine_getPayloadV2`](https://github.com/ethereum/execution-apis/blob/ff43500e653abde45aec0f545564abfb648317af/src/engine/shanghai.md#engine_getpayloadv2)
            //
            // a validation **MUST** be added:
            //
            // 1. Client software **MUST** return `-38005: Unsupported fork` error if the
            //    `timestamp` of payload or payloadAttributes greater or equal to the Cancun
            //    activation timestamp.
            return Err(EngineApiError::UnsupportedFork)
        }
        if version == EngineApiMessageVersion::V3 && !is_cancun {
            // From the Engine API spec:
            // <https://github.com/ethereum/execution-apis/blob/ff43500e653abde45aec0f545564abfb648317af/src/engine/cancun.md#specification-2>
            //
            // 1. Client software **MUST** return `-38005: Unsupported fork` error if the
            //    `timestamp` of the built payload does not fall within the time frame of the Cancun
            //    fork.
            return Err(EngineApiError::UnsupportedFork)
        }
        Ok(())
    }
    
}