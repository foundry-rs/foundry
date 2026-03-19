//! MPP (Machine Payments Protocol) HTTP transport.
//!
//! Wraps a standard reqwest HTTP transport with automatic 402 Payment Required
//! handling via the MPP protocol. When the RPC endpoint returns a 402 response,
//! this transport automatically pays the challenge and retries the request.

use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_transport::{TransportError, TransportErrorKind, TransportFut, TransportResult};
use mpp::{
    MppError, PaymentChallenge,
    protocol::core::{
        AUTHORIZATION_HEADER, PaymentCredential, PaymentPayload, WWW_AUTHENTICATE_HEADER,
        format_authorization, parse_www_authenticate,
    },
};
use reqwest::StatusCode;
use std::task;
use tower::Service;
use tracing::{Instrument, debug, debug_span, trace};
use url::Url;

/// HTTP transport that automatically handles MPP 402 challenges.
///
/// When an RPC endpoint is 402-gated, this transport:
/// 1. Sends the initial JSON-RPC request
/// 2. If 402 is returned, parses the challenge from `WWW-Authenticate`
/// 3. Pays via the configured [`MppSigner`]
/// 4. Retries the request with the payment credential
#[derive(Clone)]
pub struct MppHttpTransport {
    client: reqwest::Client,
    url: Url,
    signer: MppSigner,
}

impl std::fmt::Debug for MppHttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MppHttpTransport").field("url", &self.url).finish_non_exhaustive()
    }
}

impl MppHttpTransport {
    /// Create a new MPP HTTP transport.
    pub fn new(client: reqwest::Client, url: Url, signer: MppSigner) -> Self {
        Self { client, url, signer }
    }

    /// Send a JSON-RPC request with automatic 402 payment handling.
    async fn do_request(self, req: RequestPacket) -> TransportResult<ResponsePacket> {
        let body = serde_json::to_vec(&req).map_err(TransportErrorKind::custom)?;
        let headers = req.headers();

        let resp = self
            .client
            .post(self.url.clone())
            .headers(headers.clone())
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
            .map_err(TransportErrorKind::custom)?;

        // If not 402, handle normally
        if resp.status() != StatusCode::PAYMENT_REQUIRED {
            return Self::handle_response(resp).await;
        }

        // Parse the 402 challenge
        let www_auth = resp
            .headers()
            .get(WWW_AUTHENTICATE_HEADER)
            .or_else(|| resp.headers().get("www-authenticate"))
            .ok_or_else(|| {
                TransportErrorKind::custom(std::io::Error::other(
                    "402 response missing WWW-Authenticate header",
                ))
            })?
            .to_str()
            .map_err(|e| {
                TransportErrorKind::custom(std::io::Error::other(format!(
                    "invalid WWW-Authenticate header: {e}"
                )))
            })?;

        let challenge = parse_www_authenticate(www_auth).map_err(|e| {
            TransportErrorKind::custom(std::io::Error::other(format!("invalid MPP challenge: {e}")))
        })?;

        debug!(id = %challenge.id, method = %challenge.method, "received MPP 402 challenge, paying");

        // Pay the challenge
        let credential = self.signer.pay(&challenge).await.map_err(|e| {
            TransportErrorKind::custom(std::io::Error::other(format!("MPP payment failed: {e}")))
        })?;

        let auth_header = format_authorization(&credential).map_err(|e| {
            TransportErrorKind::custom(std::io::Error::other(format!(
                "failed to format MPP credential: {e}"
            )))
        })?;

        // Retry with payment credential
        let retry_resp = self
            .client
            .post(self.url.clone())
            .headers(headers)
            .header("content-type", "application/json")
            .header(AUTHORIZATION_HEADER, auth_header)
            .body(body)
            .send()
            .await
            .map_err(TransportErrorKind::custom)?;

        Self::handle_response(retry_resp).await
    }

    async fn handle_response(resp: reqwest::Response) -> TransportResult<ResponsePacket> {
        let status = resp.status();
        debug!(%status, "received response from MPP transport");

        let body = resp.bytes().await.map_err(TransportErrorKind::custom)?;

        if tracing::enabled!(tracing::Level::TRACE) {
            trace!(body = %String::from_utf8_lossy(&body), "response body");
        } else {
            debug!(bytes = body.len(), "retrieved response body");
        }

        if !status.is_success() {
            return Err(TransportErrorKind::http_error(
                status.as_u16(),
                String::from_utf8_lossy(&body).into_owned(),
            ));
        }

        serde_json::from_slice(&body)
            .map_err(|err| TransportError::deser_err(err, String::from_utf8_lossy(&body)))
    }
}

