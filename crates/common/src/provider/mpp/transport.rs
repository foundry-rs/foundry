//! MPP (Machine Payments Protocol) HTTP transport.
//!
//! Wraps a standard reqwest HTTP transport with automatic 402 Payment Required
//! handling via the MPP protocol. When the RPC endpoint returns a 402 response,
//! this transport automatically pays the challenge and retries the request.
//!
//! - [`MppHttpTransport<P>`]: Generic transport that delegates 402 payment to any `PaymentProvider`
//!   implementation. Used directly in tests with mock providers.
//! - [`LazyMppHttpTransport`]: Production alias that lazily discovers Tempo wallet keys on first
//!   402 response.

use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_transport::{TransportError, TransportErrorKind, TransportFut, TransportResult};
use mpp::{
    client::PaymentProvider,
    protocol::core::{
        AUTHORIZATION_HEADER, WWW_AUTHENTICATE_HEADER, format_authorization, parse_www_authenticate,
    },
};
use reqwest::StatusCode;
use std::{fmt, sync::Mutex, task};
use tower::Service;
use tracing::{Instrument, debug, debug_span, trace};
use url::Url;

use super::{keys::discover_mpp_config, session::SessionProvider};

/// Production transport: lazily discovers MPP keys from the Tempo wallet on
/// first 402 response. Used by [`super::super::runtime_transport::InnerTransport`].
pub type LazyMppHttpTransport = MppHttpTransport<LazySessionProvider>;

/// A payment provider that lazily initializes a [`SessionProvider`] from the
/// Tempo wallet configuration on first use.
#[derive(Clone, Debug)]
pub struct LazySessionProvider {
    inner: std::sync::Arc<Mutex<Option<SessionProvider>>>,
}

impl LazySessionProvider {
    fn new() -> Self {
        Self { inner: std::sync::Arc::new(Mutex::new(None)) }
    }

    /// Get or lazily initialize the session provider from Tempo wallet config.
    fn get_or_init(&self) -> TransportResult<SessionProvider> {
        let mut guard = self.inner.lock().unwrap();
        if let Some(ref provider) = *guard {
            return Ok(provider.clone());
        }

        let config = discover_mpp_config().ok_or_else(|| {
            TransportErrorKind::custom(std::io::Error::other(
                "RPC endpoint returned HTTP 402 Payment Required. \
                 This endpoint requires payment via the Machine Payments Protocol (MPP).\n\n\
                 To configure MPP, install the Tempo wallet CLI and create a key:\n\
                 \n  curl -sSL https://tempo.xyz/install.sh | bash\
                 \n  tempo wallet login\
                 \n\nSee https://docs.tempo.xyz for more information.",
            ))
        })?;

        let signer: mpp::PrivateKeySigner = config.key.parse().map_err(|e| {
            TransportErrorKind::custom(std::io::Error::other(format!("invalid MPP key: {e}")))
        })?;

        let signing_mode = if let Some(ref wallet_addr) = config.wallet_address {
            let wallet: alloy_primitives::Address = wallet_addr.parse().map_err(|e| {
                TransportErrorKind::custom(std::io::Error::other(format!(
                    "invalid MPP wallet address: {e}"
                )))
            })?;
            mpp::client::tempo::signing::TempoSigningMode::Keychain {
                wallet,
                key_authorization: None,
                version: mpp::client::tempo::signing::KeychainVersion::V2,
            }
        } else {
            mpp::client::tempo::signing::TempoSigningMode::Direct
        };

        let mut provider = SessionProvider::new(signer)
            .with_signing_mode(signing_mode)
            .with_default_deposit(100_000);

        if let Some(ref key_addr) = config.key_address
            && let Ok(addr) = key_addr.parse()
        {
            provider = provider.with_authorized_signer(addr);
        }

        *guard = Some(provider.clone());
        Ok(provider)
    }
}

