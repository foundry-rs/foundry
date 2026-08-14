use crate::RpcHandler;
use anvil_rpc::{
    error::RpcError,
    request::{Id, Request, RpcCall, RpcMethodCall, Version},
    response::{Response, RpcResponse},
};
use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response as AxumResponse},
};
use futures::{FutureExt, future};

/// Handles incoming JSON-RPC Request.
// NOTE: `handler` must come first because the `request` extractor consumes the request body.
pub async fn handle<Http: RpcHandler, Ws>(
    State((handler, _)): State<(Http, Ws)>,
    request: Result<Json<Request>, JsonRejection>,
) -> AxumResponse {
    match request {
        Ok(Json(req)) => handle_request(req, handler)
            .await
            .map(Json)
            .map(IntoResponse::into_response)
            .unwrap_or_else(|| StatusCode::NO_CONTENT.into_response()),
        Err(err) => {
            warn!(target: "rpc", ?err, "invalid request");
            Json(Response::error(RpcError::invalid_request())).into_response()
        }
    }
}

/// Handle the JSON-RPC [Request]
///
/// This will try to deserialize the payload into the request type of the handler and if successful
/// invoke the handler
pub async fn handle_request<Handler: RpcHandler>(
    req: Request,
    handler: Handler,
) -> Option<Response> {
    /// processes batch calls
    fn responses_as_batch(outs: Vec<Option<RpcResponse>>) -> Option<Response> {
        let batch: Vec<_> = outs.into_iter().flatten().collect();
        (!batch.is_empty()).then_some(Response::Batch(batch))
    }

    match req {
        Request::Single(call) => handle_call(call, handler).await.map(Response::Single),
        Request::Batch(calls) => {
            if calls.is_empty() {
                return Some(Response::error(RpcError::invalid_request()));
            }
            future::join_all(calls.into_iter().map(move |call| handle_call(call, handler.clone())))
                .map(responses_as_batch)
                .await
        }
    }
}

/// handle a single RPC method call
async fn handle_call<Handler: RpcHandler>(call: RpcCall, handler: Handler) -> Option<RpcResponse> {
    match call {
        RpcCall::MethodCall(call) => {
            trace!(target: "rpc", id = ?call.id , method = ?call.method,  "handling call");
            Some(handler.on_call(call).await)
        }
        RpcCall::Notification(notification) => {
            let call = RpcMethodCall {
                jsonrpc: notification.jsonrpc.unwrap_or(Version::V2),
                method: notification.method,
                params: notification.params,
                id: Id::Null,
            };
            trace!(target: "rpc", method = ?call.method, "handling rpc notification");
            drop(handler.on_call(call).await);
            None
        }
        RpcCall::Invalid { id } => {
            warn!(target: "rpc", ?id,  "invalid rpc call");
            Some(RpcResponse::invalid_request(id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_rpc::{
        request::{RequestParams, RpcNotification},
        response::ResponseResult,
    };
    use axum::body::to_bytes;
    use std::{
        pin::pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll, Waker},
    };

    #[derive(Clone, Default)]
    struct TestHandler {
        requests: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl RpcHandler for TestHandler {
        type Request = serde_json::Value;

        async fn on_request(&self, _request: Self::Request) -> ResponseResult {
            self.requests.fetch_add(1, Ordering::Relaxed);
            ResponseResult::success(())
        }
    }

    fn notification() -> RpcCall {
        RpcCall::Notification(RpcNotification {
            jsonrpc: Some(Version::V2),
            method: "record".to_owned(),
            params: RequestParams::None,
        })
    }

    fn method_call(id: i64) -> RpcCall {
        RpcCall::MethodCall(RpcMethodCall {
            jsonrpc: Version::V2,
            method: "record".to_owned(),
            params: RequestParams::None,
            id: Id::Number(id),
        })
    }

    fn run_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut future = pin!(future);
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("future unexpectedly pending"),
        }
    }

    #[test]
    fn empty_batch_returns_invalid_request() {
        let response = run_ready(handle_request(Request::Batch(vec![]), TestHandler::default()));

        assert_eq!(response, Some(Response::error(RpcError::invalid_request())));
    }

    #[test]
    fn notification_only_batch_executes_without_response() {
        let handler = TestHandler::default();
        let response = run_ready(handle_request(
            Request::Batch(vec![notification(), notification()]),
            handler.clone(),
        ));

        assert_eq!(response, None);
        assert_eq!(handler.requests.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn mixed_batch_executes_notification_without_including_response() {
        let handler = TestHandler::default();
        let response = run_ready(handle_request(
            Request::Batch(vec![notification(), method_call(1)]),
            handler.clone(),
        ));

        assert_eq!(
            response,
            Some(Response::Batch(vec![RpcResponse::new(
                Id::Number(1),
                ResponseResult::success(())
            )]))
        );
        assert_eq!(handler.requests.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn http_notification_returns_no_content_after_execution() {
        let handler = TestHandler::default();
        let response = run_ready(handle(
            State((handler.clone(), ())),
            Ok(Json(Request::Single(notification()))),
        ));

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(run_ready(to_bytes(response.into_body(), usize::MAX)).unwrap().is_empty());
        assert_eq!(handler.requests.load(Ordering::Relaxed), 1);
    }
}
