use super::ApiRequest;
use crate::substrate_node::service::Service;
use anvil_core::eth::EthRequest;
use anvil_rpc::{error::RpcError, response::ResponseResult};
use foundry_common::sh_println;
use futures::{channel::mpsc, StreamExt};

pub struct ApiServer {
    req_receiver: mpsc::Receiver<ApiRequest>,
}

impl ApiServer {
    pub fn new(_substrate_service: &Service, req_receiver: mpsc::Receiver<ApiRequest>) -> Self {
        Self { req_receiver }
    }

    pub async fn run(mut self) {
        while let Some(msg) = self.req_receiver.next().await {
            sh_println!("GOT REQUEST: {:?}", msg.req).unwrap();

            let resp = self.execute(msg.req).await;

            msg.resp_sender.send(resp).expect("Dropped receiver");
        }
    }

    pub async fn execute(&mut self, _req: EthRequest) -> ResponseResult {
        ResponseResult::Error(RpcError::internal_error())
    }
}
