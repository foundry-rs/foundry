use anvil_core::eth::subscription::SubscriptionId;
use anvil_rpc::{request::Version, response::ResponseResult};
use futures::{ready, Stream};
use serde::Serialize;
use std::{
    pin::Pin,
    task::{Context, Poll},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EthSubscriptionResponse {
    jsonrpc: Version,
    method: &'static str,
    params: EthSubscriptionParams,
}

impl EthSubscriptionResponse {
    pub fn new(params: EthSubscriptionParams) -> Self {
        Self { jsonrpc: Version::V2, method: "eth_subscription", params }
    }
}

/// Represents the `params` field of an `eth_subscription` event
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EthSubscriptionParams {
    subscription: SubscriptionId,
    #[serde(flatten)]
    result: ResponseResult,
}

/// Represents an ethereum Websocket subscription
#[derive(Debug)]
pub enum EthSubscription {
    // Unimplemented
}

impl EthSubscription {
    fn poll_response(&mut self, _cx: &mut Context<'_>) -> Poll<Option<EthSubscriptionResponse>> {
        // Unimplemented
        Poll::Pending
    }
}

impl Stream for EthSubscription {
    type Item = serde_json::Value;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();
        match ready!(pin.poll_response(cx)) {
            None => Poll::Ready(None),
            Some(res) => Poll::Ready(Some(serde_json::to_value(res).expect("can't fail;"))),
        }
    }
}
