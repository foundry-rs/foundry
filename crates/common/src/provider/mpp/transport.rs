//! MPP (Machine Payments Protocol) HTTP transport.
//!
//! Wraps a standard reqwest HTTP transport with automatic 402 Payment Required
//! handling via the MPP protocol. When the RPC endpoint returns a 402 response,
//! this transport automatically pays the challenge and retries the request.

use alloy_chains::Chain;
use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_transport::{TransportError, TransportErrorKind, TransportFut, TransportResult};
use mpp::{
    client::{PaymentProvider, TempoAccountsProvider},
    protocol::core::{
        AUTHORIZATION_HEADER, WWW_AUTHENTICATE_HEADER, format_authorization,
        parse_www_authenticate_all,
    },
};
use reqwest::{StatusCode, header::HeaderMap};
use std::{
    collections::HashMap,
    env, fmt, io,
    io::IsTerminal,
    process::{Command, Stdio},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tower::Service;
use tracing::{Instrument, debug, debug_span, trace};
use url::Url;

use tempo_alloy::accounts::TempoAccountsStore;

/// Timeout for MPP retry requests that may wait for on-chain settlement.
const MPP_RETRY_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Default)]
pub(crate) struct FundingContext {
    wallet_address: Option<alloy_primitives::Address>,
    token: Option<String>,
    chain_id: Option<Chain>,
}

impl FundingContext {
    fn token_line(&self) -> String {
        self.token
            .as_ref()
            .map(|token| format!("Requested payment token: {token}\n\n"))
            .unwrap_or_default()
    }

    fn network(&self) -> Option<String> {
        self.chain_id.filter(|chain| chain.is_tempo()).map(|chain| chain.to_string())
    }
}

fn format_http_diagnostics(headers: &HeaderMap) -> String {
    const DIAGNOSTIC_HEADERS: &[&str] = &["x-request-id", "cf-ray", "server", "report-to", "nel"];

    let pairs: Vec<String> = DIAGNOSTIC_HEADERS
        .iter()
        .filter_map(|name| {
            headers.get(*name).and_then(|value| value.to_str().ok().map(|v| (*name, v)))
        })
        .map(|(name, value)| format!("{name}: {value}"))
        .collect();

    if pairs.is_empty() {
        String::new()
    } else {
        format!("\n\nHTTP diagnostics:\n{}", pairs.join("\n"))
    }
}

fn tempo_wallet_fund_help(ctx: &FundingContext) -> String {
    let mut command = "tempo wallet fund".to_string();
    if let Some(address) = ctx.wallet_address {
        command.push_str(&format!(" --address {address}"));
    }
    if let Some(network) = ctx.network() {
        command.push_str(&format!(" --network {network}"));
    }

    let mut no_browser = command.clone();
    no_browser.push_str(" --no-browser");

    format!(
        "\n\nTempo wallet payment could not be funded for this paid RPC request.\n\n{}\
         Fund the wallet, then rerun the command:\n  {command}\n\n\
         If this CLI is running on a remote or headless host, use:\n  {no_browser}",
        ctx.token_line()
    )
}

/// Decide whether the interactive `tempo wallet fund` flow may be launched.
///
/// Policy (library-safe):
/// - never run inside CI
/// - never run unless both stdin and stderr are real terminals
/// - `FOUNDRY_MPP_NO_AUTO_FUND` is honored as an opt-out; it must not bypass CI/TTY guards in
///   shared transport code that may be embedded inside long-running RPC daemons.
fn interactive_tempo_fund_allowed(
    no_auto_fund: Option<&str>,
    in_ci: bool,
    stdin_is_terminal: bool,
    stderr_is_terminal: bool,
) -> bool {
    if no_auto_fund.is_some_and(|v| {
        !(v == "0" || v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("off"))
    }) {
        return false;
    }

    if in_ci {
        return false;
    }

    stdin_is_terminal && stderr_is_terminal
}

fn can_run_interactive_tempo_fund() -> bool {
    if cfg!(test) {
        return false;
    }

    interactive_tempo_fund_allowed(
        std::env::var("FOUNDRY_MPP_NO_AUTO_FUND").ok().as_deref(),
        std::env::var_os("CI").is_some(),
        std::io::stdin().is_terminal(),
        std::io::stderr().is_terminal(),
    )
}

fn tempo_bin() -> String {
    std::env::var("TEMPO_BIN").unwrap_or_else(|_| "tempo".to_string())
}

