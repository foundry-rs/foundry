//! Bootstrap [axum] servers

use crate::eth::EthApi;
use axum::{
    extract::{Extension},
    routing::{post},
    Router, Server,
};
use std::{future::Future, net::SocketAddr};
use tower_http::trace::TraceLayer;

/// handlers for axum server
mod handler;

/// Configures an [axum::Server] that handles [EthApi] related JSON-RPC calls via HTTP
// TODO unify http and ws with wrapper type that impl FromRequest and returns based on the uri
pub fn http_server(addr: SocketAddr, api: EthApi) -> impl Future<Output = hyper::Result<()>> {
    let svc = Router::new()
        .route("/", post(handler::handle_rpc))
        .layer(Extension(api))
        .layer(TraceLayer::new_for_http())
        .into_make_service();
    Server::bind(&addr).serve(svc)
}

/// Configures an [axum::Server] that handles [EthApi] related JSON-RPC calls via Websockets
pub fn ws_server(addr: SocketAddr, api: EthApi) -> impl Future<Output = hyper::Result<()>> {
    let svc = Router::new()
        .route("/", post(handler::ws_handler))
        .layer(Extension(api))
        .layer(TraceLayer::new_for_http())
        .into_make_service();
    Server::bind(&addr).serve(svc)
}

pub(crate) fn init_tracing() {
    tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}
