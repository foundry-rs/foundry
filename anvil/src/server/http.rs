use crate::{server::RpcHandler, EthApi};
use anvil_core::eth::EthRequest;
use anvil_rpc::response::ResponseResult;

/// A `RpcHandler` that expects `EthRequest` rpc calls via http
#[derive(Clone)]
pub struct HttpEthRpcHandler {
    /// Access to the node
    api: EthApi,
}

#[async_trait::async_trait]
impl RpcHandler for HttpEthRpcHandler {
    type Request = EthRequest;

    async fn on_request(&self, request: Self::Request) -> ResponseResult {
        todo!()
    }
}
