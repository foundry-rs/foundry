//! MPP WebSocket transport.
//!
//! Implements [`PubSubConnect`] with automatic MPP 402 challenge/credential
//! handshake at WebSocket connect time. Non-MPP servers are handled via a
//! timeout-based fallback: if no challenge frame arrives within a short window
//! after connecting, we assume the server is a plain JSON-RPC WebSocket.

use alloy_json_rpc::PubSubItem;
use alloy_pubsub::{ConnectionHandle, PubSubConnect};
use alloy_transport::{Authorization, TransportErrorKind, TransportResult, utils::guess_local_url};
use alloy_transport_ws::WsBackend;
use futures::{SinkExt, StreamExt};
use mpp::{
    client::{
        PaymentProvider,
        ws::{WsClientMessage, WsServerMessage},
    },
    protocol::core::{PaymentChallenge, format_authorization},
};
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use std::{io, time::Duration};
use tokio::time::timeout;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        http::{HeaderValue, header::AUTHORIZATION},
    },
};
use tracing::debug;

use super::{
    keys::DiscoverOptions,
    transport::{LazySessionProvider, extract_challenge_chain_and_currency},
};

type TungsteniteStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Timeout for waiting on an MPP challenge frame from the server.
///
/// If no challenge arrives within this window after the WebSocket upgrade
/// completes, we assume it's a plain (non-MPP) JSON-RPC WebSocket.
const MPP_CHALLENGE_TIMEOUT: Duration = Duration::from_millis(500);

/// Keepalive ping interval (matches alloy-transport-ws default).
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// WebSocket connector with automatic MPP payment at connect time.
///
/// Implements [`PubSubConnect`] so it can be used as a drop-in replacement for
/// alloy's `WsConnect`. On connect, it:
///
/// 1. Opens a WebSocket connection.
/// 2. Waits briefly for an MPP challenge frame from the server.
/// 3. If a challenge arrives, performs the payment handshake using [`LazySessionProvider`] (same
///    payment logic as the HTTP transport).
/// 4. Spawns a backend loop that bridges the authenticated WebSocket to the alloy
///    [`PubSubFrontend`](alloy_pubsub::PubSubFrontend).
///
/// Non-MPP servers work transparently — the timeout expires and the backend
/// proceeds with normal JSON-RPC message forwarding.
#[derive(Clone, Debug)]
pub struct MppWsConnect {
    url: String,
    auth: Option<Authorization>,
    provider: LazySessionProvider,
}

impl MppWsConnect {
    /// Create a new MPP WebSocket connector for the given URL.
    pub fn new(url: String) -> Self {
        let origin = url.clone();
        let auth =
            url::Url::parse(&url).ok().and_then(|parsed| Authorization::extract_from_url(&parsed));
        Self { url, auth, provider: LazySessionProvider::new(origin) }
    }

    /// Set the authorization header (e.g. JWT bearer token).
    pub fn with_auth(mut self, auth: Authorization) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Attempt the MPP handshake on an already-connected WebSocket.
    ///
    /// Waits up to [`MPP_CHALLENGE_TIMEOUT`] for a challenge frame. If one
    /// arrives, pays it and sends the credential back. Returns any buffered
    /// non-challenge messages that arrived during the handshake window.
    async fn try_mpp_handshake(
        socket: &mut TungsteniteStream,
        provider: &LazySessionProvider,
    ) -> TransportResult<Vec<String>> {
        let mut buffered_messages: Vec<String> = Vec::new();

        // Wait briefly for a challenge frame.
        let first_msg = timeout(MPP_CHALLENGE_TIMEOUT, socket.next()).await;

        let challenge_frame = match first_msg {
            Err(_) => {
                // Timeout — not an MPP server.
                debug!("no MPP challenge within timeout, treating as plain WS");
                return Ok(buffered_messages);
            }
            Ok(None) => {
                return Err(TransportErrorKind::custom(io::Error::other(
                    "WebSocket closed before any message",
                )));
            }
            Ok(Some(Err(e))) => {
                return Err(TransportErrorKind::custom(e));
            }
            Ok(Some(Ok(msg))) => msg,
        };

        let text = match &challenge_frame {
            Message::Text(t) => t.as_ref(),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                return Ok(buffered_messages);
            }
            Message::Binary(_) | Message::Close(_) => {
                return Err(TransportErrorKind::custom(io::Error::other(
                    "unexpected binary/close frame on WS connect",
                )));
            }
        };

