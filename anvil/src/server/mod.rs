//! Bootstrap [axum] servers

use crate::eth::EthApi;
use anvil_core::eth::EthRequest;
use anvil_rpc::response::ResponseResult;
use axum::{extract::Extension, routing::post, Router, Server};
use serde::de::DeserializeOwned;
use std::{future::Future, net::SocketAddr};
use tower_http::trace::TraceLayer;

/// handlers for axum server
mod handler;
mod http;
mod ws;

/// Configures an [axum::Server] that handles [EthApi] related JSON-RPC calls via HTTP
pub fn serve(addr: SocketAddr, api: EthApi) -> impl Future<Output = hyper::Result<()>> {
    let svc = Router::new()
        .route("/", post(handler::handle_rpc).get(ws::ws_handler))
        .layer(Extension(api))
        .layer(TraceLayer::new_for_http())
        .into_make_service();
    Server::bind(&addr).serve(svc)
}

/// Helper trait that is used to execute ethereum rpc calls
#[async_trait::async_trait]
pub trait RpcHandler {
    /// The request type to expect
    type Request: DeserializeOwned;

    /// Invoked when the request was a
    async fn on_request(&self, request: Self::Request) -> ResponseResult;
}
