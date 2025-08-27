//! Contains RPC handlers
use anvil_core::eth::{subscription::SubscriptionId, EthPubSub, EthRequest, EthRpcCall};
use anvil_rpc::{error::RpcError, response::ResponseResult};
use anvil_server::{PubSubContext, PubSubRpcHandler, RpcHandler};
use futures::{channel::oneshot, SinkExt};

use crate::{
    api_server::{ApiHandle, ApiRequest},
    pubsub::EthSubscription,
};

/// A `RpcHandler` that expects `EthRequest` rpc calls via http
#[derive(Clone)]
pub struct HttpEthRpcHandler {
    api_handle: ApiHandle,
}

impl HttpEthRpcHandler {
    /// Creates a new instance of the handler using the given `ApiHandle`
    pub fn new(api_handle: ApiHandle) -> Self {
        Self { api_handle }
    }
}

#[async_trait::async_trait]
impl RpcHandler for HttpEthRpcHandler {
    type Request = EthRequest;

    async fn on_request(&self, request: Self::Request) -> ResponseResult {
        let (tx, rx) = oneshot::channel();
        self.api_handle
            .clone()
            .send(ApiRequest { req: request, resp_sender: tx })
            .await
            .expect("Dropped receiver");

        rx.await.expect("Dropped sender")
    }
}

/// A `RpcHandler` that expects `EthRequest` rpc calls and `EthPubSub` via pubsub connection
#[derive(Clone)]
pub struct PubSubEthRpcHandler {
    api_handle: ApiHandle,
}

impl PubSubEthRpcHandler {
    /// Creates a new instance of the handler using the given `ApiHandle`
    pub fn new(api_handle: ApiHandle) -> Self {
        Self { api_handle }
    }

    /// Invoked for an ethereum pubsub rpc call
    async fn on_pub_sub(&self, _pubsub: EthPubSub, _cx: PubSubContext<Self>) -> ResponseResult {
        ResponseResult::Error(RpcError::invalid_params("Not implemented"))
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
            EthRpcCall::Request(request) => {
                let (tx, rx) = oneshot::channel();
                self.api_handle
                    .clone()
                    .send(ApiRequest { req: *request, resp_sender: tx })
                    .await
                    .expect("Dropped receiver");

                rx.await.expect("Dropped sender")
            }
            EthRpcCall::PubSub(pubsub) => self.on_pub_sub(pubsub, cx).await,
        }
    }
}
