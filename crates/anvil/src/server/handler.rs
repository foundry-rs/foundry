//! Contains RPC handlers
use crate::{
    eth::error::to_rpc_result,
    pubsub::{EthSubscription, LogsSubscription},
    EthApi,
};
use alloy_rpc_types::{
    pubsub::{Params, SubscriptionKind},
    FilteredParams,
};
use anvil_core::eth::{subscription::SubscriptionId, EthPubSub, EthRequest, EthRpcCall};
use anvil_rpc::{error::RpcError, response::ResponseResult};
use anvil_server::{PubSubContext, PubSubRpcHandler, RpcHandler};

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
                let filter = match *params {
                    Params::None => None,
                    Params::Logs(filter) => Some(*filter),
                    Params::Bool(_) => {
                        return ResponseResult::Error(RpcError::invalid_params(
                            "Expected params for logs subscription",
                        ))
                    }
                };
                let params = FilteredParams::new(filter);

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