async fn run_interactive_tempo_fund(ctx: &FundingContext) -> TransportResult<bool> {
    if !can_run_interactive_tempo_fund() {
        return Ok(false);
    }

    let tempo = tempo_bin();
    let mut args = vec!["wallet".to_string(), "fund".to_string()];
    if let Some(address) = ctx.wallet_address {
        args.push("--address".to_string());
        args.push(address.to_string());
    }
    if let Some(network) = ctx.network() {
        args.push("--network".to_string());
        args.push(network);
    }

    tracing::warn!(
        token = ?ctx.token,
        chain_id = ?ctx.chain_id,
        "MPP payment could not be funded; opening `tempo wallet fund`"
    );

    let status = tokio::task::spawn_blocking(move || {
        Command::new(tempo)
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
    })
    .await
    .map_err(|e| {
        TransportErrorKind::custom(std::io::Error::other(format!(
            "failed to join tempo wallet fund process: {e}"
        )))
    })?
    .map_err(|e| {
        TransportErrorKind::custom(std::io::Error::other(format!(
            "failed to run `tempo wallet fund`: {e}{}",
            tempo_wallet_fund_help(ctx)
        )))
    })?;

    if status.success() {
        Ok(true)
    } else {
        Err(TransportErrorKind::custom(std::io::Error::other(format!(
            "`tempo wallet fund` exited with status {status}{}",
            tempo_wallet_fund_help(ctx)
        ))))
    }
}

/// Single-attempt guard around [`run_interactive_tempo_fund`].
///
/// Ensures that for one logical request we launch `tempo wallet fund` at most
/// once, regardless of how many recovery paths (`do_request`, `pay_and_retry`,
/// `handle_response_or_retry_after_fund`, ...) attempt it.
async fn maybe_auto_fund(used: &AtomicBool, ctx: &FundingContext) -> TransportResult<bool> {
    if !can_run_interactive_tempo_fund() {
        return Ok(false);
    }
    if used.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Ok(false);
    }
    run_interactive_tempo_fund(ctx).await
}

/// Returns true iff a 402 response carries a structured insufficient-balance
/// problem (RFC 9457 `PaymentErrorDetails`).
///
/// We deliberately do **not** match on free-text body content or on generic
/// `verification-failed` problem types, as those have many non-funding causes
/// (bad signature, replay, expired challenge, clock skew, key provisioning,
/// malformed auth, ...).
fn should_suggest_tempo_fund(status: StatusCode, body: &[u8]) -> bool {
    if status != StatusCode::PAYMENT_REQUIRED {
        return false;
    }
    let Ok(problem) = serde_json::from_slice::<mpp::error::PaymentErrorDetails>(body) else {
        return false;
    };
    problem.problem_type.ends_with("/insufficient-balance")
}

fn format_mpp_payment_failure(
    error: impl fmt::Display,
    ctx: &FundingContext,
    suggest_fund: bool,
) -> String {
    let message = error.to_string();
    if suggest_fund {
        format!("MPP payment failed: {message}{}", tempo_wallet_fund_help(ctx))
    } else {
        format!("MPP payment failed: {message}")
    }
}

/// Process-wide payment serialization locks, keyed by origin URL.
///
/// Created eagerly so the lock exists before the first provider init,
/// preventing concurrent first-402 races.
static GLOBAL_PAY_LOCKS: LazyLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Production transport: lazily opens the Tempo Accounts wallet on
/// first 402 response.
pub type LazyMppHttpTransport = MppHttpTransport<LazyAccountsProvider>;

/// A Charge provider that lazily initializes from the Tempo Accounts store.
#[derive(Clone)]
pub struct LazyAccountsProvider {
    inner: Arc<Mutex<HashMap<Option<u64>, TempoAccountsProvider>>>,
    /// Eagerly-created, process-wide payment serialization lock for this origin.
    pay_lock: Arc<AsyncMutex<()>>,
    origin: String,
}

impl fmt::Debug for LazyAccountsProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LazyAccountsProvider")
            .field("origin", &self.origin)
            .field("initialized_chains", &self.inner.lock().unwrap().keys())
            .finish()
    }
}

