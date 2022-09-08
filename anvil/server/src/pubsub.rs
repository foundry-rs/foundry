use crate::{handler::handle_request, RpcHandler};
use anvil_rpc::{
    error::RpcError,
    request::Request,
    response::{Response, ResponseResult},
};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        WebSocketUpgrade,
    },
    response::IntoResponse,
    Extension,
};
use futures::{FutureExt, SinkExt, Stream, StreamExt};
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
use tracing::{error, trace};

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

// === impl WsContext ===

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
            trace!(target: "rpc::ws", ?id,  "removed subscription");
            removed = Some(subscriptions.swap_remove(idx).1);
        }
        trace!(target: "rpc::ws", ?id,  "added subscription");
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
            trace!(target: "rpc::ws", ?id,  "removed subscription");
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
pub struct PubSubConnection<Handler: PubSubRpcHandler> {
    /// the handler for the websocket connection
    handler: Handler,
    /// contains all the subscription related context
    context: PubSubContext<Handler>,
    /// The established connection
    connection: WebSocket,
    /// currently in progress requests
    processing: Vec<Pin<Box<dyn Future<Output = Response> + Send>>>,
    /// pending messages to send
    pending: VecDeque<Message>,
}

// === impl WsConnection ===

impl<Handler: PubSubRpcHandler> PubSubConnection<Handler> {
    pub fn new(socket: WebSocket, handler: Handler) -> Self {
        Self {
            connection: socket,
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
                    error!(target: "rpc::ws", ?err, "invalid request");
                    Response::error(RpcError::invalid_request())
                }
            }
        }));
    }

    fn on_message(&mut self, msg: Message) -> bool {
        match msg {
            Message::Text(text) => {
                self.process_request(serde_json::from_str(&text));
            }
            Message::Binary(data) => {
                // the binary payload type is the request as-is but as bytes, if this is a valid
                // `Request` then we can deserialize the Json from the data Vec
                self.process_request(serde_json::from_slice(&data));
            }
            Message::Close(_) => {
                trace!(target: "rpc::ws", "ws client disconnected");
                return true
            }
            Message::Ping(ping) => {
                self.pending.push_back(Message::Pong(ping));
            }
            _ => {}
        }
        false
    }
}

impl<Handler: PubSubRpcHandler> Future for PubSubConnection<Handler> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();
        loop {
            // drive the sink
            while let Poll::Ready(Ok(())) = pin.connection.poll_ready_unpin(cx) {
                // only start sending if socket is ready
                if let Some(msg) = pin.pending.pop_front() {
                    if let Err(err) = pin.connection.start_send_unpin(msg) {
                        error!(target: "rpc::ws", ?err, "Failed to send message");
                    }
                } else {
                    break
                }
            }

            loop {
                match pin.connection.poll_next_unpin(cx) {
                    Poll::Ready(Some(msg)) => {
                        if let Ok(msg) = msg {
                            if pin.on_message(msg) {
                                return Poll::Ready(())
                            }
                        } else {
                            trace!(target: "rpc::ws", "client disconnected");
                            return Poll::Ready(())
                        }
                    }
                    Poll::Ready(None) => {
                        trace!(target: "rpc::ws", "socket connection finished");
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
                            pin.pending.push_back(Message::Text(text));
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
                                    pin.pending.push_back(Message::Text(text));
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
