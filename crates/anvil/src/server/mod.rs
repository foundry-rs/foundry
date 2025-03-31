//! Contains the code to launch an Ethereum RPC server.

use crate::{
    eth::EthApi,
    NodeConfig,
    IpcTask,
};
use anvil_server::{
    http_ws_router,
    ipc::IpcEndpoint,
    ServerConfig,
};
use axum::Router;
use std::{io, future::Future, net::SocketAddr};
use futures::StreamExt;
use handler::{HttpEthRpcHandler, PubSubEthRpcHandler};
use tokio::net::TcpListener;

pub mod error;
pub mod handler;
pub mod server_config;

/// Configures a server that handles [`EthApi`] related JSON-RPC calls via HTTP and WS.
///
/// The returned future creates a new server, binding it to the given address, which returns another
/// future that runs it.
pub async fn serve(
    addr: SocketAddr,
    api: EthApi,
    config: ServerConfig,
) -> io::Result<impl Future<Output = io::Result<()>>> {
    let tcp_listener = TcpListener::bind(addr).await?;
    Ok(serve_on(tcp_listener, api, config))
}

/// Configures a server that handles [`EthApi`] related JSON-RPC calls via HTTP and WS.
pub async fn serve_on(
    tcp_listener: TcpListener,
    api: EthApi,
    config: ServerConfig,
) -> io::Result<()> {
    axum::serve(tcp_listener, router(api, config).into_make_service()).await
}

/// Configures an [`axum::Router`] that handles [`EthApi`] related JSON-RPC calls via HTTP and WS.
pub fn router(api: EthApi, config: ServerConfig) -> Router {
    let http = HttpEthRpcHandler::new(api.clone()).with_headers(config.anvil_headers.clone());
    let ws = PubSubEthRpcHandler::new(api);
    http_ws_router(config, http, ws)
}

/// Spawns the IPC server endpoint
pub fn spawn_ipc(api: EthApi, path: String) -> IpcTask {
    try_spawn_ipc(api, path).expect("Failed to spawn IPC server")
}

/// Attempts to spawn the IPC server endpoint
pub fn try_spawn_ipc(api: EthApi, path: String) -> io::Result<IpcTask> {
    let handler = PubSubEthRpcHandler::new(api);
    let endpoint = IpcEndpoint::new(handler, path);
    let incoming = endpoint.incoming()?;
    
    Ok(tokio::spawn(async move {
        let mut incoming = Box::pin(incoming);
        while let Some(connection) = incoming.next().await {
            tokio::spawn(connection);
        }
    }))
}
