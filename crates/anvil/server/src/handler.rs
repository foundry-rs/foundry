use crate::RpcHandler;
use anvil_rpc::{
    error::RpcError,
    request::{Request, RpcCall},
    response::{Response, RpcResponse},
};
use axum::{
    Json,
    extract::{ConnectInfo, State, rejection::JsonRejection},
};
use futures::{FutureExt, future};
use std::net::SocketAddr;

/// Handles incoming JSON-RPC Request.
// NOTE: `handler` must come first because the `request` extractor consumes the request body.
pub async fn handle<Http: RpcHandler, Ws>(
    State((handler, _)): State<(Http, Ws)>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    request: Result<Json<Request>, JsonRejection>,
) -> Json<Response> {
    Json(match request {
        Ok(Json(req)) => handle_request(req, handler, Some(peer_addr))
            .await
            .unwrap_or_else(|| Response::error(RpcError::invalid_request())),
        Err(err) => {
            warn!(target: "rpc", ?err, "invalid request");
            Response::error(RpcError::invalid_request())
        }
    })
}

/// Handle the JSON-RPC [Request]
///
/// This will try to deserialize the payload into the request type of the handler and if successful
/// invoke the handler
pub async fn handle_request<Handler: RpcHandler>(
    req: Request,
    handler: Handler,
    peer_addr: Option<SocketAddr>,
) -> Option<Response> {
    /// processes batch calls
    fn responses_as_batch(outs: Vec<Option<RpcResponse>>) -> Option<Response> {
        let batch: Vec<_> = outs.into_iter().flatten().collect();
        (!batch.is_empty()).then_some(Response::Batch(batch))
    }

    match req {
        Request::Single(call) => handle_call(call, handler, peer_addr).await.map(Response::Single),
        Request::Batch(calls) => {
            future::join_all(
                calls.into_iter().map(move |call| handle_call(call, handler.clone(), peer_addr)),
            )
            .map(responses_as_batch)
            .await
        }
    }
}

/// handle a single RPC method call
async fn handle_call<Handler: RpcHandler>(
    call: RpcCall,
    handler: Handler,
    peer_addr: Option<SocketAddr>,
) -> Option<RpcResponse> {
    match call {
        RpcCall::MethodCall(call) => {
            trace!(
                target: "rpc",
                id = ?call.id ,
                method = ?call.method,
                ?peer_addr,
                "handling call"
            );
            Some(handler.on_call_with_addr(call, peer_addr).await)
        }
        RpcCall::Notification(notification) => {
            trace!(target: "rpc", method = ?notification.method, "received rpc notification");
            None
        }
        RpcCall::Invalid { id } => {
            warn!(target: "rpc", ?id,  "invalid rpc call");
            Some(RpcResponse::invalid_request(id))
        }
    }
}
