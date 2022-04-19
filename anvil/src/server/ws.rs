use crate::{
    server::{handler::handle_request, RpcHandler},
    EthApi,
};
use anvil_core::eth::EthRpcCall;
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
use tracing::{trace, warn};

/// A `RpcHandler` that expects `EthRequest` rpc calls and `EthPubSub` via websocket
#[derive(Clone)]
pub struct WsEthRpcHandler {
    /// Access to the node
    api: EthApi,
}

#[async_trait::async_trait]
impl RpcHandler for WsEthRpcHandler {
    type Request = EthRpcCall;

    async fn on_request(&self, request: Self::Request) -> ResponseResult {
        todo!()
    }
}

/// Handles incoming Websocket upgrade
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Extension(api): Extension<EthApi>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_ws_socket(socket, api))
}

async fn handle_ws_socket(mut socket: WebSocket, api: EthApi) {
    let mut conn = WsConnection::new(api);

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

/// Represents a connection to a client via websocket
///
/// Contains the state for the entire connection
struct WsConnection {
    /// access to the ethereum api
    api: EthApi,
}

impl WsConnection {
    pub fn new(api: EthApi) -> Self {
        Self { api }
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
            Ok(req) => handle_request(req, self.api.clone())
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
