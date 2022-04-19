//! Bootstrap [axum] servers

use crate::{
    eth::EthApi,
    server::{eth::HttpEthRpcHandler, ws::WsRpcHandler},
};

use anvil_rpc::{
    error::RpcError,
    request::RpcMethodCall,
    response::{ResponseResult, RpcResponse},
};
use axum::{extract::Extension, routing::post, Router, Server};
use eth::WsEthRpcHandler;
use serde::de::DeserializeOwned;
use std::{future::Future, net::SocketAddr};
use tower_http::trace::TraceLayer;
use tracing::{trace, warn};

mod eth;
/// handlers for axum server
mod handler;
mod ws;

/// Configures an [axum::Server] that handles [EthApi] related JSON-RPC calls via HTTP and WS
pub fn serve(addr: SocketAddr, api: EthApi) -> impl Future<Output = hyper::Result<()>> {
    let http = HttpEthRpcHandler::new(api.clone());
    let ws = WsEthRpcHandler::new(api);
    serve_http_ws(addr, http, ws)
}

/// Configures an [axum::Server] that handles RPC-Calls, both HTTP requests and requests via
/// websocket
pub fn serve_http_ws<Http, Ws>(
    addr: SocketAddr,
    http: Http,
    ws: Ws,
) -> impl Future<Output = hyper::Result<()>>
where
    Http: RpcHandler,
    Ws: WsRpcHandler,
{
    let svc = Router::new()
        .route("/", post(handler::handle::<Http>).get(ws::handle_ws::<Ws>))
        .layer(Extension(http))
        .layer(Extension(ws))
        .layer(TraceLayer::new_for_http())
        .into_make_service();
    Server::bind(&addr).serve(svc)
}

/// Helper trait that is used to execute ethereum rpc calls
#[async_trait::async_trait]
pub trait RpcHandler: Clone + Send + Sync + 'static {
    /// The request type to expect
    type Request: DeserializeOwned + Send + Sync;

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
                let result = self.on_request(req).await;
                trace!(target: "rpc", "sending rpc result {:?}", result);
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
