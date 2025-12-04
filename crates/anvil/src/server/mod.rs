//! This module provides the infrastructure to launch an Ethereum JSON-RPC server
//! (via HTTP, WebSocket, and IPC) and Beacon Node REST API.

use crate::{EthApi, IpcTask};
use anvil_server::{ServerConfig, ipc::IpcEndpoint};
use axum::Router;
use futures::StreamExt;
use rpc_handlers::{HttpEthRpcHandler, PubSubEthRpcHandler};
use std::{io, net::SocketAddr, pin::pin};
use tokio::net::TcpListener;

mod beacon;
mod rpc_handlers;

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

/// Configures an [`axum::Router`] that handles [`EthApi`] related JSON-RPC calls via HTTP and WS,
/// and Beacon REST API calls.
pub fn router(api: EthApi, config: ServerConfig) -> Router {
    let http = HttpEthRpcHandler::new(api.clone());
    let ws = PubSubEthRpcHandler::new(api.clone());

    // JSON-RPC router
    let rpc_router = anvil_server::http_ws_router(config, http, ws);

    // Beacon REST API router
    let beacon_router = beacon::router(api);

    // Merge the routers
    rpc_router.merge(beacon_router)
}

/// Launches an ipc server at the given path in a new task
///
/// # Panics
///
/// Panics if setting up the IPC connection was unsuccessful.
#[track_caller]
pub fn spawn_ipc(api: EthApi, path: String) -> IpcTask {
    try_spawn_ipc(api, path).expect("failed to establish ipc connection")
}

/// Launches an ipc server at the given path in a new task.
pub fn try_spawn_ipc(api: EthApi, path: String) -> io::Result<IpcTask> {
    let handler = PubSubEthRpcHandler::new(api);
    let ipc = IpcEndpoint::new(handler, path);
    let incoming = ipc.incoming()?;

    let task = tokio::task::spawn(async move {
        let mut incoming = pin!(incoming);
        while let Some(stream) = incoming.next().await {
            trace!(target: "ipc", "new ipc connection");
            tokio::task::spawn(stream);
        }
    });

    Ok(task)
}
