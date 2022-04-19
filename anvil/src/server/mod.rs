//! Contains the code to launch an ethereum RPC-Server
use crate::EthApi;
use handler::{HttpEthRpcHandler, WsEthRpcHandler};
use std::{future::Future, net::SocketAddr};

mod handler;

/// Configures an [axum::Server] that handles [EthApi] related JSON-RPC calls via HTTP and WS
pub fn serve(addr: SocketAddr, api: EthApi) -> impl Future<Output = hyper::Result<()>> {
    let http = HttpEthRpcHandler::new(api.clone());
    let ws = WsEthRpcHandler::new(api);
    anvil_server::serve_http_ws(addr, http, ws)
}