        // Try to parse as an MPP server message.
        let server_msg: WsServerMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(_) => {
                // Not an MPP message — buffer it for the backend.
                buffered_messages.push(text.to_owned());
                return Ok(buffered_messages);
            }
        };

        let challenge: PaymentChallenge = match server_msg {
            WsServerMessage::Challenge { ref challenge, .. } => {
                serde_json::from_value(challenge.clone()).map_err(|e| {
                    TransportErrorKind::custom(io::Error::other(format!(
                        "failed to parse MPP WS challenge: {e}"
                    )))
                })?
            }
            _ => {
                // Non-challenge MPP message — buffer it.
                buffered_messages.push(text.to_owned());
                return Ok(buffered_messages);
            }
        };

        debug!(id = %challenge.id, method = %challenge.method, intent = %challenge.intent, "received MPP WS challenge, paying");

        // Resolve the payment provider (lazily discovers keys on first use).
        let (chain_id, currency) = extract_challenge_chain_and_currency(&challenge);
        let currency = currency.and_then(|s| s.parse().ok());
        let session =
            provider.get_or_init(DiscoverOptions { chain_id, currency }).map_err(|e| {
                TransportErrorKind::custom(io::Error::other(format!(
                    "MPP key discovery failed: {e}"
                )))
            })?;

        let credential = session.pay(&challenge).await.map_err(|e| {
            TransportErrorKind::custom(io::Error::other(format!("MPP WS payment failed: {e}")))
        })?;

        // Everything after pay() must rollback on failure — wrap so we can't
        // miss an error path.
        let result = async {
            let auth_header = format_authorization(&credential).map_err(|e| {
                TransportErrorKind::custom(io::Error::other(format!(
                    "failed to format MPP credential: {e}"
                )))
            })?;

            // Send credential as a WS message.
            let cred_msg = WsClientMessage::Credential { credential: auth_header };
            let cred_text = serde_json::to_string(&cred_msg).map_err(|e| {
                TransportErrorKind::custom(io::Error::other(format!(
                    "failed to serialize credential message: {e}"
                )))
            })?;

            socket.send(Message::Text(cred_text.into())).await.map_err(|e| {
                TransportErrorKind::custom(io::Error::other(format!(
                    "failed to send MPP credential: {e}"
                )))
            })?;

            // Wait for server acknowledgement (receipt or data).
            let ack = timeout(Duration::from_secs(30), socket.next()).await.map_err(|_| {
                TransportErrorKind::custom(io::Error::other(
                    "timeout waiting for MPP server acknowledgement",
                ))
            })?;

            match ack {
                None => {
                    return Err(TransportErrorKind::custom(io::Error::other(
                        "WebSocket closed after sending credential",
                    )));
                }
                Some(Err(e)) => return Err(TransportErrorKind::custom(e)),
                Some(Ok(Message::Text(t))) => {
                    if let Ok(msg) = serde_json::from_str::<WsServerMessage>(t.as_ref()) {
                        match msg {
                            WsServerMessage::Receipt { .. } => {
                                debug!("MPP WS handshake complete (receipt received)");
                            }
                            WsServerMessage::Error { error } => {
                                return Err(TransportErrorKind::custom(io::Error::other(format!(
                                    "MPP WS server error: {error}"
                                ))));
                            }
                            _ => {
                                buffered_messages.push(t.to_string());
                            }
                        }
                    } else {
                        buffered_messages.push(t.to_string());
                    }
                }
                Some(Ok(Message::Close(_))) => {
                    return Err(TransportErrorKind::custom(io::Error::other(
                        "WebSocket closed after sending credential",
                    )));
                }
                Some(Ok(_)) => {}
            }

            Ok(buffered_messages)
        }
        .await;

        match &result {
            Ok(_) => provider.flush_pending(),
            Err(_) => provider.rollback_pending(),
        }

        result
    }
}