/// HTTP transport with automatic MPP (Machine Payments Protocol) 402 handling.
///
/// Generic over the payment provider `P`. Works as a normal HTTP transport until
/// a 402 Payment Required response is received, then delegates payment to `P`.
///
/// Use [`LazyMppHttpTransport`] for production (lazy key discovery) or
/// `MppHttpTransport<YourProvider>` for testing with mock providers.
#[derive(Clone, Debug)]
pub struct MppHttpTransport<P> {
    client: reqwest::Client,
    url: Url,
    provider: P,
}

impl MppHttpTransport<LazySessionProvider> {
    /// Create a new lazy MPP transport that discovers keys on first 402.
    pub fn lazy(client: reqwest::Client, url: Url) -> Self {
        Self { client, url, provider: LazySessionProvider::new() }
    }
}

impl<P> MppHttpTransport<P> {
    /// Create a new MPP transport with an explicit payment provider.
    pub fn new(client: reqwest::Client, url: Url, provider: P) -> Self {
        Self { client, url, provider }
    }

    /// Returns a reference to the underlying reqwest client.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

#[allow(private_bounds)]
impl<P: ResolveProvider + Clone + Send + Sync + 'static> MppHttpTransport<P>
where
    P::Provider: Send + Sync + 'static,
{
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

        // If not 402, handle normally — no MPP overhead
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

        debug!(id = %challenge.id, method = %challenge.method, intent = %challenge.intent, "received MPP 402 challenge, paying");

        // Resolve the payment provider (lazy init for production, direct for tests)
        let resolved = self.provider.resolve()?;

        if !resolved.supports(challenge.method.as_str(), challenge.intent.as_str()) {
            return Err(TransportErrorKind::custom(std::io::Error::other(format!(
                "MPP challenge requires method={} intent={}, which is not supported",
                challenge.method, challenge.intent,
            ))));
        }

        // Pay the challenge
        let credential = resolved.pay(&challenge).await.map_err(|e| {
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

/// Trait for resolving a concrete `PaymentProvider` from a potentially lazy wrapper.
///
/// Direct providers (like `MockPaymentProvider` or `SessionProvider`) return
/// themselves. [`LazySessionProvider`] discovers keys and creates a
/// `SessionProvider` on first call.
pub(crate) trait ResolveProvider {
    type Provider: PaymentProvider;
    fn resolve(&self) -> TransportResult<Self::Provider>;
}

/// Any direct `PaymentProvider` resolves to itself.
impl<P: PaymentProvider + Clone> ResolveProvider for P {
    type Provider = P;
    fn resolve(&self) -> TransportResult<P> {
        Ok(self.clone())
    }
}

impl ResolveProvider for LazySessionProvider {
    type Provider = SessionProvider;
    fn resolve(&self) -> TransportResult<SessionProvider> {
        self.get_or_init()
    }
}

impl<P> fmt::Display for MppHttpTransport<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MppHttpTransport({})", self.url)
    }
}

#[allow(private_bounds)]
impl<P: ResolveProvider + Clone + Send + Sync + fmt::Debug + 'static> Service<RequestPacket>
    for MppHttpTransport<P>
where
    P::Provider: Send + Sync + 'static,
{
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        mpp::keys::discover_mpp_key, runtime_transport::RuntimeTransportBuilder,
    };
    use alloy_json_rpc::{Id, Request, RequestMeta};
    use axum::{
        extract::State, http::StatusCode as AxumStatusCode, response::IntoResponse, routing::post,
    };
    use mpp::{
        MppError,
        client::tempo::signing::{KeychainVersion, TempoSigningMode},
        protocol::core::{
            Base64UrlJson, PaymentChallenge, PaymentCredential, PaymentPayload,
            format_www_authenticate, parse_authorization,
        },
    };
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    /// Mock payment provider for unit tests.
    ///
    /// Returns a simple hash-type credential without making any on-chain calls,
    /// unlike [`mpp::client::TempoProvider`] which performs real gas estimation.
    #[derive(Clone, Debug)]
    struct MockPaymentProvider;

    impl PaymentProvider for MockPaymentProvider {
        fn supports(&self, method: &str, intent: &str) -> bool {
            method == "tempo" && intent == "charge"
        }

        async fn pay(&self, challenge: &PaymentChallenge) -> Result<PaymentCredential, MppError> {
            Ok(PaymentCredential::with_source(
                challenge.to_echo(),
                "did:pkh:eip155:42431:0xmockpayer",
                PaymentPayload::hash("0xmocktxhash"),
            ))
        }
    }

    /// Build a test challenge and its formatted WWW-Authenticate header.
    fn test_challenge() -> (mpp::PaymentChallenge, String) {
        let request = Base64UrlJson::from_value(&serde_json::json!({
            "amount": "1000",
            "currency": "0x20c0000000000000000000000000000000000000",
            "recipient": "0x742d35Cc6634C0532925a3b844Bc9e7595f1B0F2",
            "methodDetails": { "chainId": 42431 }
        }))
        .unwrap();
        let challenge =
            mpp::PaymentChallenge::new("test-id-42", "rpc.example.com", "tempo", "charge", request);
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
    async fn test_mpp_transport_non_402_passthrough() {
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
        let mut transport = MppHttpTransport::new(
            reqwest::Client::new(),
            Url::parse(&base_url).unwrap(),
            MockPaymentProvider,
        );

        let resp = tower::Service::call(&mut transport, test_request()).await.unwrap();

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
        let mut transport = MppHttpTransport::new(
            reqwest::Client::new(),
            Url::parse(&base_url).unwrap(),
            MockPaymentProvider,
        );

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
        let mut transport = MppHttpTransport::new(
            reqwest::Client::new(),
            Url::parse(&base_url).unwrap(),
            MockPaymentProvider,
        );

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
        // Server returns 402 without WWW-Authenticate header
        let app = axum::Router::new()
            .route("/", post(|| async { (AxumStatusCode::PAYMENT_REQUIRED, "pay up") }));

        let (base_url, handle) = spawn_server(app).await;
        let mut transport = MppHttpTransport::new(
            reqwest::Client::new(),
            Url::parse(&base_url).unwrap(),
            MockPaymentProvider,
        );

        let err = tower::Service::call(&mut transport, test_request()).await.unwrap_err();
        assert!(
            err.to_string().contains("WWW-Authenticate"),
            "expected WWW-Authenticate error, got: {err}"
        );

        handle.abort();
    }

    /// Verify that a 402 response on a lazy MPP transport (no MPP configured)
    /// produces an actionable error message with setup instructions.
    #[tokio::test]
    async fn test_plain_http_402_shows_mpp_setup_instructions() {
        let (_, www_auth) = test_challenge();

        let app = axum::Router::new().route(
            "/",
            post(move || {
                let www_auth = www_auth.clone();
                async move {
                    (
                        AxumStatusCode::PAYMENT_REQUIRED,
                        [("www-authenticate", www_auth)],
                        "Payment Required",
                    )
                }
            }),
        );

        let (base_url, handle) = spawn_server(app).await;

        // Ensure no MPP key is discovered so lazy init fails with instructions.
        unsafe {
            std::env::set_var("TEMPO_HOME", "/nonexistent/path");
            std::env::remove_var("TEMPO_PRIVATE_KEY");
        }

        let transport = RuntimeTransportBuilder::new(Url::parse(&base_url).unwrap()).build();
        let err = transport.request(test_request()).await.unwrap_err();
        let msg = err.to_string();

        assert!(
            msg.contains("402 Payment Required"),
            "expected 402 Payment Required in error, got: {msg}"
        );
        assert!(
            msg.contains("tempo wallet login"),
            "expected setup instructions in error, got: {msg}"
        );

        handle.abort();
        unsafe { std::env::remove_var("TEMPO_HOME") };
    }

    /// Verify that `rpc.mpp.tempo.xyz` returns a valid 402 MPP challenge.
    ///
    /// This test confirms the endpoint is 402-gated and returns a parseable
    /// `WWW-Authenticate` header. It does NOT complete the payment flow
    /// (the endpoint uses `session` intent which requires on-chain escrow).
    ///
    /// ```sh
    /// cargo test -p foundry-common test_mpp_live_402 -- --ignored
    /// ```
    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_mpp_live_402() {
        let client = reqwest::Client::new();
        let resp = client
            .post("https://rpc.mpp.tempo.xyz")
            .header("content-type", "application/json")
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}"#)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::PAYMENT_REQUIRED);

        let www_auth = resp
            .headers()
            .get(WWW_AUTHENTICATE_HEADER)
            .expect("missing WWW-Authenticate header")
            .to_str()
            .unwrap();

        let challenge = parse_www_authenticate(www_auth).unwrap();
        assert_eq!(challenge.realm, "rpc.mpp.tempo.xyz");
        assert_eq!(challenge.method.as_str(), "tempo");
    }

    /// End-to-end integration test: pay a real 402 challenge on `rpc.mpp.tempo.xyz`
    /// and get a valid JSON-RPC response.
    ///
    /// Requires a funded Tempo wallet with keychain access key configured in
    /// `~/.tempo/wallet/keys.toml` (managed by `tempo wallet`).
    ///
    /// ```sh
    /// cargo test -p foundry-common test_mpp_live_pay -- --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore = "requires network access and a funded Tempo wallet"]
    async fn test_mpp_live_pay() {
        let mpp_key = discover_mpp_key().expect(
            "no MPP key found; set TEMPO_PRIVATE_KEY or configure ~/.tempo/wallet/keys.toml",
        );

        let signer: mpp::PrivateKeySigner =
            mpp_key.parse().expect("failed to parse MPP key as PrivateKeySigner");

        // Read keys.toml to get wallet_address for keychain signing.
        // The key is already provisioned on-chain by `tempo wallet`, so we don't
        // need to pass key_authorization (it's only needed for first-time provisioning).
        let keys_path = dirs::home_dir().unwrap().join(".tempo/wallet/keys.toml");
        let keys_toml: toml::Value =
            toml::from_str(&std::fs::read_to_string(&keys_path).unwrap()).unwrap();

        let key_entry = keys_toml["keys"]
            .as_array()
            .and_then(|keys| keys.first())
            .expect("no key entries in keys.toml");

        let wallet_address: alloy_primitives::Address = key_entry["wallet_address"]
            .as_str()
            .expect("missing wallet_address")
            .parse()
            .expect("invalid wallet_address");

        let signer_address: alloy_primitives::Address = key_entry["key_address"]
            .as_str()
            .expect("missing key_address")
            .parse()
            .expect("invalid key_address");

        let signing_mode = TempoSigningMode::Keychain {
            wallet: wallet_address,
            key_authorization: None,
            version: KeychainVersion::V2,
        };

        let service_url = "https://rpc.mpp.tempo.xyz";
        let provider = super::super::session::SessionProvider::new(signer)
            .with_signing_mode(signing_mode)
            .with_authorized_signer(signer_address)
            .with_default_deposit(100_000);

        let mut transport = MppHttpTransport::new(
            reqwest::Client::new(),
            Url::parse(service_url).unwrap(),
            provider,
        );

        let resp = tower::Service::call(&mut transport, test_request()).await.unwrap();

        match resp {
            ResponsePacket::Single(r) => {
                assert!(r.is_success(), "expected successful JSON-RPC response, got: {r:?}");
                let _ = sh_eprintln!("got live MPP response: {r:?}");
            }
            _ => panic!("expected single response"),
        }
    }
}
