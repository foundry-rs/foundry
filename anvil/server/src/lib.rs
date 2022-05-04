//! Bootstrap [axum] RPC servers

#![deny(missing_docs, unsafe_code, unused_crate_dependencies)]

use anvil_rpc::{
    error::RpcError,
    request::RpcMethodCall,
    response::{ResponseResult, RpcResponse},
};
use axum::{
    extract::Extension,
    http::{header, HeaderValue, Method},
    routing::post,
    Router, Server,
};
use serde::de::DeserializeOwned;
use std::{fmt, future::Future, net::SocketAddr};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{trace, warn};

mod config;
/// handlers for axum server
mod handler;
mod ws;
pub use crate::ws::{WsContext, WsRpcHandler};
pub use config::ServerConfig;

/// Configures an [axum::Server] that handles RPC-Calls, both HTTP requests and requests via
/// websocket
pub fn serve_http_ws<Http, Ws>(
    addr: SocketAddr,
    config: ServerConfig,
    http: Http,
    ws: Ws,
) -> impl Future<Output = hyper::Result<()>>
where
    Http: RpcHandler,
    Ws: WsRpcHandler,
{
    let ServerConfig { allow_origin, no_cors } = config;

    let svc = Router::new()
        .route("/", post(handler::handle::<Http>).get(ws::handle_ws::<Ws>))
        .layer(Extension(http))
        .layer(Extension(ws))
        .layer(TraceLayer::new_for_http());

    let svc = if no_cors {
        svc
    } else {
        svc.layer(
            // see https://docs.rs/tower-http/latest/tower_http/cors/index.html
            // for more details
            CorsLayer::new()
                .allow_origin(allow_origin.0)
                .allow_headers(vec![header::CONTENT_TYPE])
                .allow_methods(vec![Method::GET, Method::POST]),
        )
    }
    .into_make_service();
    Server::bind(&addr).serve(svc)
}

/// Configures an [axum::Server] that handles RPC-Calls listing for POST on `/`
pub fn serve_http<Http>(
    addr: SocketAddr,
    config: ServerConfig,
    http: Http,
) -> impl Future<Output = hyper::Result<()>>
where
    Http: RpcHandler,
{
    let ServerConfig { allow_origin, no_cors } = config;

    let svc = Router::new()
        .route("/", post(handler::handle::<Http>))
        .layer(Extension(http))
        .layer(TraceLayer::new_for_http());
    let svc = if no_cors {
        svc
    } else {
        svc.layer(
            // see https://docs.rs/tower-http/latest/tower_http/cors/index.html
            // for more details
            CorsLayer::new()
                .allow_origin(allow_origin.0)
                .allow_headers(vec![header::CONTENT_TYPE])
                .allow_methods(vec![Method::GET, Method::POST]),
        )
    }
    .into_make_service();

    Server::bind(&addr).serve(svc)
}

/// Helper trait that is used to execute ethereum rpc calls
#[async_trait::async_trait]
pub trait RpcHandler: Clone + Send + Sync + 'static {
    /// The request type to expect
    type Request: DeserializeOwned + Send + Sync + fmt::Debug;

    /// Invoked when the request was received
    async fn on_request(&self, request: Self::Request) -> ResponseResult;

    /// Invoked for every incoming `RpcMethodCall`
    ///
    /// This will attempt to deserialize a `{ "method" : "<name>", "params": "<params>" }` message
    /// into the `Request` type of this handler. If a `Request` instance was deserialized
    /// successfully, [`Self::on_request`] will be invoked.
    ///
    /// **Note**: override this function if the expected `Request` deviates from `{ "method" :
    /// "<name>", "params": "<params>" }`
    async fn on_call(&self, call: RpcMethodCall) -> RpcResponse {
        trace!(target: "rpc", "received method call {:?}", call);
        let RpcMethodCall { method, params, id, .. } = call;

        let params: serde_json::Value = params.into();
        let m = method.clone();
        let call = serde_json::json!({
            "method": method,
            "params": params
        });

        match serde_json::from_value::<Self::Request>(call) {
            Ok(req) => {
                trace!(target: "rpc", "received handler request {:?}", req);
                let result = self.on_request(req).await;
                trace!(target: "rpc", "prepared rpc result {:?}", result);
                RpcResponse::new(id, result)
            }
            Err(err) => {
                let msg = err.to_string();
                warn!(target: "rpc", "failed to deserialize method `{}`: {}", m, msg);
                if msg.contains("unknown variant") {
                    RpcResponse::new(id, RpcError::method_not_found())
                } else {
                    RpcResponse::new(id, RpcError::invalid_params(msg))
                }
            }
        }
    }
}
