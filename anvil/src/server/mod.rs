//! Contains the code to launch an ethereum RPC-Server
use crate::EthApi;
use anvil_server::{ipc::IpcEndpoint, AnvilServer, ServerConfig};
use handler::{HttpEthRpcHandler, PubSubEthRpcHandler};
use std::net::SocketAddr;
use tokio::{io, task::JoinHandle};

mod handler;

pub mod error;

/// Configures an [axum::Server] that handles [EthApi] related JSON-RPC calls via HTTP and WS
pub fn serve(addr: SocketAddr, api: EthApi, config: ServerConfig) -> AnvilServer {
    let http = HttpEthRpcHandler::new(api.clone());
    let ws = PubSubEthRpcHandler::new(api);
    anvil_server::serve_http_ws(addr, config, http, ws)
}

/// Launches an ipc server at the given path in a new task
pub fn spawn_ipc(api: EthApi, path: impl Into<String>) -> JoinHandle<io::Result<()>> {
    let path = path.into();
    tokio::task::spawn(async move {
        let handler = PubSubEthRpcHandler::new(api);
        let ipc = IpcEndpoint::new(handler, path);
        ipc.listen().await;

        Ok(())
    })
}
