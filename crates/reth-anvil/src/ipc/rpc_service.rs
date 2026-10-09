//! IPC method dispatch from reth-ipc, with connection-scoped subscriptions.
use futures::{
    future::Either,
    stream::{FuturesOrdered, StreamExt},
};
use jsonrpsee::{
    BatchResponse, BatchResponseBuilder, BoundedSubscriptions, ConnectionId, MethodCallback,
    MethodResponse, MethodSink, Methods, SubscriptionState,
    core::middleware::{Batch, BatchEntry},
    server::{
        IdProvider,
        middleware::rpc::{ResponseFuture, RpcServiceT},
    },
    types::{ErrorCode, ErrorObject, Id, Request, error::reject_too_many_subscriptions},
};
use std::sync::Arc;

/// JSON-RPC service middleware.
#[derive(Clone, Debug)]
pub(super) struct RpcService {
    conn_id: ConnectionId,
    methods: Methods,
    max_response_body_size: usize,
    cfg: RpcServiceCfg,
}

/// Configuration of the `RpcService`.
#[derive(Clone, Debug)]
pub(super) struct RpcServiceCfg {
    pub(super) bounded_subscriptions: BoundedSubscriptions,
    pub(super) sink: MethodSink,
    pub(super) id_provider: Arc<dyn IdProvider>,
}

impl RpcService {
    /// Create a new service.
    pub(super) const fn new(
        methods: Methods,
        max_response_body_size: usize,
        conn_id: ConnectionId,
        cfg: RpcServiceCfg,
    ) -> Self {
        Self { methods, max_response_body_size, conn_id, cfg }
    }
}

impl RpcServiceT for RpcService {
    type MethodResponse = MethodResponse;
    type NotificationResponse = Option<MethodResponse>;
    type BatchResponse = BatchResponse;

    fn call<'a>(&self, req: Request<'a>) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        let conn_id = self.conn_id;
        let max_response_body_size = self.max_response_body_size;

        let params = req.params();
        let name = req.method_name();
        let id = req.id().clone();
        let extensions = req.extensions.clone();

        match self.methods.method_with_name(name) {
            None => {
                let rp = MethodResponse::error(id, ErrorObject::from(ErrorCode::MethodNotFound));
                ResponseFuture::ready(rp)
            }
            Some((_name, method)) => match method {
                MethodCallback::Sync(callback) => {
                    let rp = (callback)(id, params, max_response_body_size, extensions);
                    ResponseFuture::ready(rp)
                }
                MethodCallback::Async(callback) => {
                    let params = params.into_owned();
                    let id = id.into_owned();

                    let fut = (callback)(id, params, conn_id, max_response_body_size, extensions);
                    ResponseFuture::future(fut)
                }
                MethodCallback::Subscription(callback) => {
                    let cfg = &self.cfg;

                    if let Some(p) = cfg.bounded_subscriptions.acquire() {
                        let conn_state = SubscriptionState {
                            conn_id,
                            id_provider: &*cfg.id_provider,
                            subscription_permit: p,
                        };

                        let fut =
                            callback(id.clone(), params, cfg.sink.clone(), conn_state, extensions);
                        ResponseFuture::future(fut)
                    } else {
                        let max = cfg.bounded_subscriptions.max();
                        let rp = MethodResponse::error(id, reject_too_many_subscriptions(max));
                        ResponseFuture::ready(rp)
                    }
                }
                MethodCallback::Unsubscription(callback) => {
                    // Don't adhere to any resource or subscription limits; always let unsubscribing
                    // happen!

                    let rp = callback(id, params, conn_id, max_response_body_size, extensions);
                    ResponseFuture::ready(rp)
                }
            },
        }
    }

    fn batch<'a>(&self, req: Batch<'a>) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        let entries: Vec<_> = req.into_iter().collect();
        let mut batch_response = BatchResponseBuilder::new_with_limit(self.max_response_body_size);

        let mut pending_calls: FuturesOrdered<_> = entries
            .into_iter()
            .filter_map(|v| match v {
                Ok(BatchEntry::Call(call)) => Some(Either::Right(self.call(call))),
                Ok(BatchEntry::Notification(_n)) => None,
                Err(_err) => Some(Either::Left(async {
                    MethodResponse::error(Id::Null, ErrorObject::from(ErrorCode::InvalidRequest))
                })),
            })
            .collect();
        async move {
            while let Some(response) = pending_calls.next().await {
                if let Err(too_large) = batch_response.append(response) {
                    let mut error_batch = BatchResponseBuilder::new_with_limit(1);
                    let _ = error_batch.append(too_large);
                    return error_batch.finish();
                }
            }

            batch_response.finish()
        }
    }

    #[expect(clippy::manual_async_fn)]
    fn notification<'a>(
        &self,
        _n: jsonrpsee::core::middleware::Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        async move { None }
    }
}