impl LazyAccountsProvider {
    pub(super) fn new(origin: String) -> Self {
        let pay_lock = GLOBAL_PAY_LOCKS
            .lock()
            .unwrap()
            .entry(origin.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        Self { inner: Arc::new(Mutex::new(HashMap::new())), pay_lock, origin }
    }

    /// Drop cached providers after the device-code flow updates `store.json`.
    fn invalidate(&self) {
        self.inner.lock().unwrap().clear();
    }

    pub(super) fn get_or_init(
        &self,
        chain_id: Option<u64>,
    ) -> TransportResult<TempoAccountsProvider> {
        let mut guard = self.inner.lock().unwrap();
        if let Some(provider) = guard.get(&chain_id) {
            return Ok(provider.clone());
        }

        let mut provider = TempoAccountsProvider::from_default_store().map_err(|error| {
            TransportErrorKind::custom(io::Error::other(format!(
                "RPC endpoint returned HTTP 402 Payment Required, but the Tempo Accounts \
                     store could not provide a Charge wallet: {error}\n\n\
                     Authorize an access key with:\n  cast tempo login\n\n\
                     In a headless environment, use:\n  cast tempo login --no-browser"
            )))
        })?;
        if let Some(chain_id) = chain_id {
            provider = provider.with_expected_chain_id(chain_id);
        }
        guard.insert(chain_id, provider.clone());
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

impl MppHttpTransport<LazyAccountsProvider> {
    /// Create a new transport that opens the Tempo Accounts store on first 402.
    ///
    /// Uses the provided `client` for all requests. Per-request timeouts are
    /// extended on retries that may wait for on-chain settlement.
    pub fn lazy(client: reqwest::Client, url: Url) -> Self {
        let origin = url.to_string();
        Self { client, url, provider: LazyAccountsProvider::new(origin) }
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
        // Per-request guard: launch `tempo wallet fund` at most once for one
        // logical request, regardless of how many recovery paths attempt it.
        let auto_fund_used = AtomicBool::new(false);
        self.do_request_inner(req, &auto_fund_used).await
    }

    async fn do_request_inner(
        self,
        req: RequestPacket,
        auto_fund_used: &AtomicBool,
    ) -> TransportResult<ResponsePacket> {
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

        // No local key for any offered challenge → run device-code flow,
        // invalidate the cached provider, and fetch a fresh 402 (the original
        // may have expired during the browser/passkey flow).
        let (resolved, challenge) =
            if let Some(chain_id) = tempo_chain_needing_auth(&self.url, &resp) {
                debug!(chain_id, "launching wallet.tempo authorization");
                let cfg = crate::tempo::EnsureAccessKeyConfig::from_env(chain_id);
                crate::tempo::ensure_access_key(cfg).await.map_err(|e| {
                    TransportErrorKind::custom(io::Error::other(format!(
                        "tempo access key authorization failed: {e}"
                    )))
                })?;
                self.provider.invalidate_cached_provider();
                self.fetch_fresh_challenge(&headers, &body).await?
            } else {
                Self::select_challenge(&resp, &self.provider)?
            };
        let funding_ctx = self.provider.funding_context(&challenge);

        debug!(id = %challenge.id, method = %challenge.method, intent = %challenge.intent, "received MPP 402 challenge, paying");

        let retry_resp =
            self.pay_and_retry(&challenge, &resolved, &headers, &body, auto_fund_used).await?;
        self.handle_response_or_retry_after_fund(
            retry_resp,
            &headers,
            &body,
            &funding_ctx,
            auto_fund_used,
        )
        .await
    }

    /// Pay a challenge and send the authenticated retry request.
    async fn pay_and_retry(
        &self,
        challenge: &mpp::protocol::core::PaymentChallenge,
        provider: &P::Provider,
        headers: &reqwest::header::HeaderMap,
        body: &[u8],
        auto_fund_used: &AtomicBool,
    ) -> TransportResult<reqwest::Response> {
        let funding_ctx = self.provider.funding_context(challenge);
        let credential = match provider.pay(challenge).await {
            Ok(credential) => credential,
            Err(e) => {
                let is_insufficient = matches!(e, mpp::MppError::InsufficientBalance(_));
                if is_insufficient && maybe_auto_fund(auto_fund_used, &funding_ctx).await? {
                    provider.pay(challenge).await.map_err(|e2| {
                        let suggest = matches!(e2, mpp::MppError::InsufficientBalance(_));
                        TransportErrorKind::custom(std::io::Error::other(
                            format_mpp_payment_failure(e2, &funding_ctx, suggest),
                        ))
                    })?
                } else {
                    return Err(TransportErrorKind::custom(std::io::Error::other(
                        format_mpp_payment_failure(e, &funding_ctx, is_insufficient),
                    )));
                }
            }
        };

        let auth_header = format_authorization(&credential).map_err(|e| {
            TransportErrorKind::custom(io::Error::other(format!(
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
            .map_err(TransportErrorKind::custom)
    }

    async fn handle_response_or_retry_after_fund(
        &self,
        resp: reqwest::Response,
        headers: &reqwest::header::HeaderMap,
        body: &[u8],
        funding_ctx: &FundingContext,
        auto_fund_used: &AtomicBool,
    ) -> TransportResult<ResponsePacket> {
        if resp.status() != StatusCode::PAYMENT_REQUIRED {
            return Self::handle_response_with_funding(resp, Some(funding_ctx)).await;
        }

        let diagnostics = format_http_diagnostics(resp.headers());
        let status = resp.status();
        let resp_body = resp.bytes().await.map_err(TransportErrorKind::custom)?;

        if should_suggest_tempo_fund(status, &resp_body)
            && maybe_auto_fund(auto_fund_used, funding_ctx).await?
        {
            let (resolved, fresh_challenge) = self.fetch_fresh_challenge(headers, body).await?;
            let final_resp = self
                .pay_and_retry(&fresh_challenge, &resolved, headers, body, auto_fund_used)
                .await?;
            return Self::handle_response_with_funding(final_resp, Some(funding_ctx)).await;
        }

        let mut error_text = format!("{}{diagnostics}", String::from_utf8_lossy(&resp_body));
        if should_suggest_tempo_fund(status, &resp_body) {
            error_text.push_str(&tempo_wallet_fund_help(funding_ctx));
        }
        Err(TransportErrorKind::http_error(status.as_u16(), error_text))
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
                TransportErrorKind::custom(io::Error::other(
                    "unexpected success on unauthenticated fresh probe",
                ))
            }));
        }

        Self::select_challenge(&fresh_resp, &self.provider)
    }

    /// Parse `WWW-Authenticate` challenges from a 402 response and resolve
    /// the first supported provider for the challenge's chain.
    fn select_challenge(
        resp: &reqwest::Response,
        provider: &P,
    ) -> TransportResult<(P::Provider, mpp::protocol::core::PaymentChallenge)> {
        let challenges = parse_challenges(resp);
        if challenges.is_empty() && resp.headers().get(WWW_AUTHENTICATE_HEADER).is_none() {
            return Err(TransportErrorKind::custom(io::Error::other(format!(
                "402 response missing WWW-Authenticate header{}",
                format_http_diagnostics(resp.headers())
            ))));
        }

        let mut last_resolve_err: Option<TransportError> = None;
        let resolved_pair = challenges.iter().find_map(|c| {
            let (chain_id, _) = extract_challenge_chain_and_currency(c);
            match provider.resolve_for(chain_id) {
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
            TransportErrorKind::custom(io::Error::other(format!(
                "no supported MPP challenge; server offered [{}]",
                offered.join(", "),
            )))
        })
    }

    async fn handle_response(resp: reqwest::Response) -> TransportResult<ResponsePacket> {
        Self::handle_response_with_funding(resp, None).await
    }

    /// Like [`Self::handle_response`] but, when an unsuccessful 402 looks like a
    /// fundable error, appends actionable `tempo wallet fund` help that uses
    /// the per-request `FundingContext` (so the suggested command includes
    /// `--address` and `--network` when known).
    async fn handle_response_with_funding(
        resp: reqwest::Response,
        funding_ctx: Option<&FundingContext>,
    ) -> TransportResult<ResponsePacket> {
        let status = resp.status();
        debug!(%status, "received response from MPP transport");
        let diagnostics = format_http_diagnostics(resp.headers());

        let body = resp.bytes().await.map_err(TransportErrorKind::custom)?;

        if tracing::enabled!(tracing::Level::TRACE) {
            trace!(body = %String::from_utf8_lossy(&body), "response body");
        } else {
            debug!(bytes = body.len(), "retrieved response body");
        }

        if !status.is_success() {
            let mut body_text = format!("{}{diagnostics}", String::from_utf8_lossy(&body));
            if should_suggest_tempo_fund(status, &body) {
                let default_ctx;
                let ctx = match funding_ctx {
                    Some(c) => c,
                    None => {
                        default_ctx = FundingContext::default();
                        &default_ctx
                    }
                };
                body_text.push_str(&tempo_wallet_fund_help(ctx));
            }
            return Err(TransportErrorKind::http_error(status.as_u16(), body_text));
        }

        serde_json::from_slice(&body)
            .map_err(|err| TransportError::deser_err(err, String::from_utf8_lossy(&body)))
    }
}

/// Returns `Some(chain_id)` when a 402 response should trigger the
/// `wallet.tempo.xyz` device-code authorization flow.
///
/// Conditions: known Tempo endpoint, interactive (TTY, not `CI`), and no
/// locally signable key in the Tempo Accounts store for an offered Charge
/// challenge's chain.
/// The picked chain matches the first unresolved challenge — same iteration
/// order [`MppHttpTransport::select_challenge`] uses.
fn tempo_chain_needing_auth(url: &Url, resp: &reqwest::Response) -> Option<u64> {
    if !io::stderr().is_terminal() || env::var_os("CI").is_some() {
        return None;
    }
    pick_chain_needing_auth(url, &parse_challenges(resp))
}

/// Extract all parseable MPP challenges from a 402 response's `WWW-Authenticate` headers.
fn parse_challenges(resp: &reqwest::Response) -> Vec<mpp::protocol::core::PaymentChallenge> {
    let values: Vec<&str> = resp
        .headers()
        .get_all(WWW_AUTHENTICATE_HEADER)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();
    parse_www_authenticate_all(values).into_iter().filter_map(|r| r.ok()).collect()
}

/// Inner logic of [`tempo_chain_needing_auth`], factored out for testing.
fn pick_chain_needing_auth(
    url: &Url,
    challenges: &[mpp::protocol::core::PaymentChallenge],
) -> Option<u64> {
    if !crate::tempo::is_known_tempo_endpoint(url) {
        return None;
    }

    let charge_chains = challenges.iter().filter_map(|challenge| {
        (challenge.method.as_str() == "tempo" && challenge.intent.as_str() == "charge")
            .then(|| extract_challenge_chain_and_currency(challenge).0)
            .flatten()
    });

    charge_chains.into_iter().find(|chain_id| !accounts_store_has_key(*chain_id))
}

fn accounts_store_has_key(chain_id: u64) -> bool {
    let Ok(Some(store)) = TempoAccountsStore::try_open_default() else {
        return false;
    };
    let Ok(account) = store.active_account() else {
        return false;
    };
    let Ok(keys) = store.access_keys() else {
        return false;
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |duration| duration.as_secs());

    keys.into_iter().any(|key| {
        key.account() == account
            && key.chain_id() == chain_id
            && key.is_locally_signable()
            && key.expiry().is_none_or(|expiry| expiry > now)
    })
}

/// Extract `(chainId, currency)` from a parsed MPP challenge.
pub(super) fn extract_challenge_chain_and_currency(
    c: &mpp::protocol::core::PaymentChallenge,
) -> (Option<u64>, Option<String>) {
    use mpp::protocol::methods::tempo::TempoChargeExt;

    if c.method.as_str() != "tempo" || c.intent.as_str() != "charge" {
        return (None, None);
    }
    let Ok(request) = c.request.decode::<mpp::protocol::intents::ChargeRequest>() else {
        return (None, None);
    };
    (request.chain_id(), Some(request.currency))
}

/// Trait for resolving a concrete `PaymentProvider` from a potentially lazy wrapper.
pub(crate) trait ResolveProvider {
    type Provider: PaymentProvider;
    fn resolve_for(&self, chain_id: Option<u64>) -> TransportResult<Self::Provider>;
    /// Drop any cached payment provider so the next `resolve_for` re-runs
    /// selection. Called after the device-code flow writes `store.json`.
    fn invalidate_cached_provider(&self) {}
    fn funding_wallet_address(&self) -> Option<alloy_primitives::Address> {
        None
    }
    fn funding_chain_id(&self) -> Option<u64> {
        None
    }
    fn funding_context(&self, challenge: &mpp::protocol::core::PaymentChallenge) -> FundingContext {
        let (challenge_chain_id, token) = extract_challenge_chain_and_currency(challenge);
        FundingContext {
            wallet_address: self.funding_wallet_address(),
            token,
            chain_id: challenge_chain_id.or_else(|| self.funding_chain_id()).map(Chain::from_id),
        }
    }
    /// Acquire the payment serialization lock. The returned guard must be held
    /// across the entire 402 → pay → retry → response cycle to prevent
    /// colliding expiring-nonce transactions.
    fn lock_pay(&self) -> impl Future<Output = Option<OwnedMutexGuard<()>>> + Send {
        async { None }
    }
}

impl<P: PaymentProvider + Clone> ResolveProvider for P {
    type Provider = P;
    fn resolve_for(&self, _chain_id: Option<u64>) -> TransportResult<P> {
        Ok(self.clone())
    }
}

impl ResolveProvider for LazyAccountsProvider {
    type Provider = TempoAccountsProvider;

    fn resolve_for(&self, chain_id: Option<u64>) -> TransportResult<Self::Provider> {
        self.get_or_init(chain_id)
    }

    fn invalidate_cached_provider(&self) {
        Self::invalidate(self)
    }

    fn funding_wallet_address(&self) -> Option<alloy_primitives::Address> {
        self.inner
            .lock()
            .unwrap()
            .values()
            .next()
            .and_then(|provider| provider.wallet().active_account().ok())
            .or_else(|| {
                TempoAccountsStore::try_open_default().ok().flatten()?.active_account().ok()
            })
    }

    fn funding_chain_id(&self) -> Option<u64> {
        self.inner.lock().unwrap().values().find_map(TempoAccountsProvider::expected_chain_id)
    }

    fn lock_pay(&self) -> impl Future<Output = Option<OwnedMutexGuard<()>>> + Send {
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
            method == "tempo" && intent == "charge"
        }

        fn pay(
            &self,
            challenge: &PaymentChallenge,
        ) -> impl Future<Output = Result<PaymentCredential, MppError>> + Send {
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

    #[derive(Clone, Debug)]
    struct InsufficientBalanceProvider;

    impl PaymentProvider for InsufficientBalanceProvider {
        fn supports(&self, method: &str, intent: &str) -> bool {
            method == "tempo" && intent == "charge"
        }

        async fn pay(&self, _challenge: &PaymentChallenge) -> Result<PaymentCredential, MppError> {
            Err(MppError::InsufficientBalance(Some(
                "wallet has 0 pathUSD but needs 100000".to_string(),
            )))
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
            intent: IntentName::new("charge"),
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

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn write_accounts_store(home: &std::path::Path, chain_id: u64, expiry: Option<u64>) {
        let wallet = home.join("wallet");
        std::fs::create_dir_all(&wallet).unwrap();
        let store = serde_json::json!({
            "tempo-cli.store": {
                "state": {
                    "activeAccount": 0,
                    "chainId": chain_id,
                    "accounts": [{
                        "address": "0x0000000000000000000000000000000000000001"
                    }],
                    "accessKeys": [{
                        "access": "0x0000000000000000000000000000000000000001",
                        "address": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                        "chainId": chain_id,
                        "keyType": "secp256k1",
                        "privateKey": "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
                        "expiry": expiry,
                    }],
                },
            },
        });
        std::fs::write(wallet.join("store.json"), serde_json::to_vec(&store).unwrap()).unwrap();
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
            test_client(),
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
            test_client(),
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
    async fn lazy_transport_pays_charge_from_accounts_store() {
        let _g = crate::tempo::test_env_mutex().lock().await;
        let tempo_home = tempfile::tempdir().unwrap();
        write_accounts_store(tempo_home.path(), 42431, None);
        unsafe { std::env::set_var(crate::tempo::TEMPO_HOME_ENV, tempo_home.path()) };

        let request = Base64UrlJson::from_value(&serde_json::json!({
            "amount": "0",
            "currency": "0x20c0000000000000000000000000000000000000",
            "recipient": "0x0000000000000000000000000000000000000002",
            "methodDetails": {"chainId": 42431},
        }))
        .unwrap();
        let challenge = PaymentChallenge {
            id: "accounts-charge".to_string(),
            realm: "test-realm".to_string(),
            method: MethodName::new("tempo"),
            intent: IntentName::new("charge"),
            request,
            expires: None,
            description: None,
            digest: None,
            opaque: None,
        };
        let www_auth = format_www_authenticate(&challenge).unwrap();

        let app = axum::Router::new().route(
            "/",
            post(move |req: axum::http::Request<axum::body::Body>| {
                let www_auth = www_auth.clone();
                async move {
                    if let Some(auth) = req.headers().get("authorization") {
                        let credential = parse_authorization(auth.to_str().unwrap()).unwrap();
                        assert_eq!(credential.challenge.id, "accounts-charge");
                        assert!(credential.charge_payload().unwrap().is_proof());
                        (
                            AxumStatusCode::OK,
                            axum::Json(serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 1,
                                "result": "0xpaid",
                            })),
                        )
                            .into_response()
                    } else {
                        (
                            AxumStatusCode::PAYMENT_REQUIRED,
                            [("www-authenticate", www_auth)],
                            "Payment Required",
                        )
                            .into_response()
                    }
                }
            }),
        );

        let (base_url, handle) = spawn_server(app).await;
        let mut transport = MppHttpTransport::lazy(test_client(), Url::parse(&base_url).unwrap());
        let response = tower::Service::call(&mut transport, test_request()).await.unwrap();
        assert!(matches!(response, ResponsePacket::Single(response) if response.is_success()));

        handle.abort();
        unsafe { std::env::remove_var(crate::tempo::TEMPO_HOME_ENV) };
    }

    #[tokio::test]
    async fn test_mpp_transport_402_missing_www_authenticate() {
        let app = axum::Router::new()
            .route("/", post(|| async { (AxumStatusCode::PAYMENT_REQUIRED, "pay up") }));

        let (base_url, handle) = spawn_server(app).await;
        let mut transport = MppHttpTransport::new(
            test_client(),
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
    async fn test_mpp_transport_payment_failure_suggests_tempo_wallet_fund() {
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
        let mut transport = MppHttpTransport::new(
            test_client(),
            Url::parse(&base_url).unwrap(),
            InsufficientBalanceProvider,
        );

        let err = tower::Service::call(&mut transport, test_request()).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Tempo wallet payment could not be funded"), "got: {msg}");
        assert!(msg.contains("tempo wallet fund"), "got: {msg}");
        assert!(msg.contains("--no-browser"), "got: {msg}");
        assert!(msg.contains("Requested payment token: 0x20c0"), "got: {msg}");

        handle.abort();
    }

    #[tokio::test]
    async fn test_mpp_transport_retry_402_insufficient_balance_suggests_fund() {
        let (_, www_auth) = test_challenge();

        let app = axum::Router::new().route(
            "/",
            post(move |req: axum::http::Request<axum::body::Body>| {
                let www_auth = www_auth.clone();
                async move {
                    if req.headers().get("authorization").is_some() {
                        (
                            AxumStatusCode::PAYMENT_REQUIRED,
                            [("content-type", "application/problem+json")],
                            serde_json::to_string(
                                &mpp::error::PaymentErrorDetails::session("insufficient-balance")
                                    .with_title("InsufficientBalanceError")
                                    .with_detail(
                                        "Insufficient pathUSD balance: have 0, need 100000",
                                    ),
                            )
                            .unwrap(),
                        )
                            .into_response()
                    } else {
                        (
                            AxumStatusCode::PAYMENT_REQUIRED,
                            [("www-authenticate", www_auth)],
                            "Payment Required".to_string(),
                        )
                            .into_response()
                    }
                }
            }),
        );

        let (base_url, handle) = spawn_server(app).await;
        let mut transport = MppHttpTransport::new(
            test_client(),
            Url::parse(&base_url).unwrap(),
            MockPaymentProvider,
        );

        let err = tower::Service::call(&mut transport, test_request()).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("InsufficientBalanceError"), "got: {msg}");
        assert!(msg.contains("Tempo wallet payment could not be funded"), "got: {msg}");
        assert!(msg.contains("tempo wallet fund"), "got: {msg}");
        assert!(msg.contains("--no-browser"), "got: {msg}");
        assert!(msg.contains("Requested payment token: 0x20c0"), "got: {msg}");

        handle.abort();
    }

    /// Generic `verification-failed` has many non-funding causes (bad signature,
    /// replay, expired challenge, clock skew, ...). The transport must surface
    /// the original error verbatim and must NOT add a "fund your wallet" hint.
    #[tokio::test]
    async fn test_mpp_transport_final_402_verification_failed_does_not_suggest_fund() {
        let (_, www_auth) = test_challenge();

        let app = axum::Router::new().route(
            "/",
            post(move |req: axum::http::Request<axum::body::Body>| {
                let www_auth = www_auth.clone();
                async move {
                    if req.headers().get("authorization").is_some() {
                        (
                            AxumStatusCode::PAYMENT_REQUIRED,
                            [("content-type", "application/problem+json")],
                            serde_json::to_string(
                                &mpp::error::PaymentErrorDetails::core("verification-failed")
                                    .with_title("Verification Failed")
                                    .with_detail("Payment verification failed."),
                            )
                            .unwrap(),
                        )
                            .into_response()
                    } else {
                        (
                            AxumStatusCode::PAYMENT_REQUIRED,
                            [("www-authenticate", www_auth)],
                            "Payment Required".to_string(),
                        )
                            .into_response()
                    }
                }
            }),
        );

        let (base_url, handle) = spawn_server(app).await;
        let mut transport = MppHttpTransport::new(
            test_client(),
            Url::parse(&base_url).unwrap(),
            MockPaymentProvider,
        );

        let err = tower::Service::call(&mut transport, test_request()).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Verification Failed"), "got: {msg}");
        assert!(
            !msg.contains("Tempo wallet payment could not be funded"),
            "verification-failed must not be classified as fundable; got: {msg}"
        );

        handle.abort();
    }

    // --- Classifier unit tests --------------------------------------------

    #[test]
    fn classifier_only_triggers_on_explicit_insufficient_balance_problem() {
        // explicit insufficient-balance → true
        let body = serde_json::to_vec(
            &mpp::error::PaymentErrorDetails::session("insufficient-balance")
                .with_title("InsufficientBalanceError")
                .with_detail("Insufficient pathUSD balance"),
        )
        .unwrap();
        assert!(should_suggest_tempo_fund(StatusCode::PAYMENT_REQUIRED, &body));
    }

    #[test]
    fn classifier_does_not_trigger_on_verification_failed() {
        let body = serde_json::to_vec(
            &mpp::error::PaymentErrorDetails::core("verification-failed")
                .with_title("Verification Failed")
                .with_detail("Payment verification failed."),
        )
        .unwrap();
        assert!(!should_suggest_tempo_fund(StatusCode::PAYMENT_REQUIRED, &body));
    }

    #[test]
    fn classifier_does_not_trigger_on_unrelated_text_with_balance_words() {
        // Free-text 402 body that just happens to mention the word "balance"
        // must NOT trigger the fund suggestion (no structured problem details).
        let body =
            b"402 Payment Required: server could not balance ledger entries; insufficient inputs.";
        assert!(!should_suggest_tempo_fund(StatusCode::PAYMENT_REQUIRED, body));
    }

    #[test]
    fn classifier_does_not_trigger_outside_402() {
        let body = serde_json::to_vec(
            &mpp::error::PaymentErrorDetails::session("insufficient-balance")
                .with_detail("Insufficient balance"),
        )
        .unwrap();
        assert!(!should_suggest_tempo_fund(StatusCode::INTERNAL_SERVER_ERROR, &body));
        assert!(!should_suggest_tempo_fund(StatusCode::OK, &body));
    }

    #[test]
    fn fund_help_includes_address_and_network_for_known_chain() {
        let ctx = FundingContext {
            wallet_address: Some("0x000000000000000000000000000000000000dEaD".parse().unwrap()),
            token: Some("0x20c0".to_string()),
            chain_id: Some(Chain::from_id(42431)),
        };
        let help = tempo_wallet_fund_help(&ctx);
        assert!(help.contains("--address 0x"), "missing --address: {help}");
        assert!(help.contains("--network tempo-moderato"), "missing --network: {help}");
        assert!(help.contains("--no-browser"), "missing --no-browser: {help}");
        assert!(help.contains("Requested payment token: 0x20c0"), "missing token: {help}");

        let mainnet = FundingContext { chain_id: Some(Chain::from_id(4217)), ..ctx };
        let help2 = tempo_wallet_fund_help(&mainnet);
        assert!(help2.contains("--network tempo"), "missing tempo network: {help2}");
    }

    #[test]
    fn auto_fund_policy_blocks_in_ci_and_non_tty() {
        assert!(!interactive_tempo_fund_allowed(Some("1"), true, true, true), "must not run in CI");
        assert!(
            interactive_tempo_fund_allowed(Some("0"), false, true, true),
            "FOUNDRY_MPP_NO_AUTO_FUND=0 must not disable"
        );
        assert!(
            interactive_tempo_fund_allowed(Some("false"), false, true, true),
            "FOUNDRY_MPP_NO_AUTO_FUND=false must not disable"
        );
        assert!(
            !interactive_tempo_fund_allowed(None, false, false, true),
            "stdin must be a terminal"
        );
        assert!(
            !interactive_tempo_fund_allowed(None, false, true, false),
            "stderr must be a terminal"
        );
        assert!(!interactive_tempo_fund_allowed(Some("1"), false, true, true));
        assert!(!interactive_tempo_fund_allowed(Some("true"), false, true, true));
        assert!(interactive_tempo_fund_allowed(None, false, true, true));
    }

    #[tokio::test]
    async fn test_plain_http_402_shows_mpp_setup_instructions() {
        let _g = crate::tempo::test_env_mutex().lock().await;
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

    /// `invalidate_cached_provider` clears the cache so the next
    /// `get_or_init` reopens the store after `ensure_access_key` updates
    /// `store.json`.
    #[tokio::test]
    async fn lazy_accounts_provider_invalidate_clears_cache() {
        let _g = crate::tempo::test_env_mutex().lock().await;
        let dir = tempfile::tempdir().unwrap();
        write_accounts_store(dir.path(), 42431, None);
        unsafe { std::env::set_var(crate::tempo::TEMPO_HOME_ENV, dir.path()) };

        let lazy = LazyAccountsProvider::new("https://rpc.example.com".into());
        let _ = lazy.get_or_init(Some(42431)).expect("store opens");
        assert!(
            lazy.inner.lock().unwrap().contains_key(&Some(42431)),
            "expected provider to be cached"
        );

        ResolveProvider::invalidate_cached_provider(&lazy);
        assert!(lazy.inner.lock().unwrap().is_empty(), "expected cache to be cleared");

        let _ = lazy.get_or_init(Some(42431)).expect("store reopens");
        assert!(
            lazy.inner.lock().unwrap().contains_key(&Some(42431)),
            "expected re-init to repopulate cache"
        );

        unsafe { std::env::remove_var(crate::tempo::TEMPO_HOME_ENV) };
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

    #[test]
    fn pick_chain_needing_auth_reads_accounts_store() {
        let _g = crate::tempo::test_env_mutex().blocking_lock();
        let dir = tempfile::tempdir().unwrap();
        write_accounts_store(dir.path(), 4217, None);
        unsafe { std::env::set_var(crate::tempo::TEMPO_HOME_ENV, dir.path()) };

        let url = Url::parse("https://rpc.mpp.tempo.xyz").unwrap();
        let mk = |chain_id: u64| -> PaymentChallenge {
            PaymentChallenge {
                id: "x".into(),
                realm: "api".into(),
                method: MethodName::new("tempo"),
                intent: IntentName::new("charge"),
                request: Base64UrlJson::from_value(&serde_json::json!({
                    "amount": "1",
                    "currency": "0x20c0000000000000000000000000000000000000",
                    "recipient": "0xabc",
                    "methodDetails": { "chainId": chain_id }
                }))
                .unwrap(),
                expires: None,
                description: None,
                digest: None,
                opaque: None,
            }
        };

        assert_eq!(pick_chain_needing_auth(&url, &[mk(4217)]), None);
        assert_eq!(pick_chain_needing_auth(&url, &[mk(42431)]), Some(42431));

        write_accounts_store(dir.path(), 4217, Some(1));
        assert_eq!(pick_chain_needing_auth(&url, &[mk(4217)]), Some(4217));

        let mut session = mk(4217);
        session.intent = IntentName::new("session");
        assert_eq!(pick_chain_needing_auth(&url, &[session]), None);

        // Non-Tempo host → never triggers, even without a key.
        let stripe_url = Url::parse("https://api.stripe.com").unwrap();
        assert_eq!(pick_chain_needing_auth(&stripe_url, &[mk(42431)]), None);

        unsafe { std::env::remove_var(crate::tempo::TEMPO_HOME_ENV) };
    }
}