impl PubSubConnect for MppWsConnect {
    fn is_local(&self) -> bool {
        guess_local_url(&self.url)
    }

    async fn connect(&self) -> TransportResult<ConnectionHandle> {
        let mut request =
            self.url.as_str().into_client_request().map_err(TransportErrorKind::custom)?;

        if let Some(ref auth) = self.auth {
            let mut auth_value =
                HeaderValue::from_str(&auth.to_string()).map_err(TransportErrorKind::custom)?;
            auth_value.set_sensitive(true);
            request.headers_mut().insert(AUTHORIZATION, auth_value);
        }

        // Install the default rustls crypto provider (required by rustls 0.23+).
        let _ = CryptoProvider::install_default(aws_lc_rs::default_provider());

        let (mut socket, _) = connect_async(request).await.map_err(TransportErrorKind::custom)?;

        // Attempt MPP handshake (timeout-based fallback for non-MPP servers).
        let buffered = Self::try_mpp_handshake(&mut socket, &self.provider).await?;

        let (handle, interface) = ConnectionHandle::new();

        // Replay any messages that arrived during the handshake window.
        for msg in &buffered {
            if let Ok(item) = serde_json::from_str::<PubSubItem>(msg) {
                let _ = interface.send_to_frontend(item);
            }
        }

        // Reuse alloy's WsBackend for the post-handshake JSON-RPC loop.
        WsBackend::from_socket(socket, interface, KEEPALIVE_INTERVAL).spawn();

        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_rpc::{Id, Request, RequestMeta, RequestPacket, ResponsePacket};
    use alloy_primitives::hex;
    use mpp::{
        PrivateKeySigner,
        protocol::core::{Base64UrlJson, IntentName, MethodName, parse_authorization},
    };
    use tokio::task::JoinHandle;
    use tokio_tungstenite::accept_async;
    use tower::Service;

    fn test_challenge() -> PaymentChallenge {
        let request = Base64UrlJson::from_value(&serde_json::json!({
            "amount": "1000",
            "currency": "0x0000000000000000000000000000000000000000",
            "recipient": "0x0000000000000000000000000000000000000001",
            "methodDetails": {
                "chainId": 42431
            }
        }))
        .unwrap();

        PaymentChallenge {
            id: "ws-test-id".to_string(),
            realm: "test-realm".to_string(),
            method: MethodName::new("tempo"),
            intent: IntentName::new("session"),
            request,
            expires: None,
            description: None,
            digest: None,
            opaque: None,
        }
    }

    /// Spawn a WS server on localhost, returns (ws_url, join_handle).
    /// `handler` receives the server-side socket for full control.
    async fn spawn_ws_server<F, Fut>(handler: F) -> (String, JoinHandle<()>)
    where
        F: FnOnce(WebSocketStream<tokio::net::TcpStream>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = accept_async(stream).await.unwrap();
            handler(ws).await;
        });
        (format!("ws://{addr}"), handle)
    }

    /// Plain WS server (no MPP) — connect and send a JSON-RPC request.
    #[tokio::test]
    async fn test_ws_no_mpp_plain_jsonrpc() {
        let (url, server) = spawn_ws_server(|mut ws| async move {
            // Wait for a JSON-RPC request, echo a response.
            while let Some(Ok(msg)) = ws.next().await {
                if let Message::Text(text) = msg {
                    let req: serde_json::Value = serde_json::from_str(&text).unwrap();
                    let id = req.get("id").unwrap().clone();
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": "0xabc"
                    });
                    ws.send(Message::Text(resp.to_string().into())).await.unwrap();
                }
            }
        })
        .await;

        let connector = MppWsConnect::new(url);
        let mut frontend = connector.into_service().await.unwrap();

        let req = Request {
            meta: RequestMeta::new("eth_blockNumber".into(), Id::Number(1)),
            params: serde_json::Value::Array(vec![]),
        };
        let packet = RequestPacket::Single(req.serialize().unwrap());
        let resp = frontend.call(packet).await.unwrap();

        match resp {
            ResponsePacket::Single(r) => assert!(r.is_success()),
            _ => panic!("expected single response"),
        }

        server.abort();
    }

    /// MPP server sends challenge → client pays → server sends receipt.
    #[tokio::test]
    async fn test_ws_mpp_challenge_credential_receipt() {
        let challenge = test_challenge();
        let challenge_json = serde_json::to_value(&challenge).unwrap();

        let (url, server) = spawn_ws_server(move |mut ws| async move {
            // Send challenge.
            let challenge_msg = serde_json::json!({
                "type": "challenge",
                "challenge": challenge_json
            });
            ws.send(Message::Text(challenge_msg.to_string().into())).await.unwrap();

            // Receive credential.
            let msg = ws.next().await.unwrap().unwrap();
            let text = match msg {
                Message::Text(t) => t,
                other => panic!("expected text, got {other:?}"),
            };
            let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(parsed["type"], "credential");
            // Verify it's a valid MPP credential.
            let cred_str = parsed["credential"].as_str().unwrap();
            let cred = parse_authorization(cred_str).unwrap();
            assert_eq!(cred.challenge.id, "ws-test-id");

            // Send receipt.
            let receipt_msg = serde_json::json!({
                "type": "receipt",
                "receipt": { "id": "ws-test-id" }
            });
            ws.send(Message::Text(receipt_msg.to_string().into())).await.unwrap();

            // Now serve JSON-RPC.
            while let Some(Ok(msg)) = ws.next().await {
                if let Message::Text(text) = msg {
                    let req: serde_json::Value = serde_json::from_str(&text).unwrap();
                    let id = req.get("id").unwrap().clone();
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": "0xpaid"
                    });
                    ws.send(Message::Text(resp.to_string().into())).await.unwrap();
                }
            }
        })
        .await;

        // Set a random private key so LazySessionProvider can initialize.
        let signer = PrivateKeySigner::random();
        let key_hex = hex::encode(signer.to_bytes());
        unsafe { std::env::set_var("TEMPO_PRIVATE_KEY", &key_hex) };

        let connector = MppWsConnect::new(url);
        let mut frontend = connector.into_service().await.unwrap();

        let req = Request {
            meta: RequestMeta::new("eth_blockNumber".into(), Id::Number(1)),
            params: serde_json::Value::Array(vec![]),
        };
        let packet = RequestPacket::Single(req.serialize().unwrap());
        let resp = frontend.call(packet).await.unwrap();

        match resp {
            ResponsePacket::Single(r) => assert!(r.is_success()),
            _ => panic!("expected single response"),
        }

        unsafe { std::env::remove_var("TEMPO_PRIVATE_KEY") };
        server.abort();
    }

    /// MPP server sends challenge, client pays, server closes → rollback.
    #[tokio::test]
    async fn test_ws_mpp_rollback_on_post_pay_close() {
        let challenge = test_challenge();
        let challenge_json = serde_json::to_value(&challenge).unwrap();

        let (url, server) = spawn_ws_server(move |mut ws| async move {
            // Send challenge.
            let challenge_msg = serde_json::json!({
                "type": "challenge",
                "challenge": challenge_json
            });
            ws.send(Message::Text(challenge_msg.to_string().into())).await.unwrap();

            // Receive credential (client paid).
            let _ = ws.next().await;

            // Close without sending receipt — simulates post-pay failure.
            ws.close(None).await.ok();
        })
        .await;

        let signer = PrivateKeySigner::random();
        let key_hex = hex::encode(signer.to_bytes());
        unsafe { std::env::set_var("TEMPO_PRIVATE_KEY", &key_hex) };

        let connector = MppWsConnect::new(url);
        let result = connector.connect().await;

        // Connect must fail — server closed after credential was sent.
        assert!(result.is_err(), "expected error when server closes after payment, got Ok");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("closed") || err.contains("WebSocket"),
            "expected close-related error, got: {err}"
        );

        unsafe { std::env::remove_var("TEMPO_PRIVATE_KEY") };
        server.abort();
    }
}
