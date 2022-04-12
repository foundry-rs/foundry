use crate::eth::EthApi;
use anvil_core::{
    error::RpcError,
    eth::EthRequest,
    request::{Request, RpcCall, RpcMethodCall},
    response::{Response, RpcResponse},
};
use axum::{
    extract::{rejection::JsonRejection, Extension},
    Json,
};
use futures::{future, FutureExt};
use tracing::{trace, warn};

/// Handles incoming JSON-RPC Request
pub async fn handle_rpc(
    request: Result<Json<Request>, JsonRejection>,
    Extension(api): Extension<EthApi>,
) -> Json<Response> {
    match request {
        Err(err) => {
            warn!("invalid request={:?}", err);
            Response::error(RpcError::invalid_request()).into()
        }
        Ok(req) => handle_request(req.0, api)
            .await
            .unwrap_or_else(|| Response::error(RpcError::invalid_request()))
            .into(),
    }
}

/// handle the JSON-RPC [Request]
pub async fn handle_request(req: Request, api: EthApi) -> Option<Response> {
    /// processes batch calls
    fn responses_as_batch(outs: Vec<Option<RpcResponse>>) -> Option<Response> {
        let batch: Vec<_> = outs.into_iter().flatten().collect();
        (!batch.is_empty()).then(|| Response::Batch(batch))
    }

    match req {
        Request::Single(call) => handle_call(call, api).await.map(Response::Single),
        Request::Batch(calls) => {
            future::join_all(calls.into_iter().map(move |call| handle_call(call, api.clone())))
                .map(responses_as_batch)
                .await
        }
    }
}

/// handle a single RPC method call
async fn handle_call(call: RpcCall, api: EthApi) -> Option<RpcResponse> {
    match call {
        RpcCall::MethodCall(call) => Some(execute_method_call(call, api).await),
        RpcCall::Notification(notification) => {
            trace!("received rpc notification method={}", notification.method);
            None
        }
        RpcCall::Invalid { id } => {
            trace!("invalid rpc call id={}", id);
            Some(RpcResponse::invalid_request(id))
        }
    }
}

/// Executes a valid RPC method call
async fn execute_method_call(call: RpcMethodCall, api: EthApi) -> RpcResponse {
    trace!(target: "rpc", "received method call {:?}", call);
    let RpcMethodCall { method, params, id, .. } = call;

    let params: serde_json::Value = params.into();
    let m = method.clone();
    let call = serde_json::json!({
        "method": method,
        "params": params
    });

    match serde_json::from_value::<EthRequest>(call) {
        Err(err) => {
            let msg = err.to_string();
            warn!(target: "rpc", "failed to deserialize method `{}`: {}", m, msg);
            if msg.contains("unknown variant") {
                RpcResponse::new(id, RpcError::method_not_found())
            } else {
                RpcResponse::new(id, RpcError::invalid_params(msg))
            }
        }
        Ok(req) => {
            let result = api.execute(req).await;
            trace!(target: "rpc", "sending rpc result {:?}", result);
            RpcResponse::new(id, result)
        }
    }
}
