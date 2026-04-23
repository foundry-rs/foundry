//! MPP (Machine Payments Protocol) HTTP transport.
//!
//! Wraps a standard reqwest HTTP transport with automatic 402 Payment Required
//! handling via the MPP protocol. When the RPC endpoint returns a 402 response,
//! this transport automatically pays the challenge and retries the request.

use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_transport::{TransportError, TransportErrorKind, TransportFut, TransportResult};
use mpp::{
    client::PaymentProvider,
    protocol::core::{
        AUTHORIZATION_HEADER, WWW_AUTHENTICATE_HEADER, format_authorization,
        parse_www_authenticate_all,
    },
};
use reqwest::StatusCode;
use std::{
    collections::HashMap,
    fmt,
    sync::{Mutex, OnceLock},
    task,
    time::Duration,
};
use tokio::sync::OwnedMutexGuard;
use tower::Service;
use tracing::{Instrument, debug, debug_span, trace};
use url::Url;

use super::{
    keys::{DiscoverOptions, discover_mpp_config},
    session::SessionProvider,
};

/// Default deposit amount for new channels (in base units).
const DEFAULT_DEPOSIT: u128 = 100_000;

/// Timeout for MPP retry requests (open/topUp may wait for on-chain settlement).
const MPP_RETRY_TIMEOUT: Duration = Duration::from_secs(120);

/// Resolve the deposit amount from `MPP_DEPOSIT` env var or the default.
fn default_deposit() -> u128 {
    std::env::var("MPP_DEPOSIT").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_DEPOSIT)
}

/// Process-wide payment serialization locks, keyed by origin URL.
///
/// Created eagerly so the lock exists before the first provider init,
/// preventing concurrent first-402 races.
static GLOBAL_PAY_LOCKS: OnceLock<Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> =
    OnceLock::new();

/// Production transport: lazily discovers MPP keys from the Tempo wallet on
/// first 402 response.
pub type LazyMppHttpTransport = MppHttpTransport<LazySessionProvider>;

/// A payment provider that lazily initializes a [`SessionProvider`] from the
/// Tempo wallet configuration on first use.
#[derive(Clone, Debug)]
pub struct LazySessionProvider {
    inner: std::sync::Arc<Mutex<Option<SessionProvider>>>,
    /// Eagerly-created, process-wide payment serialization lock for this origin.
    pay_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    origin: String,
}

