use crate::{
    server::{
        ws::{WsContext, WsRpcHandler},
        RpcHandler,
    },
    EthApi,
};
use anvil_core::eth::{EthPubSub, EthRequest, EthRpcCall};
use anvil_rpc::response::ResponseResult;
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

// === impl WsEthRpcHandler ===

impl WsEthRpcHandler {
    /// Creates a new instance of the handler using the given `EthApi`
    pub fn new(api: EthApi) -> Self {
        Self { api }
    }

    /// Invoked for an ethereum pubsub rpc call
    async fn on_pub_sub(
        &self,
        _pubsub: EthPubSub,
        _cx: WsContext<<Self as WsRpcHandler>::Subscription>,
    ) -> ResponseResult {
        todo!()
    }
}

#[async_trait::async_trait]
impl WsRpcHandler for WsEthRpcHandler {
    type Request = EthRpcCall;
    type Subscription = EthSubscription;

    async fn on_request(
        &self,
        request: Self::Request,
        cx: WsContext<Self::Subscription>,
    ) -> ResponseResult {
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
