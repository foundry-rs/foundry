use crate::{logging::LoggingManager, substrate_node::service::Service};
use anvil_core::eth::EthRequest;
use anvil_rpc::response::ResponseResult;
use futures::channel::{mpsc, oneshot};
use server::ApiServer;

pub mod error;
pub mod revive_conversions;
mod server;

pub type ApiHandle = mpsc::Sender<ApiRequest>;

pub struct ApiRequest {
    pub req: EthRequest,
    pub resp_sender: oneshot::Sender<ResponseResult>,
}

pub fn spawn(substrate_service: &Service, logging_manager: LoggingManager) -> ApiHandle {
    let (api_handle, receiver) = mpsc::channel(100);

    let service = substrate_service.clone();
    substrate_service.spawn_handle.spawn("anvil-api-server", "anvil", async move {
        let api_server = ApiServer::new(service, receiver, logging_manager)
            .await
            .unwrap_or_else(|err| panic!("Failed to spawn the API server: {err}"));
        api_server.run().await;
    });

    api_handle
}
