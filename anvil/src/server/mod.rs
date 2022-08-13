//! Contains the code to launch an ethereum RPC-Server
use crate::EthApi;
use anvil_server::{AnvilServer, ServerConfig};
use handler::{HttpEthRpcHandler, WsEthRpcHandler};
use std::net::SocketAddr;

mod handler;

/// Configures an [axum::Server] that handles [EthApi] related JSON-RPC calls via HTTP and WS
pub fn serve(addr: SocketAddr, api: EthApi, config: ServerConfig) -> AnvilServer {
    let http = HttpEthRpcHandler::new(api.clone());
    let ws = WsEthRpcHandler::new(api);
    anvil_server::serve_http_ws(addr, config, http, ws)
}
