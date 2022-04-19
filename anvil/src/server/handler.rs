//! Contains RPC handlers
use crate::EthApi;
use anvil_core::eth::{subscription::SubscriptionId, EthPubSub, EthRequest, EthRpcCall};
use anvil_rpc::response::ResponseResult;
use anvil_server::{RpcHandler, WsContext, WsRpcHandler};
use futures::Stream;
use std::{
    pin::Pin,
    task::{Context, Poll},
};

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

/// A `RpcHandler` that expects `EthRequest` rpc calls and `EthPubSub` via websocket
#[derive(Clone)]
pub struct WsEthRpcHandler {
    /// Access to the node
    api: EthApi,
}

impl WsEthRpcHandler {
    /// Creates a new instance of the handler using the given `EthApi`
    pub fn new(api: EthApi) -> Self {
        Self { api }
    }

    /// Invoked for an ethereum pubsub rpc call
    async fn on_pub_sub(&self, pubsub: EthPubSub, cx: WsContext<Self>) -> ResponseResult {
        match pubsub {
            EthPubSub::EthSubscribe(_kind, _params) => {
                todo!()
            }
            EthPubSub::EthUnSubscribe(id) => {
                let canceled = cx.remove_subscription(&id).is_some();
                ResponseResult::Success(canceled.into())
            }
        }
    }
}

#[async_trait::async_trait]
impl WsRpcHandler for WsEthRpcHandler {
    type Request = EthRpcCall;
    type SubscriptionId = SubscriptionId;
    type Subscription = EthSubscription;

    async fn on_request(&self, request: Self::Request, cx: WsContext<Self>) -> ResponseResult {
        match request {
            EthRpcCall::Request(request) => self.api.execute(request).await,
            EthRpcCall::PubSub(pubsub) => self.on_pub_sub(pubsub, cx).await,
        }
    }
}

/// Represents an ethereum subscription
pub struct EthSubscription;

impl Stream for EthSubscription {
    type Item = ResponseResult;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}
