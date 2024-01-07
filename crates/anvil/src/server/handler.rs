//! Contains RPC handlers
use crate::{
    eth::error::to_rpc_result,
    pubsub::{EthSubscription, LogsSubscription},
    EthApi,
};
use crate::engine::EngineApi;
use anvil_core::{eth::{
    subscription::{SubscriptionId, SubscriptionKind},
    EthPubSub, EthRequest, EthRpcCall,
}, engine::EngineRequest};
use anvil_rpc::{error::RpcError, response::ResponseResult};
use anvil_server::{PubSubContext, PubSubRpcHandler, RpcHandler};
use ethers::types::FilteredParams;

/// A `RpcHandler` that expects `EthRequest` rpc calls via http
#[derive(Clone)]
pub struct HttpEthRpcHandler {
    /// Access to the node
    api: EthApi,
}

// === impl WsEthRpcHandler ===

impl HttpEthRpcHandler {
    /// Creates a new instance of the handler using the given `EthApi`
    pub fn new(api: EthApi) -> Self {
        Self { api }
    }
}

#[async_trait::async_trait]
impl RpcHandler for HttpEthRpcHandler {
    type Request = EthRequest;

    async fn on_request(&self, request: Self::Request) -> ResponseResult {
        self.api.execute(request).await
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
            EthPubSub::EthSubscribe(kind, params) => {
                let params = FilteredParams::new(params.filter);

                let subscription = match kind {
                    SubscriptionKind::Logs => {
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
                        EthSubscription::PendingTransactions(
                            self.api.new_ready_transactions(),
                            id.clone(),
                        )
                    }
                    SubscriptionKind::Syncing => {
                        return RpcError::internal_error_with("Not implemented").into()
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


//////////////////////////////////
/// 
/// /// A `RpcHandler` that expects `EngineApi` rpc calls via http
#[derive(Clone)]
pub struct HttpEngineRpcHandler {
    /// Access to the node
    api: EngineApi,
}

// === impl WsEthRpcHandler ===

impl HttpEngineRpcHandler {
    /// Creates a new instance of the handler using the given `EngineApi`
    pub fn new(api: EngineApi) -> Self {
        Self { api }
    }
}

#[async_trait::async_trait]
impl RpcHandler for HttpEngineRpcHandler {
    type Request = EngineRequest;

    async fn on_request(&self, request: Self::Request) -> ResponseResult {
        self.api.execute(request).await;
        ResponseResult::Success(serde_json::Value::from(1))
    }
}

/// A `RpcHandler` that expects `EngineApi` rpc calls and `EthPubSub` via pubsub connection
#[derive(Clone)]
pub struct PubSubEngineRpcHandler {
    /// Access to the node
    api: EngineApi,
}

impl PubSubEngineRpcHandler {
    /// Creates a new instance of the handler using the given `EthApi`
    pub fn new(api: EngineApi) -> Self {
        Self { api }
    }

    /// Invoked for an ethereum pubsub rpc call
    async fn on_pub_sub(&self, pubsub: EthPubSub, cx: PubSubContext<Self>) -> ResponseResult {
        // ResponseResult::Error(RpcError::internal_error_with("Not implemented").into())
        ResponseResult::Success(serde_json::Value::from(1))
    }
}

#[async_trait::async_trait]
impl PubSubRpcHandler for PubSubEngineRpcHandler {
    type Request = EthRpcCall;
    type SubscriptionId = SubscriptionId;
    type Subscription = EthSubscription;

    async fn on_request(&self, request: Self::Request, cx: PubSubContext<Self>) -> ResponseResult {
        ResponseResult::Success(serde_json::Value::from(1))
    }
}

