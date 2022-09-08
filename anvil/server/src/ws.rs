use crate::{pubsub::PubSubConnection, PubSubRpcHandler};
use axum::{extract::WebSocketUpgrade, response::IntoResponse, Extension};

/// Handles incoming Websocket upgrade
///
/// This is the entrypoint invoked by the axum server for a websocket request
pub async fn handle_ws<Handler: PubSubRpcHandler>(
    ws: WebSocketUpgrade,
    Extension(handler): Extension<Handler>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| PubSubConnection::new(socket, handler))
}
