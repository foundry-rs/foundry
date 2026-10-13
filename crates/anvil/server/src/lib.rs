//! Bootstrap [axum] RPC servers.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[macro_use]
extern crate tracing;

use anvil_rpc::{
    error::RpcError,
    request::{Id, RpcMethodCall},
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

/// Redacts sensitive information from RPC request parameters for logging.
///
/// This function redacts:
/// - Fork URLs in `anvil_reset` and `anvil_setRpcUrl` methods
/// - Signed transaction data in `eth_sendRawTransaction*` methods
fn redact_params(method: &str, params: &serde_json::Value) -> serde_json::Value {
    match method {
        // Redact fork URLs that may contain API keys
        "anvil_reset" | "hardhat_reset" => {
            if let Some(array) = params.as_array() {
                let redacted: Vec<_> = array
                    .iter()
                    .map(|param| {
                        if let Some(obj) = param.as_object() {
                            let mut redacted_obj = obj.clone();
                            // Redact forking.jsonRpcUrl
                            if let Some(forking) = redacted_obj.get_mut("forking")
                                && let Some(forking_obj) = forking.as_object_mut()
                                && let Some(url) =
                                    forking_obj.get("jsonRpcUrl").and_then(|v| v.as_str())
                            {
                                forking_obj.insert(
                                    "jsonRpcUrl".to_string(),
                                    serde_json::Value::String(redact_url_credentials(url)),
                                );
                            }
                            // Also handle legacy top-level jsonRpcUrl
                            if let Some(url) =
                                redacted_obj.get("jsonRpcUrl").and_then(|v| v.as_str())
                            {
                                redacted_obj.insert(
                                    "jsonRpcUrl".to_string(),
                                    serde_json::Value::String(redact_url_credentials(url)),
                                );
                            }
                            serde_json::Value::Object(redacted_obj)
                        } else {
                            param.clone()
                        }
                    })
                    .collect();
                serde_json::Value::Array(redacted)
            } else {
                params.clone()
            }
        }
        "anvil_setRpcUrl" => {
            if let Some(url) = params.as_array().and_then(|a| a.first()).and_then(|v| v.as_str()) {
                serde_json::json!([redact_url_credentials(url)])
            } else {
                params.clone()
            }
        }
        // Redact signed transaction data, but keep transaction hashes visible
        "eth_sendRawTransaction"
        | "eth_sendRawTransactionSync"
        | "eth_sendRawTransactionConditional"
        | "eth_signRawTransaction" => {
            if let Some(array) = params.as_array() {
                if array.is_empty() {
                    params.clone()
                } else {
                    // Keep the first param as "[SIGNED_TX]" for visibility that a tx was sent
                    let mut redacted = vec![serde_json::Value::String("[SIGNED_TX]".to_string())];
                    // Keep other params (e.g., timeout, conditional) visible as they're not
                    // sensitive
                    redacted.extend(array.iter().skip(1).cloned());
                    serde_json::Value::Array(redacted)
                }
            } else {
                params.clone()
            }
        }
        _ => params.clone(),
    }
}

/// Redacts credentials from URLs by replacing API keys/secrets in the path or query.
fn redact_url_credentials(url: &str) -> String {
    // Parse URL and redact sensitive parts
    if let Ok(mut parsed) = url::Url::parse(url) {
        // Redact path segments that look like API keys (alphanumeric strings >= 16 chars)
        let path = parsed.path();
        let segments: Vec<&str> = path.split('/').collect();
        let redacted_segments: Vec<String> = segments
            .iter()
            .map(|seg| {
                // If segment is >= 16 chars and mostly alphanumeric, likely an API key
                if seg.len() >= 16 {
                    let alphanum_count = seg.chars().filter(|c| c.is_alphanumeric()).count();
                    if alphanum_count >= seg.len() * 3 / 4 {
                        return "[REDACTED]".to_string();
                    }
                }
                seg.to_string()
            })
            .collect();
        parsed.set_path(&redacted_segments.join("/"));

        // Redact query parameters that might contain API keys
        if parsed.query().is_some() {
            let pairs: Vec<(String, String)> = parsed
                .query_pairs()
                .map(|(k, v)| {
                    let redacted_val = if v.len() >= 16 {
                        let alphanum_count = v.chars().filter(|c| c.is_alphanumeric()).count();
                        if alphanum_count >= v.len() * 3 / 4 {
                            "[REDACTED]".to_string()
                        } else {
                            v.to_string()
                        }
                    } else {
                        v.to_string()
                    };
                    (k.to_string(), redacted_val)
                })
                .collect();
            parsed.query_pairs_mut().clear();
            for (k, v) in pairs {
                parsed.query_pairs_mut().append_pair(&k, &v);
            }
        }

        parsed.to_string()
    } else {
        // If parsing fails, just return a generic redacted message
        "[REDACTED_URL]".to_string()
    }
}

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
            let RpcMethodCall { method, params, id, .. } = call;

            let params: serde_json::Value = params.into();

            // Log with redacted sensitive parameters
            trace!(target: "rpc", id = ?id, method = ?method, params = ?redact_params(&method, &params), "received method call");

            let call = serde_json::json!({
                "method": &method,
                "params": params
            });

            match serde_json::from_value::<Self::Request>(call) {
                Ok(req) => {
                    let result = self.on_request(req).await;
                    if let ResponseResult::Error(error) = &result {
                        self.on_request_error(&method, &id, error);
                    }
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

    /// Invoked when a deserialized request returns an error from [`Self::on_request`].
    /// Deserialization errors are excluded because their messages can contain request parameters.
    fn on_request_error(&self, _method: &str, _id: &Id, _error: &RpcError) {}
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
