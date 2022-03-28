//! Bootstrap [axum] servers

use crate::eth::EthApi;
use axum::{handler::post, AddExtensionLayer, Router, Server};
use std::{future::Future, net::SocketAddr};
use tower_http::trace::TraceLayer;

/// handlers for axum server
mod handler;

/// Configures an [axum::Server] that handles [EthApi] related JSON-RPC calls via HTTP
pub fn http_server(addr: SocketAddr, api: EthApi) -> impl Future<Output = hyper::Result<()>> {
    let svc = Router::new()
        .route("/", post(handler::handle_rpc))
        .layer(AddExtensionLayer::new(api))
        .layer(TraceLayer::new_for_http())
        .into_make_service();
    Server::bind(&addr).serve(svc)
}

/// Configures an [axum::Server] that handles [EthApi] related JSON-RPC calls via Websockets
pub fn ws_server(addr: SocketAddr, api: EthApi) -> impl Future<Output = hyper::Result<()>> {
    let svc = Router::new()
        .route("/", post(handler::ws_handler))
        .layer(AddExtensionLayer::new(api))
        .layer(TraceLayer::new_for_http())
        .into_make_service();
    Server::bind(&addr).serve(svc)
}

pub(crate) fn init_tracing() {
    tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}
