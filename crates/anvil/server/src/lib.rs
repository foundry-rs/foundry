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
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, Method, header},
    routing::{MethodRouter, post},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::fmt;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

mod config;
pub use config::ServerConfig;

mod error;
mod handler;

mod pubsub;
pub use pubsub::{PubSubContext, PubSubRpcHandler};

mod beacon;
use beacon::beacon_router;

mod ws;

#[cfg(feature = "ipc")]
pub mod ipc;

/// Configures an [`axum::Router`] that handles JSON-RPC calls via both HTTP and WS.
pub fn http_ws_router<Http, Ws, Beacon>(
    config: ServerConfig,
    http: Http,
    ws: Ws,
    beacon: Beacon,
) -> Router
where
    Http: RpcHandler,
    Ws: PubSubRpcHandler,
    Beacon: BeaconApiHandler,
{
    router_inner(config, post(handler::handle).get(ws::handle_ws), (http, ws))
        .nest("/eth", beacon_router(beacon))
}

/// Configures an [`axum::Router`] that handles JSON-RPC calls via HTTP.
pub fn http_router<Http, Beacon>(config: ServerConfig, http: Http, beacon: Beacon) -> Router
where
    Http: RpcHandler,
    Beacon: BeaconApiHandler,
{
    router_inner(config, post(handler::handle), (http, ())).nest("/eth", beacon_router(beacon))
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum BeaconRequest {
    GetBlobSidecarsByBlockId(String),
}

/// Response of a _single_ rpc call
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeaconResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_optimistic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finalized: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl BeaconResponse {
    pub fn success(
        version: Option<String>,
        execution_optimistic: Option<bool>,
        finalized: Option<bool>,
        data: Option<Value>,
    ) -> Self {
        Self { version, execution_optimistic, finalized, data, ..Default::default() }
    }

    pub fn error(code: i32, message: String) -> Self {
        Self { code: Some(code), message: Some(message), ..Default::default() }
    }
}

pub trait BeaconApiHandler: Clone + Send + Sync + 'static {
    fn call(&self, request: BeaconRequest) -> BeaconResponse;
}
