//! Bootstrap [axum] RPC servers.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate tracing;

use anvil_rpc::{
    error::RpcError,
    request::RpcMethodCall,
    response::{ResponseResult, RpcResponse},
};
use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, Method, header},
    routing::{MethodRouter, post},
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

/// Helper trait that is used to execute ethereum rpc calls
pub trait RpcHandler: Clone + Send + Sync + 'static {
    /// The request type to expect
    type Request: DeserializeOwned + Send + Sync + fmt::Debug;

    /// Invoked when the request was received
    fn on_request(&self, request: Self::Request) -> impl Future<Output = ResponseResult> + Send;

    /// Invoked for every incoming [`RpcMethodCall`]. Notifications are adapted to method calls
    /// with an [`anvil_rpc::request::Id::Null`] identifier, and their responses are discarded.
    ///
    /// This will attempt to deserialize a `{ "method" : "<name>", "params": "<params>" }` message
    /// into the `Request` type of this handler. If a `Request` instance was deserialized
    /// successfully, [`Self::on_request`] will be invoked.
    ///
    /// **Note**: override this function if the expected `Request` deviates from `{ "method" :
    /// "<name>", "params": "<params>" }`
    fn on_call(&self, call: RpcMethodCall) -> impl Future<Output = RpcResponse> + Send {
        async move {
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
                    let method_not_found = serde_json::from_value::<Self::Request>(
                        serde_json::json!({ "method": &method }),
                    )
                    .is_err_and(|err| err.to_string().contains("unknown variant"));
                    if method_not_found {
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
}

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