impl Service<RequestPacket> for MppHttpTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    #[inline]
    fn poll_ready(&mut self, _cx: &mut task::Context<'_>) -> task::Poll<Result<(), Self::Error>> {
        task::Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&mut self, req: RequestPacket) -> Self::Future {
        let this = self.clone();
        let span = debug_span!("MppHttpTransport", url = %this.url);
        Box::pin(this.do_request(req).instrument(span.or_current()))
    }
}

/// Signs MPP payment challenges using an EVM private key.
///
/// Produces a credential by signing the challenge ID with EIP-191 personal_sign.
#[derive(Clone)]
pub struct MppSigner {
    signer: alloy_signer_local::PrivateKeySigner,
}

impl MppSigner {
    /// Create a new MPP signer.
    pub fn new(signer: alloy_signer_local::PrivateKeySigner) -> Self {
        Self { signer }
    }

    /// Sign an MPP challenge and produce a credential.
    async fn pay(&self, challenge: &PaymentChallenge) -> Result<PaymentCredential, MppError> {
        use alloy_signer::Signer;

        let message = format!("MPP Payment: {}", challenge.id);
        let signature = self
            .signer
            .sign_message(message.as_bytes())
            .await
            .map_err(|e| MppError::Http(format!("failed to sign: {e}")))?;

        let addr = self.signer.address();
        Ok(PaymentCredential::with_source(
            challenge.to_echo(),
            format!("did:pkh:eip155:1:{addr}"),
            PaymentPayload::hash(format!(
                "0x{}",
                alloy_primitives::hex::encode(signature.as_bytes())
            )),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_rpc::{Id, Request, RequestMeta};
    use axum::response::IntoResponse;
    use mpp::protocol::core::{Base64UrlJson, format_www_authenticate, parse_authorization};
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    /// Build a test challenge and its formatted WWW-Authenticate header.
    fn test_challenge() -> (PaymentChallenge, String) {
        let request = Base64UrlJson::from_value(&serde_json::json!({"amount": "1000"})).unwrap();
        let challenge =
            PaymentChallenge::new("test-id-42", "rpc.example.com", "tempo", "charge", request);
        let header = format_www_authenticate(&challenge).unwrap();
        (challenge, header)
    }

    /// Build a minimal JSON-RPC request packet.
    fn test_request() -> RequestPacket {
        let req = Request {
            meta: RequestMeta::new("eth_blockNumber".into(), Id::Number(1)),
            params: serde_json::value::RawValue::from_string("[]".into()).unwrap(),
        };
        req.serialize().unwrap().into()
    }

    /// Spawn an axum server and return its base URL.
    async fn spawn_server(app: axum::Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn test_mpp_signer_produces_valid_credential() {
        let signer = alloy_signer_local::PrivateKeySigner::random();
        let mpp_signer = MppSigner::new(signer.clone());

        let (challenge, _) = test_challenge();
        let credential = mpp_signer.pay(&challenge).await.unwrap();

        // Credential echoes the challenge fields
        assert_eq!(credential.challenge.id, "test-id-42");
        assert_eq!(credential.challenge.method.as_str(), "tempo");

        // Source contains the signer address
        let source = credential.source.as_ref().unwrap();
        assert!(source.starts_with("did:pkh:eip155:1:"));
        assert!(source.contains(&format!("{}", signer.address())));

        // Payload is a hash type with a 0x-prefixed hex signature
        let payload: PaymentPayload = serde_json::from_value(credential.payload).unwrap();
        assert!(payload.is_hash());
        assert!(payload.data().starts_with("0x"));
    }

    #[tokio::test]
    async fn test_mpp_transport_non_402_passthrough() {
        use axum::routing::post;

        // Server always returns 200 with a JSON-RPC response
        let app = axum::Router::new().route(
            "/",
            post(|| async {
                axum::Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": "0x1234"
                }))
            }),
        );

        let (base_url, handle) = spawn_server(app).await;
        let signer = MppSigner::new(alloy_signer_local::PrivateKeySigner::random());
        let mut transport =
            MppHttpTransport::new(reqwest::Client::new(), Url::parse(&base_url).unwrap(), signer);

        let resp = tower::Service::call(&mut transport, test_request()).await.unwrap();

        // Should get the response directly without any payment flow
        match resp {
            ResponsePacket::Single(r) => {
                assert!(r.is_success());
            }
            _ => panic!("expected single response"),
        }

        handle.abort();
    }

    #[tokio::test]
    async fn test_mpp_transport_402_then_200() {
        use axum::{extract::State, http::StatusCode as AxumStatusCode, routing::post};

        let (_, www_auth) = test_challenge();
        let call_count = Arc::new(AtomicU32::new(0));

        #[derive(Clone)]
        struct AppState {
            www_auth: String,
            call_count: Arc<AtomicU32>,
        }

        let state = AppState { www_auth, call_count: call_count.clone() };

        let app =
            axum::Router::new()
                .route(
                    "/",
                    post(
                        |State(state): State<AppState>,
                         req: axum::http::Request<axum::body::Body>| async move {
                            state.call_count.fetch_add(1, Ordering::SeqCst);

                            if req.headers().get("authorization").is_some() {
                                // Second request: has payment credential → return success
                                (
                                    AxumStatusCode::OK,
                                    axum::Json(serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": 1,
                                        "result": "0xpaid"
                                    })),
                                )
                                    .into_response()
                            } else {
                                // First request: return 402 with challenge
                                (
                                    AxumStatusCode::PAYMENT_REQUIRED,
                                    [("www-authenticate", state.www_auth)],
                                    "Payment Required",
                                )
                                    .into_response()
                            }
                        },
                    ),
                )
                .with_state(state);