impl LazySessionProvider {
    pub(super) fn new(origin: String) -> Self {
        let pay_lock = {
            let global = GLOBAL_PAY_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
            global
                .lock()
                .unwrap()
                .entry(origin.clone())
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        Self { inner: std::sync::Arc::new(Mutex::new(None)), pay_lock, origin }
    }

    fn set_key_provisioned(&self, provisioned: bool) {
        if let Some(p) = self.inner.lock().unwrap().as_ref() {
            p.set_key_provisioned(provisioned);
        }
    }

    fn clear_channels(&self) {
        if let Some(p) = self.inner.lock().unwrap().as_ref() {
            p.clear_channels();
        }
    }

    pub(super) fn flush_pending(&self) {
        if let Some(p) = self.inner.lock().unwrap().as_ref() {
            p.flush_pending();
        }
    }

    pub(super) fn rollback_pending(&self) {
        if let Some(p) = self.inner.lock().unwrap().as_ref() {
            p.rollback_pending();
        }
    }

    fn commit_topup_and_track_voucher(&self) {
        if let Some(p) = self.inner.lock().unwrap().as_ref() {
            p.commit_topup_and_track_voucher();
        }
    }

    pub(super) fn get_or_init(&self, opts: DiscoverOptions) -> TransportResult<SessionProvider> {
        let mut guard = self.inner.lock().unwrap();
        if let Some(ref provider) = *guard {
            return Ok(provider.clone());
        }

        let config = discover_mpp_config(opts).ok_or_else(|| {
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

        let signing_mode = if let Some(wallet) = config.wallet_address {
            let key_authorization = config
                .key_authorization
                .as_ref()
                .map(|hex_str| {
                    crate::tempo::decode_key_authorization(hex_str).map(Box::new).map_err(|e| {
                        TransportErrorKind::custom(std::io::Error::other(format!(
                            "invalid MPP key_authorization: {e}"
                        )))
                    })
                })
                .transpose()?;

            mpp::client::tempo::signing::TempoSigningMode::Keychain {
                wallet,
                key_authorization,
                version: mpp::client::tempo::signing::KeychainVersion::V2,
            }
        } else {
            mpp::client::tempo::signing::TempoSigningMode::Direct
        };

        let mut provider = SessionProvider::new(signer, self.origin.clone())
            .with_signing_mode(signing_mode)
            .with_default_deposit(default_deposit())
            .with_key_filters(config.chain_id, config.currencies);

        if let Some(addr) = config.key_address {
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
#[derive(Clone, Debug)]
pub struct MppHttpTransport<P> {
    client: reqwest::Client,
    url: Url,
    provider: P,
}

impl MppHttpTransport<LazySessionProvider> {
    /// Create a new lazy MPP transport that discovers keys on first 402.
    ///
    /// Uses the provided `client` for all requests. Per-request timeouts are
    /// extended on retry requests that involve on-chain settlement (channel
    /// open/topUp).
    pub fn lazy(client: reqwest::Client, url: Url) -> Self {
        let origin = url.to_string();
        Self { client, url, provider: LazySessionProvider::new(origin) }
    }
}

impl<P> MppHttpTransport<P> {
    /// Create a new MPP transport with an explicit payment provider.
    pub const fn new(client: reqwest::Client, url: Url, provider: P) -> Self {
        Self { client, url, provider }
    }

    /// Returns a reference to the underlying reqwest client.
    pub const fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

#[allow(private_bounds)]
impl<P: ResolveProvider + Clone + Send + Sync + 'static> MppHttpTransport<P>
where
    P::Provider: Send + Sync + 'static,
{
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

        if resp.status() != StatusCode::PAYMENT_REQUIRED {
            return Self::handle_response(resp).await;
        }

        // Serialize the entire 402 → pay → retry → response cycle.
        // This prevents concurrent requests from opening duplicate channels
        // or producing colliding expiring-nonce transactions. The lock is
        // held until the retry response is fully handled.
        let _pay_guard = self.provider.lock_pay().await;

        let (resolved, challenge) = Self::select_challenge(&resp, &self.provider)?;

        debug!(id = %challenge.id, method = %challenge.method, intent = %challenge.intent, "received MPP 402 challenge, paying");

        let credential = resolved.pay(&challenge).await.map_err(|e| {
            TransportErrorKind::custom(std::io::Error::other(format!("MPP payment failed: {e}")))
        })?;

        let auth_header = format_authorization(&credential).map_err(|e| {
            TransportErrorKind::custom(std::io::Error::other(format!(
                "failed to format MPP credential: {e}"
            )))
        })?;

        // Use a longer per-request timeout because the server may need to
        // settle an on-chain transaction (channel open/topUp) before responding.
        let retry_resp = self
            .client
            .post(self.url.clone())
            .timeout(MPP_RETRY_TIMEOUT)
            .headers(headers.clone())
            .header("content-type", "application/json")
            .header(AUTHORIZATION_HEADER, &auth_header)
            .body(body.clone())
            .send()
            .await
            .map_err(|e| {
                self.provider.rollback_pending();
                TransportErrorKind::custom(e)
            })?;

        // 204 No Content → topUp accepted, re-pay with voucher
        if retry_resp.status() == StatusCode::NO_CONTENT {
            debug!("MPP topUp accepted (204), retrying with voucher");

            // Top-up is confirmed — commit the deposit increase and start
            // tracking the follow-up voucher cumulative bump separately.
            self.provider.commit_topup_and_track_voucher();

            let resolved = self.provider.resolve()?;
            let voucher_resp = self.pay_and_retry(&challenge, &resolved, &headers, &body).await?;

            let result = Self::handle_response(voucher_resp).await;
            if result.is_ok() {
                self.provider.set_key_provisioned(true);
                self.provider.flush_pending();
            } else {
                self.provider.rollback_pending();
            }
            return result;
        }

        // 410 Gone → channel stale
        if retry_resp.status() == StatusCode::GONE {
            debug!("MPP channel not found (410), clearing stale local state");
            self.provider.rollback_pending();
            self.provider.clear_channels();

            return Err(TransportErrorKind::custom(std::io::Error::other(
                "MPP channel not found on server (410 Gone). \
                 The server may have restarted or the channel was closed externally.\n\
                 Local channel state has been cleared. Re-run to open a new channel.",
            )));
        }

        // Retry 402 → handle specific recoverable errors before giving up.
        if retry_resp.status() == StatusCode::PAYMENT_REQUIRED {
            let retry_body = retry_resp.bytes().await.map_err(TransportErrorKind::custom)?;
            let retry_text = String::from_utf8_lossy(&retry_body);

            // Parse RFC 9457 Problem Details if present. The `type` URI is the
            // structured error code; the `detail` string provides context.
            let problem: Option<mpp::error::PaymentErrorDetails> =
                serde_json::from_slice(&retry_body).ok();
            let problem_type = problem.as_ref().map(|p| p.problem_type.as_str()).unwrap_or("");
            let detail = problem.as_ref().map(|p| p.detail.as_str()).unwrap_or("");

            // Stale voucher: another provider instance (or a previous process)
            // already used a higher cumulative_amount. Re-pay with a fresh
            // voucher whose amount will be strictly greater.
            let is_stale_voucher = problem_type.ends_with("/stale-voucher")
                || detail.contains("cumulativeAmount must be strictly greater");
            if is_stale_voucher {
                debug!("MPP voucher stale, retrying with fresh voucher");
                let resolved = self.provider.resolve()?;
                if resolved.supports(challenge.method.as_str(), challenge.intent.as_str()) {
                    let final_resp =
                        self.pay_and_retry(&challenge, &resolved, &headers, &body).await?;

                    let result = Self::handle_response(final_resp).await;
                    if result.is_ok() {
                        self.provider.flush_pending();
                    } else {
                        self.provider.rollback_pending();
                    }
                    return result;
                }
            }

            // Retry with key_authorization when the error explicitly indicates
            // the access key is not provisioned on-chain, or when verification
            // failed and the key appears provisioned (first-time provisioning
            // where key_auth was stripped but not yet provisioned on-chain).
            //
            // We fetch a fresh challenge because the server may have consumed
            // the original challenge ID on first use.
            let needs_key_provisioning = problem_type.ends_with("/key-not-provisioned")
                || detail.contains("access key does not exist")
                || detail.contains("key is not provisioned");

            let needs_verification_retry = (problem_type.ends_with("/verification-failed")
                || detail.contains("verification-failed"))
                && self.provider.is_key_provisioned();

            if needs_key_provisioning || needs_verification_retry {
                debug!(
                    problem_type,
                    "MPP 402 key not provisioned/verification-failed, retrying with key_authorization"
                );
                self.provider.set_key_provisioned(false);
                self.provider.rollback_pending();

                let (resolved, fresh_challenge) =
                    self.fetch_fresh_challenge(&headers, &body).await?;

                let final_resp =
                    self.pay_and_retry(&fresh_challenge, &resolved, &headers, &body).await?;

                let result = Self::handle_response(final_resp).await;
                if result.is_ok() {
                    self.provider.set_key_provisioned(true);
                    self.provider.flush_pending();
                } else {
                    self.provider.rollback_pending();
                }
                return result;
            }

            self.provider.rollback_pending();
            return Err(TransportErrorKind::http_error(
                StatusCode::PAYMENT_REQUIRED.as_u16(),
                retry_text.into_owned(),
            ));
        }

        let result = Self::handle_response(retry_resp).await;
        if result.is_ok() {
            self.provider.set_key_provisioned(true);
            self.provider.flush_pending();
        } else {
            self.provider.rollback_pending();
        }
        result
    }

    /// Pay a challenge and send the authenticated retry request.
    async fn pay_and_retry(
        &self,
        challenge: &mpp::protocol::core::PaymentChallenge,
        provider: &P::Provider,
        headers: &reqwest::header::HeaderMap,
        body: &[u8],
    ) -> TransportResult<reqwest::Response> {
        let credential = provider.pay(challenge).await.map_err(|e| {
            self.provider.rollback_pending();
            TransportErrorKind::custom(std::io::Error::other(format!("MPP payment failed: {e}")))
        })?;

        let auth_header = format_authorization(&credential).map_err(|e| {
            self.provider.rollback_pending();
            TransportErrorKind::custom(std::io::Error::other(format!(
                "failed to format MPP credential: {e}"
            )))
        })?;

        self.client
            .post(self.url.clone())
            .timeout(MPP_RETRY_TIMEOUT)
            .headers(headers.clone())
            .header("content-type", "application/json")
            .header(AUTHORIZATION_HEADER, auth_header)
            .body(body.to_vec())
            .send()
            .await
            .map_err(|e| {
                self.provider.rollback_pending();
                TransportErrorKind::custom(e)
            })
    }

    /// Fetch a fresh 402 challenge from the server (unauthenticated request).
    ///
    /// Returns `Ok(Some((provider, challenge)))` if the server returns a 402
    /// with a matching challenge. Returns `Ok(None)` with the response handled
    /// if the server returns a non-402 status. Errors on network or parse failures.
    async fn fetch_fresh_challenge(
        &self,
        headers: &reqwest::header::HeaderMap,
        body: &[u8],
    ) -> TransportResult<(P::Provider, mpp::protocol::core::PaymentChallenge)> {
        let fresh_resp = self
            .client
            .post(self.url.clone())
            .timeout(MPP_RETRY_TIMEOUT)
            .headers(headers.clone())
            .header("content-type", "application/json")
            .body(body.to_vec())
            .send()
            .await
            .map_err(TransportErrorKind::custom)?;

        if fresh_resp.status() != StatusCode::PAYMENT_REQUIRED {
            // Non-402 → return whatever the server sent (could be success or error).
            let result = Self::handle_response(fresh_resp).await;
            return Err(result.err().unwrap_or_else(|| {
                TransportErrorKind::custom(std::io::Error::other(
                    "unexpected success on unauthenticated fresh probe",
                ))
            }));
        }

        Self::select_challenge(&fresh_resp, &self.provider)
    }

    /// Parse `WWW-Authenticate` challenges from a 402 response and resolve
    /// the first one matching a locally configured key (chain + currency).
    fn select_challenge(
        resp: &reqwest::Response,
        provider: &P,
    ) -> TransportResult<(P::Provider, mpp::protocol::core::PaymentChallenge)> {
        let www_auth_values: Vec<&str> = resp
            .headers()
            .get_all(WWW_AUTHENTICATE_HEADER)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();

        if www_auth_values.is_empty() {
            return Err(TransportErrorKind::custom(std::io::Error::other(
                "402 response missing WWW-Authenticate header",
            )));
        }

        let challenges: Vec<_> = parse_www_authenticate_all(www_auth_values)
            .into_iter()
            .filter_map(|r| r.ok())
            .collect();

        let mut last_resolve_err: Option<TransportError> = None;
        let resolved_pair = challenges.iter().find_map(|c| {
            let (chain_id, currency) = extract_challenge_chain_and_currency(c);
            let currency = currency.and_then(|s| s.parse().ok());
            match provider.resolve_for(DiscoverOptions { chain_id, currency }) {
                Ok(p) => p.supports(c.method.as_str(), c.intent.as_str()).then_some((p, c.clone())),
                Err(e) => {
                    last_resolve_err = Some(e);
                    None
                }
            }
        });

        resolved_pair.ok_or_else(|| {
            if let Some(err) = last_resolve_err {
                return err;
            }
            let offered: Vec<_> =
                challenges.iter().map(|c| format!("{}.{}", c.method, c.intent)).collect();
            TransportErrorKind::custom(std::io::Error::other(format!(
                "no supported MPP challenge; server offered [{}]",
                offered.join(", "),
            )))
        })
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

/// Extract `(chainId, currency)` from a parsed MPP challenge.
pub(super) fn extract_challenge_chain_and_currency(
    c: &mpp::protocol::core::PaymentChallenge,
) -> (Option<u64>, Option<String>) {
    if c.method.as_str() == "tempo" {
        let val = c.request.decode_value().ok();
        let chain_id = val.as_ref().and_then(|v| v.get("methodDetails")?.get("chainId")?.as_u64());
        let currency = val.as_ref().and_then(|v| v.get("currency")?.as_str().map(String::from));
        (chain_id, currency)
    } else {
        (None, None)
    }
}

/// Trait for resolving a concrete `PaymentProvider` from a potentially lazy wrapper.
pub(crate) trait ResolveProvider {
    type Provider: PaymentProvider;
    fn resolve(&self) -> TransportResult<Self::Provider> {
        self.resolve_for(Default::default())
    }
    fn resolve_for(&self, opts: DiscoverOptions) -> TransportResult<Self::Provider>;
    fn set_key_provisioned(&self, _provisioned: bool) {}
    fn is_key_provisioned(&self) -> bool {
        true
    }
    fn clear_channels(&self) {}
    fn flush_pending(&self) {}
    fn rollback_pending(&self) {}
    fn commit_topup_and_track_voucher(&self) {}
    /// Acquire the payment serialization lock. The returned guard must be held
    /// across the entire 402 → pay → retry → response cycle to prevent
    /// concurrent channel opens and colliding expiring-nonce transactions.
    fn lock_pay(&self) -> impl std::future::Future<Output = Option<OwnedMutexGuard<()>>> + Send {
        async { None }
    }
}

impl<P: PaymentProvider + Clone> ResolveProvider for P {
    type Provider = P;
    fn resolve_for(&self, _opts: DiscoverOptions) -> TransportResult<P> {
        Ok(self.clone())
    }
}

impl ResolveProvider for LazySessionProvider {
    type Provider = SessionProvider;
    fn resolve_for(&self, opts: DiscoverOptions) -> TransportResult<SessionProvider> {
        let provider = self.get_or_init(opts.clone())?;
        // After the first init, get_or_init returns the cached provider
        // regardless of opts. Re-check that the provider's key is compatible
        // with this challenge's chain/currency.
        if !provider.matches_challenge(opts.chain_id, opts.currency) {
            return Err(TransportErrorKind::custom(std::io::Error::other(
                "cached provider does not match challenge chain/currency",
            )));
        }
        Ok(provider)
    }
    fn set_key_provisioned(&self, provisioned: bool) {
        Self::set_key_provisioned(self, provisioned)
    }
    fn is_key_provisioned(&self) -> bool {
        self.inner.lock().unwrap().as_ref().is_none_or(|p| p.is_key_provisioned())
    }
    fn clear_channels(&self) {
        Self::clear_channels(self)
    }
    fn flush_pending(&self) {
        Self::flush_pending(self)
    }
    fn rollback_pending(&self) {
        Self::rollback_pending(self)
    }
    fn commit_topup_and_track_voucher(&self) {
        Self::commit_topup_and_track_voucher(self)
    }
    fn lock_pay(&self) -> impl std::future::Future<Output = Option<OwnedMutexGuard<()>>> + Send {
        let lock = self.pay_lock.clone();
        async move { Some(lock.lock_owned().await) }
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
    use crate::provider::runtime_transport::RuntimeTransportBuilder;
    use alloy_json_rpc::{Id, Request, RequestMeta};
    use axum::{
        extract::State, http::StatusCode as AxumStatusCode, response::IntoResponse, routing::post,
    };
    use mpp::{
        MppError,
        protocol::core::{
            Base64UrlJson, IntentName, MethodName, PaymentChallenge, PaymentCredential,
            format_www_authenticate, parse_authorization,
        },
    };

    #[derive(Clone, Debug)]
    struct MockPaymentProvider;

    impl PaymentProvider for MockPaymentProvider {
        fn supports(&self, method: &str, intent: &str) -> bool {
            method == "tempo" && (intent == "session" || intent == "charge")
        }

        fn pay(
            &self,
            challenge: &PaymentChallenge,
        ) -> impl std::future::Future<Output = Result<PaymentCredential, MppError>> + Send {
            let echo = challenge.to_echo();
            async move {
                Ok(PaymentCredential::with_source(
                    echo,
                    "test-source".to_string(),
                    serde_json::json!({"action": "voucher", "channelId": "0xtest", "cumulativeAmount": "1000", "signature": "0xtest"}),
                ))
            }
        }
    }

    fn test_challenge() -> (PaymentChallenge, String) {
        let request = Base64UrlJson::from_value(&serde_json::json!({
            "amount": "1000",
            "currency": "0x20c0",
            "recipient": "0xpayee",
            "methodDetails": {
                "chainId": 42431
            }
        }))
        .unwrap();

        let challenge = PaymentChallenge {
            id: "test-id-42".to_string(),
            realm: "test-realm".to_string(),
            method: MethodName::new("tempo"),
            intent: IntentName::new("session"),
            request,
            expires: None,
            description: None,
            digest: None,
            opaque: None,
        };

        let www_auth = format_www_authenticate(&challenge).unwrap();
        (challenge, www_auth)
    }

    fn test_request() -> RequestPacket {
        let req: Request<serde_json::Value> = Request {
            meta: RequestMeta::new("eth_blockNumber".into(), Id::Number(1)),
            params: serde_json::Value::Array(vec![]),
        };
        RequestPacket::Single(req.serialize().unwrap())
    }

    async fn spawn_server(app: axum::Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn test_mpp_transport_no_402() {
        let app = axum::Router::new().route(
            "/",
            post(|| async {
                axum::Json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": "0x123"
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
            ResponsePacket::Single(r) => assert!(r.is_success()),
            _ => panic!("expected single response"),
        }

        handle.abort();
    }

    #[tokio::test]
    async fn test_mpp_transport_402_then_success() {
        let (_, www_auth) = test_challenge();
        let state = AppState { www_auth };

        #[derive(Clone)]
        struct AppState {
            www_auth: String,
        }

        let app =
            axum::Router::new()
                .route(
                    "/",
                    post(
                        |State(state): State<AppState>,
                         req: axum::http::Request<axum::body::Body>| async move {
                            if let Some(auth) = req.headers().get("authorization") {
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

        let resp = tower::Service::call(&mut transport, test_request()).await.unwrap();
        match resp {
            ResponsePacket::Single(r) => assert!(r.is_success()),
            _ => panic!("expected single response"),
        }

        handle.abort();
    }

    #[tokio::test]
    async fn test_mpp_transport_402_missing_www_authenticate() {
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

        unsafe {
            std::env::set_var("TEMPO_HOME", "/nonexistent/path");
            std::env::remove_var("TEMPO_PRIVATE_KEY");
        }

        let transport = RuntimeTransportBuilder::new(Url::parse(&base_url).unwrap()).build();
        let err = transport.request(test_request()).await.unwrap_err();
        let msg = err.to_string();

        assert!(
            msg.contains("402 Payment Required") || msg.contains("no supported MPP challenge"),
            "expected MPP setup instructions or 'no supported MPP challenge' in error, got: {msg}"
        );

        handle.abort();
        unsafe { std::env::remove_var("TEMPO_HOME") };
    }

    #[test]
    fn test_session_provider_supports_charge_and_session() {
        let signer = mpp::PrivateKeySigner::random();
        let provider =
            super::super::session::SessionProvider::new(signer, "https://rpc.example.com".into());

        assert!(provider.supports("tempo", "session"));
        assert!(provider.supports("tempo", "charge"));
        assert!(!provider.supports("stripe", "charge"));
        assert!(!provider.supports("tempo", "subscribe"));
    }

    #[tokio::test]
    async fn test_session_provider_pay_charge_parses_challenge() {
        let signer = mpp::PrivateKeySigner::random();
        let provider =
            super::super::session::SessionProvider::new(signer, "https://rpc.example.com".into());

        // Valid charge challenge — pay_charge wires through to TempoCharge,
        // which will fail at gas estimation (no RPC), but confirms the path is connected.
        let (challenge, _) = test_challenge();
        let err = provider.pay(&challenge).await.unwrap_err();
        // Should fail deeper than "not supported" — proves charge dispatch works
        assert!(
            !err.to_string().contains("not supported"),
            "expected charge path to be wired up, got: {err}"
        );
    }

    #[test]
    fn challenge_chain_and_currency_extraction() {
        let extract = |headers: Vec<&str>| -> Vec<(Option<u64>, Option<String>)> {
            let challenges: Vec<_> =
                parse_www_authenticate_all(headers).into_iter().filter_map(|r| r.ok()).collect();
            challenges.iter().map(extract_challenge_chain_and_currency).collect()
        };

        let b64 = |v: serde_json::Value| -> String {
            Base64UrlJson::from_value(&v).unwrap().raw().to_string()
        };

        // Tempo challenge with chainId + currency
        let tempo_header = format!(
            r#"Payment id="abc", realm="api", method="tempo", intent="charge", request="{}""#,
            b64(
                serde_json::json!({"amount":"1000","currency":"0x20c0","methodDetails":{"chainId":42431},"recipient":"0xabc"})
            )
        );
        assert_eq!(extract(vec![&tempo_header]), vec![(Some(42431), Some("0x20c0".into()))]);

        // Non-tempo challenge → (None, None)
        let stripe_header = format!(
            r#"Payment id="xyz", realm="api", method="stripe", intent="charge", request="{}""#,
            b64(serde_json::json!({"amount":"100"}))
        );
        assert_eq!(extract(vec![&stripe_header]), vec![(None, None)]);

        // Tempo challenge without methodDetails → chainId None, currency present
        let no_details = format!(
            r#"Payment id="def", realm="api", method="tempo", intent="charge", request="{}""#,
            b64(serde_json::json!({"amount":"1000","currency":"0x20c0","recipient":"0xabc"}))
        );
        assert_eq!(extract(vec![&no_details]), vec![(None, Some("0x20c0".into()))]);
    }
}
