//! Bootstrap [axum] RPC servers.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

#[macro_use]
extern crate tracing;

use anvil_rpc::{
    error::RpcError,
    request::RpcMethodCall,
    response::{ResponseResult, RpcResponse},
};
use axum::{
    extract::DefaultBodyLimit,
    http::{header, HeaderValue, Method},
    routing::{post, MethodRouter},
    Router,
};
use serde::de::DeserializeOwned;
use std::fmt;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

mod config;
pub use config::ServerConfig;

mod error;
mod handler;

mod pubsub;
pub use pubsub::{PubSubContext, PubSubRpcHandler};

mod ws;

#[cfg(feature = "ipc")]
pub mod ipc;

/// Configures an [`axum::Router`] that handles JSON-RPC calls via both HTTP and WS.
pub fn http_ws_router<Http, Ws>(config: ServerConfig, http: Http, ws: Ws) -> Router
where
    Http: RpcHandler,
    Ws: PubSubRpcHandler,
{
    router_inner(config, post(handler::handle).get(ws::handle_ws), (http, ws))
}

/// Configures an [`axum::Router`] that handles JSON-RPC calls via HTTP.
pub fn http_router<Http>(config: ServerConfig, http: Http) -> Router
where
    Http: RpcHandler,
{
    router_inner(config, post(handler::handle), (http, ()))
}

fn router_inner<S: Clone + Send + Sync + 'static>(
    config: ServerConfig,
    root_method_router: MethodRouter<S>,
    state: S,
) -> Router {
    let ServerConfig { allow_origin, no_cors, no_request_size_limit } = config;

    let mut router = Router::new()
        .route("/", root_method_router)
        .with_state(state)
        .layer(TraceLayer::new_for_http());
    if !no_cors {
        // See [`tower_http::cors`](https://docs.rs/tower-http/latest/tower_http/cors/index.html)
        // for more details.
        router = router.layer(
            CorsLayer::new()
                .allow_origin(allow_origin.0)
                .allow_headers([header::CONTENT_TYPE])
                .allow_methods([Method::GET, Method::POST]),
        );
    }
    if no_request_size_limit {
        router = router.layer(DefaultBodyLimit::disable());
    }
    router
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
        trace!(target: "rpc",  id = ?call.id , method = ?call.method, params = ?call.params, "received method call");
        let RpcMethodCall { method, params, id, .. } = call;

        let params: serde_json::Value = params.into();
        let call = serde_json::json!({
            "method": &method,
            "params": params
        });

        match serde_json::from_value::<Self::Request>(call) {
            Ok(req) => {
                let result = self.on_request(req).await;
                RpcResponse::new(id, result)
            }
            Err(err) => {
                let err = err.to_string();
                if err.contains("unknown variant") {
                    error!(target: "rpc", ?method, "failed to deserialize method due to unknown variant");
                    RpcResponse::new(id, RpcError::method_not_found())
                } else {
                    error!(target: "rpc", ?method, ?err, "failed to deserialize method");
                    RpcResponse::new(id, RpcError::invalid_params(err))
                }
            }
        }
    }
}
