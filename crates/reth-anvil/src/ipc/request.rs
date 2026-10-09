//! IPC request handling from reth-ipc, retaining native batch and subscription responses.

use futures::{FutureExt, StreamExt, stream::FuturesOrdered};
use jsonrpsee::{
    BatchResponseBuilder, MethodResponse, batch_response_error,
    core::{JsonRawValue, server::helpers::prepare_error},
    server::middleware::rpc::RpcServiceT,
    types::{
        ErrorObject, Id, InvalidRequest, Notification, Request,
        error::{ErrorCode, reject_too_big_request},
    },
};
use std::panic::AssertUnwindSafe;
use tokio_util::either::Either;
use tracing::instrument;

type Notif<'a> = Notification<'a, Option<&'a JsonRawValue>>;

// Return all non-notification responses in one batch message.
#[instrument(name = "batch", skip(data, rpc_service))]
async fn process_batch_request<S>(
    data: Vec<u8>,
    rpc_service: S,
    max_response_body_size: usize,
) -> Option<Box<JsonRawValue>>
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send,
{
    if let Ok(batch) = serde_json::from_slice::<Vec<&JsonRawValue>>(&data) {
        let mut got_notif = false;
        let mut batch_response = BatchResponseBuilder::new_with_limit(max_response_body_size);

        let mut pending_calls: FuturesOrdered<_> = batch
            .into_iter()
            .filter_map(|v| {
                if let Ok(req) = serde_json::from_str::<Request<'_>>(v.get()) {
                    Some(Either::Right(catch_call_panic(req.id(), rpc_service.call(req))))
                } else if let Ok(_notif) = serde_json::from_str::<Notif<'_>>(v.get()) {
                    // notifications should not be answered.
                    got_notif = true;
                    None
                } else {
                    // Valid JSON might not parse as `InvalidRequest`.
                    let id = match serde_json::from_str::<InvalidRequest<'_>>(v.get()) {
                        Ok(err) => err.id,
                        Err(_) => Id::Null,
                    };

                    Some(Either::Left(async {
                        MethodResponse::error(id, ErrorObject::from(ErrorCode::InvalidRequest))
                    }))
                }
            })
            .collect();

        while let Some(response) = pending_calls.next().await {
            if let Err(too_large) = batch_response.append(response) {
                return Some(too_large.into_json());
            }
        }

        if got_notif && batch_response.is_empty() {
            None
        } else {
            let batch_resp = batch_response.finish();
            Some(MethodResponse::from_batch(batch_resp).into_json())
        }
    } else {
        Some(batch_response_error(Id::Null, ErrorObject::from(ErrorCode::ParseError)))
    }
}

async fn process_single_request<S>(data: Vec<u8>, rpc_service: &S) -> Option<MethodResponse>
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send,
{
    if let Ok(req) = serde_json::from_slice::<Request<'_>>(&data) {
        Some(execute_call_with_tracing(req, rpc_service).await)
    } else if serde_json::from_slice::<Notif<'_>>(&data).is_ok() {
        None
    } else {
        let (id, code) = prepare_error(&data);
        Some(MethodResponse::error(id, ErrorObject::from(code)))
    }
}

#[instrument(name = "method_call", fields(method = req.method.as_ref()), skip(req, rpc_service))]
async fn execute_call_with_tracing<'a, S>(req: Request<'a>, rpc_service: &S) -> MethodResponse
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send,
{
    catch_call_panic(req.id(), rpc_service.call(req)).await
}

/// Answers a panicking call with an internal error, otherwise the panic would only surface as a
/// failed call task and the client would never receive a response for `id`.
async fn catch_call_panic(
    id: Id<'_>,
    call: impl Future<Output = MethodResponse>,
) -> MethodResponse {
    AssertUnwindSafe(call)
        .catch_unwind()
        .await
        .unwrap_or_else(|_| MethodResponse::error(id, ErrorObject::from(ErrorCode::InternalError)))
}

pub(super) async fn call_with_service<S>(
    request: String,
    rpc_service: S,
    max_response_body_size: usize,
    max_request_body_size: usize,
) -> Option<Box<JsonRawValue>>
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send,
{
    enum Kind {
        Single,
        Batch,
    }

    let request_kind = request
        .chars()
        .find_map(|c| match c {
            '{' => Some(Kind::Single),
            '[' => Some(Kind::Batch),
            _ => None,
        })
        .unwrap_or(Kind::Single);

    let data = request.into_bytes();
    if data.len() > max_request_body_size {
        return Some(batch_response_error(
            Id::Null,
            reject_too_big_request(max_request_body_size as u32),
        ));
    }

    // Single request or notification.
    if matches!(request_kind, Kind::Single) {
        let response = process_single_request(data, &rpc_service).await;
        match response {
            Some(response) if response.is_method_call() => Some(response.into_json()),
            _ => {
                // Subscription responses go directly to the sink; do not send them twice.
                None
            }
        }
    } else {
        process_batch_request(data, rpc_service, max_response_body_size).await
    }
}
