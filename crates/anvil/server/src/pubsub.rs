use crate::{error::RequestError, handler::handle_request, RpcHandler};
use anvil_rpc::{
    error::RpcError,
    request::Request,
    response::{Response, ResponseResult},
};

use futures::{FutureExt, Sink, SinkExt, Stream, StreamExt};
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    hash::Hash,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// The general purpose trait for handling RPC requests and subscriptions
#[async_trait::async_trait]
pub trait PubSubRpcHandler: Clone + Send + Sync + Unpin + 'static {
    /// The request type to expect
    type Request: DeserializeOwned + Send + Sync + fmt::Debug;
    /// The identifier to use for subscriptions
    type SubscriptionId: Hash + PartialEq + Eq + Send + Sync + fmt::Debug;
    /// The subscription type this handle may create
    type Subscription: Stream<Item = serde_json::Value> + Send + Sync + Unpin;

    /// Invoked when the request was received
    async fn on_request(&self, request: Self::Request, cx: PubSubContext<Self>) -> ResponseResult;
}

type Subscriptions<SubscriptionId, Subscription> = Arc<Mutex<Vec<(SubscriptionId, Subscription)>>>;

/// Contains additional context and tracks subscriptions
pub struct PubSubContext<Handler: PubSubRpcHandler> {
    /// all active subscriptions `id -> Stream`
    subscriptions: Subscriptions<Handler::SubscriptionId, Handler::Subscription>,
}

impl<Handler: PubSubRpcHandler> PubSubContext<Handler> {
    /// Adds new active subscription
    ///
    /// Returns the previous subscription, if any
    pub fn add_subscription(
        &self,
        id: Handler::SubscriptionId,
        subscription: Handler::Subscription,
    ) -> Option<Handler::Subscription> {
        let mut subscriptions = self.subscriptions.lock();
        let mut removed = None;
        if let Some(idx) = subscriptions.iter().position(|(i, _)| id == *i) {
            trace!(target: "rpc", ?id,  "removed subscription");
            removed = Some(subscriptions.swap_remove(idx).1);
        }
        trace!(target: "rpc", ?id,  "added subscription");
        subscriptions.push((id, subscription));
        removed
    }

    /// Removes an existing subscription
    pub fn remove_subscription(
        &self,
        id: &Handler::SubscriptionId,
    ) -> Option<Handler::Subscription> {
        let mut subscriptions = self.subscriptions.lock();
        if let Some(idx) = subscriptions.iter().position(|(i, _)| id == i) {
            trace!(target: "rpc", ?id,  "removed subscription");
            return Some(subscriptions.swap_remove(idx).1)
        }
        None
    }
}

impl<Handler: PubSubRpcHandler> Clone for PubSubContext<Handler> {
    fn clone(&self) -> Self {
        Self { subscriptions: Arc::clone(&self.subscriptions) }
    }
}

impl<Handler: PubSubRpcHandler> Default for PubSubContext<Handler> {
    fn default() -> Self {
        Self { subscriptions: Arc::new(Mutex::new(Vec::new())) }
    }
}

/// A compatibility helper type to use common `RpcHandler` functions
struct ContextAwareHandler<Handler: PubSubRpcHandler> {
    handler: Handler,
    context: PubSubContext<Handler>,
}

impl<Handler: PubSubRpcHandler> Clone for ContextAwareHandler<Handler> {
    fn clone(&self) -> Self {
        Self { handler: self.handler.clone(), context: self.context.clone() }
    }
}

#[async_trait::async_trait]
impl<Handler: PubSubRpcHandler> RpcHandler for ContextAwareHandler<Handler> {
    type Request = Handler::Request;

    async fn on_request(&self, request: Self::Request) -> ResponseResult {
        self.handler.on_request(request, self.context.clone()).await
    }
}

/// Represents a connection to a client via websocket
///
/// Contains the state for the entire connection
pub struct PubSubConnection<Handler: PubSubRpcHandler, Connection> {
    /// the handler for the websocket connection
    handler: Handler,
    /// contains all the subscription related context
    context: PubSubContext<Handler>,
    /// The established connection
    connection: Connection,
    /// currently in progress requests
    processing: Vec<Pin<Box<dyn Future<Output = Response> + Send>>>,
    /// pending messages to send
    pending: VecDeque<String>,
}

