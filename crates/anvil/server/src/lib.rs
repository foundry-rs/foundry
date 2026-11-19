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

/// Redacts sensitive information from RPC request parameters for TRACE logging.
///
/// This function redacts:
/// - Fork URLs in `anvil_reset`, `hardhat_reset`, and `anvil_setRpcUrl` methods.
/// - Signed transaction data in submission, signing, classification, and tracing methods.
fn redact_params(method: &str, params: &serde_json::Value) -> serde_json::Value {
    match method {
        // Redact signed transaction data, but keep transaction hashes visible
        "eth_sendRawTransaction"
        | "eth_sendRawTransactionSync"
        | "eth_sendRawTransactionConditional"
        | "eth_signRawTransaction"
        | "anvil_classifyTransaction"
        | "trace_rawTransaction" => {
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
        _ => {
            let mut redacted = params.clone();
            redact_fork_credentials(method, &mut redacted);
            redacted
        }
    }
}

/// Redacts fork credentials while preserving the rest of a failed request dump.
/// Signed transactions remain available in failed dumps for reproduction.
fn redact_fork_credentials(method: &str, params: &mut serde_json::Value) {
    match method {
        "anvil_reset" | "hardhat_reset" => redact_fork_params(params),
        "anvil_setRpcUrl" => {
            if let Some(serde_json::Value::String(url)) = params.get_mut(0) {
                *url = redact_url_credentials(url);
            }
        }
        _ => {}
    }
}

/// Follows reset wrappers and positional URL slots while preserving other fields.
fn redact_fork_params(param: &mut serde_json::Value) {
    match param {
        serde_json::Value::Array(array) => {
            // Serde accepts Params, tagged forking, and ForkOpts as sequences. Their first
            // element is the next wrapper or the URL; the block number follows the URL.
            if let Some(first) = array.first_mut() {
                redact_fork_params(first);
            }
        }
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "params" | "forking" | "jsonRpcUrl") {
                    redact_fork_params(value);
                }
            }
        }
        serde_json::Value::String(url) => *url = redact_url_credentials(url),
        _ => {}
    }
}