        let (base_url, handle) = spawn_server(app).await;
        let signer = MppSigner::new(alloy_signer_local::PrivateKeySigner::random());
        let mut transport =
            MppHttpTransport::new(reqwest::Client::new(), Url::parse(&base_url).unwrap(), signer);

        let resp = tower::Service::call(&mut transport, test_request()).await.unwrap();

        // Should have made 2 calls: initial 402 + retry with credential
        assert_eq!(call_count.load(Ordering::SeqCst), 2);

        match resp {
            ResponsePacket::Single(r) => {
                assert!(r.is_success());
            }
            _ => panic!("expected single response"),
        }

        handle.abort();
    }

    #[tokio::test]
    async fn test_mpp_transport_402_credential_is_valid() {
        use axum::{extract::State, http::StatusCode as AxumStatusCode, routing::post};

        let (_, www_auth) = test_challenge();

        #[derive(Clone)]
        struct AppState {
            www_auth: String,
        }

        let state = AppState { www_auth };

        let app =
            axum::Router::new()
                .route(
                    "/",
                    post(
                        |State(state): State<AppState>,
                         req: axum::http::Request<axum::body::Body>| async move {
                            if let Some(auth) = req.headers().get("authorization") {
                                // Validate the credential is parseable
                                let auth_str = auth.to_str().unwrap();
                                let credential = parse_authorization(auth_str).unwrap();
                                assert_eq!(credential.challenge.id, "test-id-42");
                                assert_eq!(credential.challenge.method.as_str(), "tempo");
                                assert!(credential.source.is_some());

                                (
                                    AxumStatusCode::OK,
                                    axum::Json(serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": 1,
                                        "result": "0xvalidated"
                                    })),
                                )
                                    .into_response()
                            } else {
                                (
                                    AxumStatusCode::PAYMENT_REQUIRED,
                                    [("www-authenticate", state.www_auth)],
                                    "Payment Required",
                                )
                                    .into_response()
                            }
                        },
                    ),
                )
                .with_state(state);

        let (base_url, handle) = spawn_server(app).await;
        let signer = MppSigner::new(alloy_signer_local::PrivateKeySigner::random());
        let mut transport =
            MppHttpTransport::new(reqwest::Client::new(), Url::parse(&base_url).unwrap(), signer);

        // If the credential is invalid, the server-side assert will panic
        // and the request will fail
        let resp = tower::Service::call(&mut transport, test_request()).await.unwrap();
        match resp {
            ResponsePacket::Single(r) => assert!(r.is_success()),
            _ => panic!("expected single response"),
        }

        handle.abort();
    }

    #[tokio::test]
    async fn test_mpp_transport_402_missing_www_authenticate() {
        use axum::{http::StatusCode as AxumStatusCode, routing::post};

        // Server returns 402 without WWW-Authenticate header
        let app = axum::Router::new()
            .route("/", post(|| async { (AxumStatusCode::PAYMENT_REQUIRED, "pay up") }));

        let (base_url, handle) = spawn_server(app).await;
        let signer = MppSigner::new(alloy_signer_local::PrivateKeySigner::random());
        let mut transport =
            MppHttpTransport::new(reqwest::Client::new(), Url::parse(&base_url).unwrap(), signer);

        let err = tower::Service::call(&mut transport, test_request()).await.unwrap_err();
        assert!(
            err.to_string().contains("WWW-Authenticate"),
            "expected WWW-Authenticate error, got: {err}"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn test_mpp_transport_via_runtime_transport() {
        use crate::provider::runtime_transport::RuntimeTransportBuilder;
        use axum::{extract::State, http::StatusCode as AxumStatusCode, routing::post};

        let (_, www_auth) = test_challenge();
        let call_count = Arc::new(AtomicU32::new(0));

        #[derive(Clone)]
        struct AppState {
            www_auth: String,
            call_count: Arc<AtomicU32>,
        }

        let state = AppState { www_auth, call_count: call_count.clone() };

        let app =
            axum::Router::new()
                .route(
                    "/",
                    post(
                        |State(state): State<AppState>,
                         req: axum::http::Request<axum::body::Body>| async move {
                            state.call_count.fetch_add(1, Ordering::SeqCst);
                            if req.headers().get("authorization").is_some() {
                                (
                                    AxumStatusCode::OK,
                                    axum::Json(serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": 1,
                                        "result": "0xmpp_works"
                                    })),
                                )
                                    .into_response()
                            } else {
                                (
                                    AxumStatusCode::PAYMENT_REQUIRED,
                                    [("www-authenticate", state.www_auth)],
                                    "Payment Required",
                                )
                                    .into_response()
                            }
                        },
                    ),
                )
                .with_state(state);

        let (base_url, handle) = spawn_server(app).await;

        // Write a temp keys.toml and point TEMPO_HOME at it for auto-discovery.
        let signer = alloy_signer_local::PrivateKeySigner::random();
        let mpp_key = alloy_primitives::hex::encode(signer.credential().to_bytes());
        let dir = tempfile::tempdir().unwrap();
        let wallet_dir = dir.path().join("wallet");
        std::fs::create_dir_all(&wallet_dir).unwrap();
        std::fs::write(
            wallet_dir.join("keys.toml"),
            format!("[[keys]]\nkey = \"{mpp_key}\"\n"),
        )
        .unwrap();
        // SAFETY: test-only env manipulation.
        unsafe {
            std::env::set_var("TEMPO_HOME", dir.path());
            std::env::remove_var("TEMPO_PRIVATE_KEY");
        }

        let transport = RuntimeTransportBuilder::new(Url::parse(&base_url).unwrap()).build();

        let resp = transport.request(test_request()).await.unwrap();

        assert_eq!(call_count.load(Ordering::SeqCst), 2);
        match resp {
            ResponsePacket::Single(r) => assert!(r.is_success()),
            _ => panic!("expected single response"),
        }

        handle.abort();
        unsafe { std::env::remove_var("TEMPO_HOME") };
    }

    /// End-to-end test against the live `rpc.tempo.xyz` 402-gated endpoint.
    ///
    /// Requires a valid Tempo wallet key via `TEMPO_PRIVATE_KEY` env var or
    /// `~/.tempo/wallet/keys.toml`. Skipped in CI — run manually with:
    ///
    /// ```sh
    /// cargo test -p foundry-common test_mpp_live_rpc_tempo -- --ignored
    /// ```
    #[tokio::test]
    #[ignore = "requires network access and a funded Tempo wallet key"]
    async fn test_mpp_live_rpc_tempo() {
        use crate::provider::runtime_transport::RuntimeTransportBuilder;

        let transport =
            RuntimeTransportBuilder::new(Url::parse("https://rpc.tempo.xyz").unwrap()).build();

        let resp = transport.request(test_request()).await.unwrap();

        match resp {
            ResponsePacket::Single(r) => {
                assert!(r.is_success(), "expected successful response, got: {r:?}");
            }
            _ => panic!("expected single response"),
        }
    }
}
