//! Contains the code to launch an Ethereum RPC server.

use anvil_server::{ServerConfig, ipc::IpcEndpoint};
use axum::Router;
use futures::StreamExt;
use handler::{HttpEthRpcHandler, PubSubEthRpcHandler};
use polkadot_sdk::sc_service::TaskManager;
use std::{io, net::SocketAddr, pin::pin};
use tokio::net::TcpListener;

use crate::api_server::ApiHandle;

pub mod error;
mod handler;

/// Configures a server that handles JSON-RPC calls via HTTP and WS.
///
/// The returned future creates a new server, binding it to the given address, which returns another
/// future that runs it.
pub async fn serve(
    addr: SocketAddr,
    config: ServerConfig,
    api_handle: ApiHandle,
) -> io::Result<impl Future<Output = io::Result<()>>> {
    let tcp_listener = TcpListener::bind(addr).await?;
    Ok(serve_on(tcp_listener, config, api_handle))
}

/// Configures a server that handles JSON-RPC calls via HTTP and WS.
pub async fn serve_on(
    tcp_listener: TcpListener,
    config: ServerConfig,
    api_handle: ApiHandle,
) -> io::Result<()> {
    axum::serve(tcp_listener, router(api_handle, config).into_make_service()).await
}

/// Configures an [`axum::Router`] that handles JSON-RPC calls via HTTP and WS.
pub fn router(api_handle: ApiHandle, config: ServerConfig) -> Router {
    let http = HttpEthRpcHandler::new(api_handle.clone());
    let ws = PubSubEthRpcHandler::new(api_handle);
    anvil_server::http_ws_router(config, http, ws)
}

/// Launches an ipc server at the given path in a new task
///
/// # Panics
///
/// Panics if setting up the IPC connection was unsuccessful.
#[track_caller]
pub fn spawn_ipc(task_manager: &TaskManager, path: String, api_handle: ApiHandle) {
    try_spawn_ipc(task_manager, path, api_handle).expect("failed to establish ipc connection")
}

/// Launches an ipc server at the given path in a new task.
pub fn try_spawn_ipc(
    task_manager: &TaskManager,
    path: String,
    api_handle: ApiHandle,
) -> io::Result<()> {
    let handler = PubSubEthRpcHandler::new(api_handle);
    let ipc = IpcEndpoint::new(handler, path);
    let incoming = ipc.incoming()?;

    let spawn_handle = task_manager.spawn_handle();
    let inner_spawn_handle = spawn_handle.clone();

    spawn_handle.spawn("ipc", "anvil", async move {
        let mut incoming = pin!(incoming);
        while let Some(stream) = incoming.next().await {
            trace!(target: "ipc", "new ipc connection");
            inner_spawn_handle.spawn("ipc-connection", "anvil", stream);
        }
    });

    Ok(())
}
