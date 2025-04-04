//! Contains RPC handlers
use crate::{
    eth::EthApi,
    pubsub::{EthSubscription, LogsSubscription},
};
use alloy_rpc_types::{
    pubsub::{Params, SubscriptionKind},
    FilteredParams,
};
use anvil_core::eth::{
    subscription::SubscriptionId,
    EthPubSub,
    EthRequest,
    EthRpcCall,
};
use anvil_rpc::{
    error::RpcError,
    response::ResponseResult,
};
use anvil_server::{
    PubSubContext,
    PubSubRpcHandler,
    RpcHandler,
};
use axum::{
    extract::State,
    http::{Method, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use serde_json::Value;
use std::sync::Arc;

struct SubscriptionIdWrapper(SubscriptionId);

impl From<SubscriptionId> for SubscriptionIdWrapper {
    fn from(id: SubscriptionId) -> Self {
        Self(id)
    }
}

impl From<SubscriptionIdWrapper> for Value {
    fn from(wrapper: SubscriptionIdWrapper) -> Self {
        Value::String(wrapper.0.to_string())
    }
}

/// A `RpcHandler` that expects `EthRequest` rpc calls via http
#[derive(Clone)]
pub struct HttpEthRpcHandler {
    /// The eth api instance that handles the requests
    api: EthApi,
    /// Custom headers to add to responses
    headers: Vec<String>,
}

impl HttpEthRpcHandler {
    /// Creates a new instance of the handler using the given `EthApi`
    pub fn new(api: EthApi) -> Self {
        Self {
            api,
            headers: Vec::new(),
        }
    }

    /// Sets custom headers for responses
    pub fn with_headers(mut self, headers: Vec<String>) -> Self {
        self.headers = headers;
        self
    }
}

#[async_trait::async_trait]
impl RpcHandler for HttpEthRpcHandler {
    type Request = EthRequest;

    async fn on_request(&self, request: Self::Request) -> ResponseResult {
        self.api.execute(request).await
    }

    fn get_anvil_headers(&self) -> Option<&Vec<String>> {
        Some(&self.headers)
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
}

#[async_trait::async_trait]
impl PubSubRpcHandler for PubSubEthRpcHandler {
    type Request = EthRpcCall;
    type SubscriptionId = SubscriptionId;
    type Subscription = EthSubscription;

    async fn on_request(&self, request: Self::Request, cx: PubSubContext<Self>) -> ResponseResult {
        match request {
            EthRpcCall::Request(request) => self.api.execute(*request).await,
            EthRpcCall::PubSub(pubsub) => match pubsub {
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

                    let (subscription, id) = match kind {
                        SubscriptionKind::Logs => {
                            let blocks = self.api.new_block_notifications();
                            let storage = self.api.storage_info();
                            let id = SubscriptionId::random_hex();
                            let subscription = EthSubscription::Logs(Box::new(LogsSubscription {
                                blocks,
                                storage,
                                filter: params,
                                queued: Default::default(),
                                id: id.clone(),
                            }));
                            (subscription, id)
                        }
                        SubscriptionKind::NewHeads => {
                            let blocks = self.api.new_block_notifications();
                            let storage = self.api.storage_info();
                            let id = SubscriptionId::random_hex();
                            let subscription = EthSubscription::Header(blocks, storage, id.clone());
                            (subscription, id)
                        }
                        SubscriptionKind::NewPendingTransactions => {
                            let id = SubscriptionId::random_hex();
                            let subscription = EthSubscription::PendingTransactions(
                                self.api.new_ready_transactions(),
                                id.clone(),
                            );
                            (subscription, id)
                        }
                        SubscriptionKind::Syncing => {
                            return ResponseResult::Error(RpcError::internal_error_with("Not implemented"))
                        }
                    };

                    cx.add_subscription(id.clone(), subscription);
                    ResponseResult::Success(SubscriptionIdWrapper::from(id).into())
                }
                EthPubSub::EthUnSubscribe(id) => {
                    let removed = cx.remove_subscription(&id).is_some();
                    ResponseResult::Success(removed.into())
                }
            },
        }
    }
}
