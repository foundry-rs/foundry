//! Contains RPC handlers
use crate::{
    EthApi,
    eth::{api::RpcCallLogContext, error::to_rpc_result},
    pubsub::{EthSubscription, LogsSubscription},
};
use alloy_rpc_types::{
    FilteredParams,
    pubsub::{Params, SubscriptionKind},
};
use anvil_core::eth::{EthPubSub, EthRequest, EthRpcCall, subscription::SubscriptionId};
use anvil_rpc::{
    error::RpcError,
    request::RpcMethodCall,
    response::{ResponseResult, RpcResponse},
};
use anvil_server::{PubSubContext, PubSubRpcHandler, RpcHandler};
use chrono::Utc;
use serde_json::json;
use std::{fmt, net::SocketAddr};
use tracing::{error, trace};

#[derive(Clone)]
/// Wrapper around a JSON-RPC request used by `RpcHandler`.
///
/// This type keeps together:
/// - the parsed, strongly-typed representation of the request (`parsed`),
/// - the original raw JSON value as it was received over the wire (`raw`), and
/// - additional logging/telemetry metadata associated with the call (`metadata`).
///
/// Keeping both the parsed and raw forms allows handlers and logging code to
/// inspect or record the exact request payload while still working with a
/// typed representation.
pub struct JsonRpcRequest<T> {
    /// The strongly-typed, already-deserialized representation of the request.
    parsed: T,
    /// The original JSON-RPC request payload as received from the client.
    raw: serde_json::Value,
    /// Context metadata used for logging, tracing, and metrics for this call.
    metadata: RpcCallLogContext,
}

impl<T> JsonRpcRequest<T> {
    fn new(parsed: T, raw: serde_json::Value, metadata: RpcCallLogContext) -> Self {
        Self { parsed, raw, metadata }
    }

    fn into_parts(self) -> (T, serde_json::Value, RpcCallLogContext) {
        (self.parsed, self.raw, self.metadata)
    }
}

impl<T> fmt::Debug for JsonRpcRequest<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonRpcRequest")
            .field("parsed", &self.parsed)
            .field("raw", &self.raw)
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl<'de, T> serde::Deserialize<'de> for JsonRpcRequest<T>
where
    T: serde::de::DeserializeOwned,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = serde_json::Value::deserialize(deserializer)?;
        let parsed = T::deserialize(&raw).map_err(serde::de::Error::custom)?;
        Ok(Self { parsed, raw, metadata: RpcCallLogContext::default() })
    }
}

/// A `RpcHandler` that expects `EthRequest` rpc calls via http
#[derive(Clone)]
pub struct HttpEthRpcHandler {
    /// Access to the node
    api: EthApi,
}

impl HttpEthRpcHandler {
    /// Creates a new instance of the handler using the given `EthApi`
    pub fn new(api: EthApi) -> Self {
        Self { api }
    }

    /// Converts a `RpcMethodCall` into a `JsonRpcRequest<EthRequest>`.
    ///
    /// This helper captures the raw JSON request and creates logging metadata
    /// (timestamp, peer address) that will be used for verbose RPC logging.
    fn try_into_request(
        call: &RpcMethodCall,
        peer_addr: Option<SocketAddr>,
    ) -> Result<JsonRpcRequest<EthRequest>, serde_json::Error> {
        let params_value: serde_json::Value = call.params.clone().into();

        // Construct the full raw JSON-RPC request for logging
        let raw = json!({
            "jsonrpc": &call.jsonrpc,
            "method": &call.method,
            "params": &params_value,
            "id": &call.id,
        });

        // Build metadata context for logging
        let metadata = RpcCallLogContext {
            id: Some(call.id.clone()),
            method: Some(call.method.clone()),
            peer_addr,
            timestamp: Some(Utc::now()),
        };

        // Deserialize into EthRequest
        let call_value = json!({
            "method": &call.method,
            "params": params_value,
        });
        let parsed = serde_json::from_value::<EthRequest>(call_value)?;

        Ok(JsonRpcRequest::new(parsed, raw, metadata))
    }
}

#[async_trait::async_trait]
impl RpcHandler for HttpEthRpcHandler {
    type Request = JsonRpcRequest<EthRequest>;

    async fn on_request(&self, request: Self::Request) -> ResponseResult {
        let (request, raw, metadata) = request.into_parts();
        self.api.execute_with_raw(request, Some(raw), metadata).await
    }

