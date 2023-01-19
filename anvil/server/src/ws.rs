use crate::{error::RequestError, pubsub::PubSubConnection, PubSubRpcHandler};
use anvil_rpc::request::Request;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        WebSocketUpgrade,
    },
    response::IntoResponse,
    Extension,
};
use futures::{ready, Sink, Stream};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tracing::trace;

/// Handles incoming Websocket upgrade
///
/// This is the entrypoint invoked by the axum server for a websocket request
pub async fn handle_ws<Handler: PubSubRpcHandler>(
    ws: WebSocketUpgrade,
    Extension(handler): Extension<Handler>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| PubSubConnection::new(SocketConn(socket), handler))
}

#[pin_project::pin_project]
struct SocketConn(#[pin] WebSocket);

impl Stream for SocketConn {
    type Item = Result<Option<Request>, RequestError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match ready!(self.project().0.poll_next(cx)) {
            Some(msg) => Poll::Ready(Some(on_message(msg))),
            _ => Poll::Ready(None),
        }
    }
}

impl Sink<String> for SocketConn {
    type Error = axum::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().0.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: String) -> Result<(), Self::Error> {
        self.project().0.start_send(Message::Text(item))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().0.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().0.poll_close(cx)
    }
}

fn on_message(msg: Result<Message, axum::Error>) -> Result<Option<Request>, RequestError> {
    match msg? {
        Message::Text(text) => Ok(Some(serde_json::from_str(&text)?)),
        Message::Binary(data) => {
            // the binary payload type is the request as-is but as bytes, if this is a valid
            // `Request` then we can deserialize the Json from the data Vec
            Ok(Some(serde_json::from_slice(&data)?))
        }
        Message::Close(_) => {
            trace!(target: "rpc::ws", "ws client disconnected");
            Err(RequestError::Disconnect)
        }
        _ => Ok(None),
    }
}
