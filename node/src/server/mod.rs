//! Bootstrap [axum] servers

use crate::eth::EthApi;
use axum::{extract::Extension, routing::post, Router, Server};
use std::{future::Future, net::SocketAddr};
use tower_http::trace::TraceLayer;

/// handlers for axum server
mod handler;

/// Configures an [axum::Server] that handles [EthApi] related JSON-RPC calls via HTTP
pub fn serve(addr: SocketAddr, api: EthApi) -> impl Future<Output = hyper::Result<()>> {
    let svc = Router::new()
        .route("/", post(handler::handle_rpc).get(handler::ws_handler))
        .layer(Extension(api))
        .layer(TraceLayer::new_for_http())
        .into_make_service();
    Server::bind(&addr).serve(svc)
}