/// Keeps only the URL's scheme, host, and port, or fully redacts URLs without a host.
fn redact_url_credentials(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else { return "[REDACTED_URL]".to_string() };
    // A scheme-less credential string can parse with its username as the scheme.
    if parsed.host().is_none() {
        return "[REDACTED_URL]".to_string();
    }
    // Credentials can occur in userinfo, paths, queries, or fragments regardless of length.
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_path("");
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
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

            let mut params: serde_json::Value = params.into();

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
                        redact_fork_credentials(&method, &mut params);
                        self.on_request_error(&method, &id, &params, error);
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
                        error!(target: "rpc", ?id, ?method, "failed to deserialize method due to unknown variant");
                        RpcResponse::new(id, RpcError::method_not_found())
                    } else {
                        error!(target: "rpc", ?id, ?method, ?err, "failed to deserialize method");
                        RpcResponse::new(id, RpcError::invalid_params(err))
                    }
                }
            }
        }
    }

    /// Invoked when a deserialized request returns an error from [`Self::on_request`].
    /// Parameters retain their original shape with fork URL credentials redacted.
    /// Deserialization errors are excluded because their messages can contain request parameters.
    fn on_request_error(
        &self,
        _method: &str,
        _id: &Id,
        _params: &serde_json::Value,
        _error: &RpcError,
    ) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_url_credentials() {
        for (url, expected) in [
            ("https://user:pass@example.invalid:8545/", "https://example.invalid:8545/"),
            ("https://example.invalid:8545/key?token=x#secret", "https://example.invalid:8545/"),
            (
                "https://example.invalid:8545/abcdefghijklmnop?api_key=abcdefghijklmnop",
                "https://example.invalid:8545/",
            ),
            ("invalid secret URL", "[REDACTED_URL]"),
            ("https://user:pass@invalid host/secret", "[REDACTED_URL]"),
            ("mykey:secret@example.invalid/rpc/key", "[REDACTED_URL]"),
            ("mykey:/secret@example.invalid/rpc/key", "[REDACTED_URL]"),
        ] {
            assert_eq!(redact_url_credentials(url), expected, "{url}");
        }
    }

    #[test]
    fn redacts_fork_parameter_shapes_without_losing_context() {
        let url = "https://user:pass@example.invalid:8545/key?token=x#secret";
        let safe = "https://example.invalid:8545/";
        for (params, expected) in [
            (json!([{"forking": {"jsonRpcUrl": url}}]), json!([{"forking": {"jsonRpcUrl": safe}}])),
            (json!({"forking": {"jsonRpcUrl": url}}), json!({"forking": {"jsonRpcUrl": safe}})),
            (json!({"jsonRpcUrl": url}), json!({"jsonRpcUrl": safe})),
            (
                json!({"params": {"forking": {"jsonRpcUrl": url, "blockNumber": 42}}}),
                json!({"params": {"forking": {"jsonRpcUrl": safe, "blockNumber": 42}}}),
            ),
            (json!([[url, "0x2a"]]), json!([[safe, "0x2a"]])),
            (json!([{"forking": [url, 42]}]), json!([{"forking": [safe, 42]}])),
            (json!({"params": [url, 42]}), json!({"params": [safe, 42]})),
            (json!([[[url, 42]]]), json!([[[safe, 42]]])),
            (json!({"params": [[url, 42]]}), json!({"params": [[safe, 42]]})),
            (json!([[url]]), json!([[safe]])),
            (json!([[null, "0x2a"]]), json!([[null, "0x2a"]])),
            (
                json!({"params": {"blockNumber": "0x2a", "extra": ["diagnostic"]}}),
                json!({"params": {"blockNumber": "0x2a", "extra": ["diagnostic"]}}),
            ),
            (
                json!({"params": {"forking": [null, 42], "extra": ["diagnostic"]}}),
                json!({"params": {"forking": [null, 42], "extra": ["diagnostic"]}}),
            ),
        ] {
            for method in ["anvil_reset", "hardhat_reset"] {
                assert_eq!(redact_params(method, &params), expected, "{method}: {params}");
                let mut failed = params.clone();
                redact_fork_credentials(method, &mut failed);
                assert_eq!(failed, expected, "{method}: {params}");
            }
        }
        for (mut params, expected) in [
            (json!([url]), json!([safe])),
            (json!(["invalid secret URL"]), json!(["[REDACTED_URL]"])),
        ] {
            assert_eq!(redact_params("anvil_setRpcUrl", &params), expected);
            redact_fork_credentials("anvil_setRpcUrl", &mut params);
            assert_eq!(params, expected);
        }
    }

    #[test]
    fn preserves_failure_params_and_redacts_only_trace_transactions() {
        for method in [
            "eth_sendRawTransaction",
            "eth_sendRawTransactionSync",
            "eth_sendRawTransactionConditional",
            "eth_signRawTransaction",
            "anvil_classifyTransaction",
            "trace_rawTransaction",
        ] {
            for (params, expected) in [
                (json!(["0x12345678"]), json!(["[SIGNED_TX]"])),
                (json!(["0x12345678", 5000]), json!(["[SIGNED_TX]", 5000])),
                (
                    json!(["0x12345678", ["trace", "stateDiff"], "0x2a"]),
                    json!(["[SIGNED_TX]", ["trace", "stateDiff"], "0x2a"]),
                ),
                (json!([]), json!([])),
                (json!(null), json!(null)),
            ] {
                assert_eq!(redact_params(method, &params), expected, "{method}");
                let mut failed = params.clone();
                redact_fork_credentials(method, &mut failed);
                assert_eq!(failed, params, "{method}");
            }
        }
        let params = json!([{"to": "0x1111111111111111111111111111111111111111", "data": "0x12345678"}, "0x100000"]);
        assert_eq!(redact_params("eth_call", &params), params);
        let mut failed = params.clone();
        redact_fork_credentials("eth_call", &mut failed);
        assert_eq!(failed, params);
    }
}
