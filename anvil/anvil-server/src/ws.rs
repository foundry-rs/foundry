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
use futures::{Stream, StreamExt};
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use std::{
    collections::{HashMap, VecDeque},
    fmt,
    hash::Hash,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tracing::{trace, warn};

/// Handles incoming Websocket upgrade
///
/// This is the entrypoint invoked by the axum server for a websocket request
pub async fn handle_ws<Handler: WsRpcHandler>(
    ws: WebSocketUpgrade,
    Extension(handler): Extension<Handler>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_ws_socket(socket, handler))
}

/// Entrypoint once a new `WebSocket` was established
async fn handle_ws_socket<Handler: WsRpcHandler>(mut socket: WebSocket, handler: Handler) {
    let mut conn = WsConnection::new(handler);

    while let Some(msg) = socket.recv().await {
        if let Ok(msg) = msg {
            match conn.handle_msg(&mut socket, msg).await {
                Ok(None) => {
                    trace!(target: "rpc::ws", "ws client disconnected gracefully");
                    return
                }
                Err(err) => {
                    trace!(target: "rpc::ws", "ws client disconnected {:?}", err);
                    return
                }
                _ => {}
            }
        } else {
            trace!(target: "rpc::ws", "client disconnected");
            return
        }
    }
}

/// The general purpose trait for handling RPC requests via websockets
#[async_trait::async_trait]
pub trait WsRpcHandler: Clone + Send + Sync + 'static {
    /// The request type to expect
    type Request: DeserializeOwned + Send + Sync;
    /// The identifier to use for subscriptions
    type SubscriptionId: Hash + PartialEq + Eq + Send + Sync + fmt::Debug;
    /// The subscription type this handle may create
    type Subscription: Stream<Item = ResponseResult> + Send + Sync;

    /// Invoked when the request was received
    async fn on_request(&self, request: Self::Request, cx: WsContext<Self>) -> ResponseResult;
}

/// Contains additional context and tracks subscriptions
pub struct WsContext<Handler: WsRpcHandler> {
    /// all active subscriptions `id -> Stream`
    subscriptions: Arc<Mutex<HashMap<Handler::SubscriptionId, Handler::Subscription>>>,
}

// === impl WsContext ===

impl<Handler: WsRpcHandler> WsContext<Handler> {
    /// Adds new active subscription
    ///
    /// Returns the previous subscription, if any
    pub fn add_subscription(
        &self,
        id: Handler::SubscriptionId,
        subscription: Handler::Subscription,
    ) -> Option<Handler::Subscription> {
        trace!(target: "rpc::ws", "adding subscription id {:?}", id);
        self.subscriptions.lock().insert(id, subscription)
    }

    /// Removes an existing subscription
    pub fn remove_subscription(
        &self,
        id: &Handler::SubscriptionId,
    ) -> Option<Handler::Subscription> {
        trace!(target: "rpc::ws", "removing subscription id {:?}", id);
        self.subscriptions.lock().remove(id)
    }
}

impl<Handler: WsRpcHandler> Clone for WsContext<Handler> {
    fn clone(&self) -> Self {
        Self { subscriptions: Arc::clone(&self.subscriptions) }
    }
}

impl<Handler: WsRpcHandler> Default for WsContext<Handler> {
    fn default() -> Self {
        Self { subscriptions: Arc::new(Mutex::new(HashMap::new())) }
    }
}

impl<Handler: WsRpcHandler> Stream for WsContext<Handler> {
    type Item = ResponseResult;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        todo!()
    }
}

/// A compatibility helper type to use common `RpcHandler` functions
struct ContextAwareHandler<Handler: WsRpcHandler> {
    handler: Handler,
    context: WsContext<Handler>,
}

impl<Handler: WsRpcHandler> Clone for ContextAwareHandler<Handler> {
    fn clone(&self) -> Self {
        Self { handler: self.handler.clone(), context: self.context.clone() }
    }
}

#[async_trait::async_trait]
impl<Handler: WsRpcHandler> RpcHandler for ContextAwareHandler<Handler> {
    type Request = Handler::Request;

    async fn on_request(&self, request: Self::Request) -> ResponseResult {
        self.handler.on_request(request, self.context.clone()).await
    }
}

/// Represents a connection to a client via websocket
///
/// Contains the state for the entire connection
struct WsConnection<Handler: WsRpcHandler> {
    /// the handler for the websocket connection
    handler: Handler,
    /// contains all the subscription related context
    context: WsContext<Handler>,
}

// === impl WsConnection ===

impl<Handler: WsRpcHandler> WsConnection<Handler> {
    pub fn new(handler: Handler) -> Self {
        Self { handler, context: Default::default() }
    }

    /// Returns a compatibility `RpcHandler`
    fn compat_helper(&self) -> ContextAwareHandler<Handler> {
        ContextAwareHandler { handler: self.handler.clone(), context: self.context.clone() }
    }

    async fn handle_msg(
        &mut self,
        socket: &mut WebSocket,
        msg: Message,
    ) -> Result<Option<()>, axum::Error> {
        match msg {
            Message::Text(text) => {
                trace!(target: "rpc::ws", "client send str: {:?}", text);
                self.handle_text(socket, text).await?;
            }
            Message::Binary(_) => {
                warn!(target: "rpc::ws","unexpected binary data");
                return Ok(None)
            }
            Message::Close(_) => {
                trace!(target: "rpc::ws", "ws client disconnected");
                return Ok(None)
            }
            Message::Ping(ping) => {
                trace!(target: "rpc::ws", "received ping");
                socket.send(Message::Pong(ping)).await?;
            }
            _ => {}
        }
        Ok(Some(()))
    }

    async fn get_rpc_response(&self, text: String) -> Response {
        match serde_json::from_str::<Request>(&text) {
            Ok(req) => handle_request(req, self.compat_helper())
                .await
                .unwrap_or_else(|| Response::error(RpcError::invalid_request())),
            Err(err) => {
                warn!("invalid request={:?}", err);
                Response::error(RpcError::invalid_request())
            }
        }
    }

    async fn handle_text(
        &mut self,
        socket: &mut WebSocket,
        text: String,
    ) -> Result<(), axum::Error> {
        let resp = self.get_rpc_response(text).await;
        match serde_json::to_string(&resp) {
            Ok(txt) => {
                socket.send(Message::Text(txt)).await?;
                Ok(())
            }
            Err(err) => Err(axum::Error::new(err)),
        }
    }
}

impl<Handler: WsRpcHandler> Stream for WsConnection<Handler> {
    type Item = ();

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        todo!()
    }
}