    async fn on_call(&self, call: RpcMethodCall, peer_addr: Option<SocketAddr>) -> RpcResponse {
        trace!(
            target: "rpc",
            id = ?call.id,
            method = ?call.method,
            ?peer_addr,
            "handling call"
        );

        let method = &call.method;
        let id = call.id.clone();

        match Self::try_into_request(&call, peer_addr) {
            Ok(request) => {
                let result = self.on_request(request).await;
                RpcResponse::new(id, result)
            }
            Err(err) => {
                let err = err.to_string();
                if err.contains("unknown variant") {
                    error!(
                        target: "rpc",
                        method = ?method,
                        "failed to deserialize method due to unknown variant"
                    );
                    RpcResponse::new(id, RpcError::method_not_found())
                } else {
                    error!(target: "rpc", method = ?method, ?err, "failed to deserialize method");
                    RpcResponse::new(id, RpcError::invalid_params(err))
                }
            }
        }
    }
}

/// A `RpcHandler` that expects `EthRequest` rpc calls and `EthPubSub` via pubsub connection
#[derive(Clone)]
pub struct PubSubEthRpcHandler {
    /// Access to the node
    api: EthApi,
}

impl PubSubEthRpcHandler {
    /// Creates a new instance of the handler using the given `EthApi`
    pub fn new(api: EthApi) -> Self {
        Self { api }
    }

    /// Invoked for an ethereum pubsub rpc call
    async fn on_pub_sub(&self, pubsub: EthPubSub, cx: PubSubContext<Self>) -> ResponseResult {
        let id = SubscriptionId::random_hex();
        trace!(target: "rpc::ws", "received pubsub request {:?}", pubsub);
        match pubsub {
            EthPubSub::EthUnSubscribe(id) => {
                trace!(target: "rpc::ws", "canceling subscription {:?}", id);
                let canceled = cx.remove_subscription(&id).is_some();
                ResponseResult::Success(canceled.into())
            }
            EthPubSub::EthSubscribe(kind, raw_params) => {
                let filter = match &*raw_params {
                    Params::None => None,
                    Params::Logs(filter) => Some(filter.clone()),
                    Params::Bool(_) => None,
                };
                let params = FilteredParams::new(filter.map(|b| *b));

                let subscription = match kind {
                    SubscriptionKind::Logs => {
                        if raw_params.is_bool() {
                            return ResponseResult::Error(RpcError::invalid_params(
                                "Expected params for logs subscription",
                            ));
                        }

                        trace!(target: "rpc::ws", "received logs subscription {:?}", params);
                        let blocks = self.api.new_block_notifications();
                        let storage = self.api.storage_info();
                        EthSubscription::Logs(Box::new(LogsSubscription {
                            blocks,
                            storage,
                            filter: params,
                            queued: Default::default(),
                            id: id.clone(),
                        }))
                    }
                    SubscriptionKind::NewHeads => {
                        trace!(target: "rpc::ws", "received header subscription");
                        let blocks = self.api.new_block_notifications();
                        let storage = self.api.storage_info();
                        EthSubscription::Header(blocks, storage, id.clone())
                    }
                    SubscriptionKind::NewPendingTransactions => {
                        trace!(target: "rpc::ws", "received pending transactions subscription");
                        match *raw_params {
                            Params::Bool(true) => EthSubscription::FullPendingTransactions(
                                self.api.full_pending_transactions(),
                                id.clone(),
                            ),
                            Params::Bool(false) | Params::None => {
                                EthSubscription::PendingTransactions(
                                    self.api.new_ready_transactions(),
                                    id.clone(),
                                )
                            }
                            _ => {
                                return ResponseResult::Error(RpcError::invalid_params(
                                    "Expected boolean parameter for newPendingTransactions",
                                ));
                            }
                        }
                    }
                    SubscriptionKind::Syncing => {
                        return RpcError::internal_error_with("Not implemented").into();
                    }
                };

                cx.add_subscription(id.clone(), subscription);

                trace!(target: "rpc::ws", "created new subscription: {:?}", id);
                to_rpc_result(id)
            }
        }
    }
}

#[async_trait::async_trait]
impl PubSubRpcHandler for PubSubEthRpcHandler {
    type Request = EthRpcCall;
    type SubscriptionId = SubscriptionId;
    type Subscription = EthSubscription;

    async fn on_request(&self, request: Self::Request, cx: PubSubContext<Self>) -> ResponseResult {
        trace!(target: "rpc", "received pubsub request {:?}", request);
        match request {
            EthRpcCall::Request(request) => self.api.execute(*request).await,
            EthRpcCall::PubSub(pubsub) => self.on_pub_sub(pubsub, cx).await,
        }
    }
}