impl<Handler: PubSubRpcHandler, Connection> PubSubConnection<Handler, Connection> {
    pub fn new(connection: Connection, handler: Handler) -> Self {
        Self {
            connection,
            handler,
            context: Default::default(),
            pending: Default::default(),
            processing: Default::default(),
        }
    }

    /// Returns a compatibility `RpcHandler`
    fn compat_helper(&self) -> ContextAwareHandler<Handler> {
        ContextAwareHandler { handler: self.handler.clone(), context: self.context.clone() }
    }

    fn process_request(&mut self, req: serde_json::Result<Request>) {
        let handler = self.compat_helper();
        self.processing.push(Box::pin(async move {
            match req {
                Ok(req) => handle_request(req, handler)
                    .await
                    .unwrap_or_else(|| Response::error(RpcError::invalid_request())),
                Err(err) => {
                    error!(target: "rpc", ?err, "invalid request");
                    Response::error(RpcError::invalid_request())
                }
            }
        }));
    }
}

impl<Handler, Connection> Future for PubSubConnection<Handler, Connection>
where
    Handler: PubSubRpcHandler,
    Connection: Sink<String> + Stream<Item = Result<Option<Request>, RequestError>> + Unpin,
    <Connection as Sink<String>>::Error: fmt::Debug,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();
        loop {
            // drive the websocket
            while matches!(pin.connection.poll_ready_unpin(cx), Poll::Ready(Ok(()))) {
                // only start sending if socket is ready
                if let Some(msg) = pin.pending.pop_front() {
                    if let Err(err) = pin.connection.start_send_unpin(msg) {
                        error!(target: "rpc", ?err, "Failed to send message");
                    }
                } else {
                    break
                }
            }

            // Ensure any pending messages are flushed
            // this needs to be called manually for tungsenite websocket: <https://github.com/foundry-rs/foundry/issues/6345>
            if let Poll::Ready(Err(err)) = pin.connection.poll_flush_unpin(cx) {
                trace!(target: "rpc", ?err, "websocket err");
                // close the connection
                return Poll::Ready(())
            }

            loop {
                match pin.connection.poll_next_unpin(cx) {
                    Poll::Ready(Some(req)) => match req {
                        Ok(Some(req)) => {
                            pin.process_request(Ok(req));
                        }
                        Err(err) => match err {
                            RequestError::Axum(err) => {
                                trace!(target: "rpc", ?err, "client disconnected");
                                return Poll::Ready(())
                            }
                            RequestError::Io(err) => {
                                trace!(target: "rpc", ?err, "client disconnected");
                                return Poll::Ready(())
                            }
                            RequestError::Serde(err) => {
                                pin.process_request(Err(err));
                            }
                            RequestError::Disconnect => {
                                trace!(target: "rpc", "client disconnected");
                                return Poll::Ready(())
                            }
                        },
                        _ => {}
                    },
                    Poll::Ready(None) => {
                        trace!(target: "rpc", "socket connection finished");
                        return Poll::Ready(())
                    }
                    Poll::Pending => break,
                }
            }

            let mut progress = false;
            for n in (0..pin.processing.len()).rev() {
                let mut req = pin.processing.swap_remove(n);
                match req.poll_unpin(cx) {
                    Poll::Ready(resp) => {
                        if let Ok(text) = serde_json::to_string(&resp) {
                            pin.pending.push_back(text);
                            progress = true;
                        }
                    }
                    Poll::Pending => pin.processing.push(req),
                }
            }

            {
                // process subscription events
                let mut subscriptions = pin.context.subscriptions.lock();
                'outer: for n in (0..subscriptions.len()).rev() {
                    let (id, mut sub) = subscriptions.swap_remove(n);
                    'inner: loop {
                        match sub.poll_next_unpin(cx) {
                            Poll::Ready(Some(res)) => {
                                if let Ok(text) = serde_json::to_string(&res) {
                                    pin.pending.push_back(text);
                                    progress = true;
                                }
                            }
                            Poll::Ready(None) => continue 'outer,
                            Poll::Pending => break 'inner,
                        }
                    }

                    subscriptions.push((id, sub));
                }
            }

            if !progress {
                return Poll::Pending
            }
        }
    }
}
