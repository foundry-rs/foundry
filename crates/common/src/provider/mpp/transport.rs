//! Foundry policy for the canonical MPP Alloy HTTP transport.

use alloy_chains::Chain;
use alloy_json_rpc::{RequestPacket, ResponsePacket};
use alloy_transport::{TransportError, TransportFut};
use alloy_transport_mpp::{MppHttpTransport, MppWsConnect};
use mpp::{
    MppError,
    client::{
        PaymentContext, PaymentProvider, TempoAccountsProvider,
        tempo::{
            autoswap::{AutoswapConfig, DEFAULT_SLIPPAGE_BPS},
            session::store::{SqliteChannelStore, SqliteChannelStoreOptions},
        },
    },
    protocol::{
        core::{PaymentChallenge, PaymentCredential},
        intents::{ChargeRequest, SessionRequest},
    },
};
use std::{
    collections::HashMap,
    env, fmt, io,
    io::IsTerminal,
    process::{Command, Stdio},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    task,
};
use tempo_alloy::accounts::{TempoAccountsError, TempoAccountsStore};
use tower::Service;
use url::Url;

/// Keep high-fanout fork database reads from overwhelming paid RPC endpoints.
const MAX_CONCURRENT_MPP_HTTP_REQUESTS: usize = 4;

/// Open a channel with 0.02 tokens when the server does not suggest a deposit.
const DEFAULT_MPP_SESSION_DEPOSIT: u128 = 20_000;

/// Never let a paid RPC reserve more than one six-decimal token automatically.
const MAX_MPP_SESSION_DEPOSIT: u128 = 1_000_000;

/// The MPP transport used by Foundry's runtime transport builder.
#[derive(Clone, Debug)]
pub struct LazyMppHttpTransport(MppHttpTransport<LazyAccountsProvider>);

impl LazyMppHttpTransport {
    /// Create a transport that opens Tempo Accounts only after a paid challenge.
    pub fn lazy(client: reqwest::Client, url: Url) -> Self {
        let provider = LazyAccountsProvider::new(url.to_string());
        Self(
            MppHttpTransport::new(client, url, provider)
                .with_max_concurrent_requests(MAX_CONCURRENT_MPP_HTTP_REQUESTS),
        )
    }

    /// Return the underlying HTTP client.
    pub const fn client(&self) -> &reqwest::Client {
        self.0.client()
    }
}

impl Service<RequestPacket> for LazyMppHttpTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(&mut self, cx: &mut task::Context<'_>) -> task::Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, request: RequestPacket) -> Self::Future {
        self.0.call(request)
    }
}

/// Build the canonical MPP WebSocket connector with Foundry's lazy Accounts
/// provider.
pub(crate) fn lazy_mpp_ws_connect(url: &Url) -> MppWsConnect<LazyAccountsProvider> {
    let mut origin = url.clone();
    let http_scheme = match origin.scheme() {
        "ws" => Some("http"),
        "wss" => Some("https"),
        _ => None,
    };
    if let Some(http_scheme) = http_scheme {
        let _ = origin.set_scheme(http_scheme);
    }
    MppWsConnect::new(url.to_string(), LazyAccountsProvider::new(origin.to_string()))
}

/// Lazily resolves a chain-bound MPP Charge provider from Tempo Accounts.
///
/// HTTP payment mechanics live in `alloy-transport-mpp`; this wrapper contains
/// only Foundry CLI policy: account-store discovery, optional interactive login
/// and funding, and session channel configuration.
#[derive(Clone)]
pub struct LazyAccountsProvider {
    inner: Arc<Mutex<HashMap<Option<u64>, TempoAccountsProvider>>>,
    origin: String,
}

impl fmt::Debug for LazyAccountsProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LazyAccountsProvider")
            .field("origin", &redacted_url(&self.origin))
            .finish_non_exhaustive()
    }
}

impl LazyAccountsProvider {
    pub(super) fn new(origin: String) -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())), origin }
    }

    fn resolve(&self, chain_id: Option<u64>) -> Result<TempoAccountsProvider, MppError> {
        let mut providers = lock_map(&self.inner);
        if let Some(provider) = providers.get(&chain_id) {
            return Ok(provider.clone());
        }

        let mut provider = TempoAccountsProvider::from_default_store().map_err(|error| {
            MppError::InvalidConfig(format!(
                "RPC endpoint returned HTTP 402 Payment Required, but the Tempo Accounts store \
                 could not provide a Charge wallet: {error}\n\nAuthorize an access key with:\n  \
                 cast tempo login\n\nIn a headless environment, use:\n  cast tempo login --no-browser"
            ))
        })?;
        if let Some(chain_id) = chain_id {
            provider = provider.with_expected_chain_id(chain_id);
        }
        let request_url =
            Url::parse(&self.origin).map_err(|error| MppError::InvalidConfig(error.to_string()))?;
        let store = SqliteChannelStore::open(SqliteChannelStoreOptions {
            namespace: request_url.origin().ascii_serialization(),
            path: None,
            request_url: Some(redacted_url(&self.origin)),
        })
        .map_err(|error| {
            MppError::InvalidConfig(format!("failed to open Tempo channel store: {error}"))
        })?;
        provider = provider
            .with_autoswap(AutoswapConfig::new(
                crate::tempo::PATH_USD_ADDRESS,
                DEFAULT_SLIPPAGE_BPS,
            ))
            .with_session_store(Arc::new(store))
            .with_session_default_deposit(DEFAULT_MPP_SESSION_DEPOSIT)
            .with_session_top_up_amount(DEFAULT_MPP_SESSION_DEPOSIT)
            .with_session_max_deposit(MAX_MPP_SESSION_DEPOSIT);
        providers.insert(chain_id, provider.clone());
        Ok(provider)
    }

    fn invalidate(&self) {
        lock_map(&self.inner).clear();
    }

    async fn provider_for(
        &self,
        challenge: &PaymentChallenge,
    ) -> Result<TempoAccountsProvider, MppError> {
        let (chain_id, _) = extract_challenge_chain_and_currency(challenge);
        let provider = self.resolve(chain_id)?;
        let Some(chain_id) = chain_id else {
            return Ok(provider);
        };
        if !interactive_login_allowed()
            || !Url::parse(&self.origin)
                .is_ok_and(|origin| crate::tempo::is_known_tempo_endpoint(&origin))
            || has_access_key_for_challenge(&provider, challenge, chain_id).await?
        {
            return Ok(provider);
        }

        let config = crate::tempo::EnsureAccessKeyConfig::from_env(chain_id);
        crate::tempo::ensure_access_key(config).await.map_err(|error| {
            MppError::InvalidConfig(format!("Tempo access key authorization failed: {error}"))
        })?;
        self.invalidate();
        self.resolve(Some(chain_id))
    }

    fn funding_context(&self, challenge: &PaymentChallenge) -> FundingContext {
        let (chain_id, token) = extract_challenge_chain_and_currency(challenge);
        FundingContext {
            wallet_address: lock_map(&self.inner)
                .values()
                .next()
                .and_then(|provider| provider.wallet().active_account().ok())
                .or_else(|| {
                    TempoAccountsStore::try_open_default().ok().flatten()?.active_account().ok()
                }),
            token,
            chain_id: chain_id.map(Chain::from_id),
        }
    }
}

impl PaymentProvider for LazyAccountsProvider {
    fn supports(&self, method: &str, intent: &str) -> bool {
        method == "tempo" && matches!(intent, "session" | "charge")
    }

    async fn pay(&self, challenge: &PaymentChallenge) -> Result<PaymentCredential, MppError> {
        let provider = self.provider_for(challenge).await?;
        match provider.pay(challenge).await {
            Ok(credential) => Ok(credential),
            Err(error @ MppError::InsufficientBalance(_)) => {
                let context = self.funding_context(challenge);
                if run_interactive_tempo_fund(&context).await? {
                    provider.pay(challenge).await
                } else {
                    Err(with_funding_help(error, &context))
                }
            }
            Err(error) => Err(error),
        }
    }

    async fn pay_with_context(
        &self,
        challenge: &PaymentChallenge,
        context: PaymentContext,
    ) -> Result<PaymentCredential, MppError> {
        let provider = self.provider_for(challenge).await?;
        match provider.pay_with_context(challenge, context.clone()).await {
            Ok(credential) => Ok(credential),
            Err(error @ MppError::InsufficientBalance(_)) => {
                let funding = self.funding_context(challenge);
                if run_interactive_tempo_fund(&funding).await? {
                    provider.pay_with_context(challenge, context).await
                } else {
                    Err(with_funding_help(error, &funding))
                }
            }
            Err(error) => Err(error),
        }
    }

    async fn commit_payment(
        &self,
        challenge: &PaymentChallenge,
        credential: &PaymentCredential,
    ) -> Result<(), MppError> {
        let (chain_id, _) = extract_challenge_chain_and_currency(challenge);
        self.resolve(chain_id)?.commit_payment(challenge, credential).await
    }

    async fn rollback_payment(
        &self,
        challenge: &PaymentChallenge,
        credential: &PaymentCredential,
    ) -> Result<(), MppError> {
        let (chain_id, _) = extract_challenge_chain_and_currency(challenge);
        self.resolve(chain_id)?.rollback_payment(challenge, credential).await
    }

    fn abandon_payment(&self, challenge: &PaymentChallenge, credential: &PaymentCredential) {
        let (chain_id, _) = extract_challenge_chain_and_currency(challenge);
        if let Ok(provider) = self.resolve(chain_id) {
            provider.abandon_payment(challenge, credential);
        }
    }

    fn accept_payment_header(&self) -> Option<String> {
        Some("tempo/session, tempo/charge;q=0.5".to_owned())
    }
}

async fn has_access_key_for_challenge(
    provider: &TempoAccountsProvider,
    challenge: &PaymentChallenge,
    chain_id: u64,
) -> Result<bool, MppError> {
    if challenge.intent.as_str() == "charge" {
        return provider.has_access_key_for_challenge(challenge).await;
    }
    match provider.wallet().clone().with_chain_id(chain_id).active_access_key() {
        Ok(_) => Ok(true),
        Err(TempoAccountsError::MissingAccessKey { .. }) => Ok(false),
        Err(error) => Err(MppError::InvalidConfig(format!(
            "failed to inspect Tempo Accounts access key: {error}"
        ))),
    }
}

#[derive(Clone, Debug, Default)]
struct FundingContext {
    wallet_address: Option<alloy_primitives::Address>,
    token: Option<String>,
    chain_id: Option<Chain>,
}

impl FundingContext {
    fn help(&self) -> String {
        let mut command = "tempo wallet fund".to_owned();
        if let Some(address) = self.wallet_address {
            command.push_str(&format!(" --address {address}"));
        }
        if let Some(chain) = self.chain_id.filter(|chain| chain.is_tempo()) {
            command.push_str(&format!(" --network {chain}"));
        }
        let token = self
            .token
            .as_ref()
            .map(|token| format!("Requested payment token: {token}\n\n"))
            .unwrap_or_default();
        format!(
            "\n\nTempo wallet payment could not be funded for this paid RPC request.\n\n{token}\
             Fund the wallet, then rerun the command:\n  {command}\n\n\
             If this CLI is running on a remote or headless host, use:\n  {command} --no-browser"
        )
    }
}

fn with_funding_help(error: MppError, context: &FundingContext) -> MppError {
    MppError::InsufficientBalance(Some(format!("{error}{}", context.help())))
}

fn interactive_login_allowed() -> bool {
    !cfg!(test) && env::var_os("CI").is_none() && io::stderr().is_terminal()
}

fn interactive_fund_allowed() -> bool {
    if cfg!(test) || env::var_os("CI").is_some() {
        return false;
    }
    if env::var("FOUNDRY_MPP_NO_AUTO_FUND").ok().is_some_and(|value| {
        !(value == "0" || value.eq_ignore_ascii_case("false") || value.eq_ignore_ascii_case("off"))
    }) {
        return false;
    }
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

async fn run_interactive_tempo_fund(context: &FundingContext) -> Result<bool, MppError> {
    if !interactive_fund_allowed() {
        return Ok(false);
    }

    let binary = env::var("TEMPO_BIN").unwrap_or_else(|_| "tempo".to_owned());
    let mut args = vec!["wallet".to_owned(), "fund".to_owned()];
    if let Some(address) = context.wallet_address {
        args.push("--address".to_owned());
        args.push(address.to_string());
    }
    if let Some(chain) = context.chain_id.filter(|chain| chain.is_tempo()) {
        args.push("--network".to_owned());
        args.push(chain.to_string());
    }
    let help = context.help();
    let status = tokio::task::spawn_blocking(move || {
        Command::new(binary)
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
    })
    .await
    .map_err(|error| MppError::InvalidConfig(format!("failed to join wallet fund: {error}{help}")))?
    .map_err(|error| {
        MppError::InvalidConfig(format!("failed to run wallet fund: {error}{help}"))
    })?;
    if status.success() {
        Ok(true)
    } else {
        Err(MppError::InvalidConfig(format!("wallet fund exited with status {status}{help}")))
    }
}

/// Extract `(chainId, currency)` from a Tempo Charge or Session challenge.
pub(super) fn extract_challenge_chain_and_currency(
    challenge: &PaymentChallenge,
) -> (Option<u64>, Option<String>) {
    use mpp::protocol::methods::tempo::{TempoChargeExt, TempoSessionExt};

    if challenge.method.as_str() != "tempo" {
        return (None, None);
    }
    match challenge.intent.as_str() {
        "charge" => challenge
            .request
            .decode::<ChargeRequest>()
            .map(|request| (request.chain_id(), Some(request.currency)))
            .unwrap_or_default(),
        "session" => challenge
            .request
            .decode::<SessionRequest>()
            .map(|request| (request.chain_id(), Some(request.currency)))
            .unwrap_or_default(),
        _ => (None, None),
    }
}

fn lock_map<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn redacted_url(raw: &str) -> String {
    let Ok(mut redacted) = Url::parse(raw) else {
        return "<invalid>".to_owned();
    };
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.set_query(None);
    redacted.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpp::protocol::core::{Base64UrlJson, IntentName, MethodName};

    fn challenge(method: &str, intent: &str) -> PaymentChallenge {
        PaymentChallenge {
            id: "test".to_owned(),
            realm: "rpc.example".to_owned(),
            method: MethodName::new(method),
            intent: IntentName::new(intent),
            request: Base64UrlJson::from_value(&serde_json::json!({
                "amount": "1",
                "currency": "0x20c0000000000000000000000000000000000000",
                "recipient": "0x0000000000000000000000000000000000000001",
                "methodDetails": {"chainId": 42431}
            }))
            .unwrap(),
            expires: None,
            description: None,
            digest: None,
            opaque: None,
        }
    }

    #[test]
    fn extracts_tempo_charge_routing() {
        assert_eq!(
            extract_challenge_chain_and_currency(&challenge("tempo", "charge")),
            (Some(42431), Some("0x20c0000000000000000000000000000000000000".to_owned()))
        );
        assert_eq!(
            extract_challenge_chain_and_currency(&challenge("tempo", "session")),
            (Some(42431), Some("0x20c0000000000000000000000000000000000000".to_owned()))
        );
        assert_eq!(
            extract_challenge_chain_and_currency(&challenge("stripe", "charge")),
            (None, None)
        );
    }

    #[test]
    fn advertises_sessions_before_charges() {
        let provider = LazyAccountsProvider::new("https://rpc.example".to_owned());
        assert_eq!(
            provider.accept_payment_header().as_deref(),
            Some("tempo/session, tempo/charge;q=0.5")
        );
    }

    #[test]
    fn debug_redacts_origin_secrets() {
        let provider = LazyAccountsProvider::new(
            "https://user:password@example.com/rpc?token=secret".to_owned(),
        );
        let debug = format!("{provider:?}");
        assert!(!debug.contains("password"));
        assert!(!debug.contains("secret"));
        assert!(debug.contains("https://example.com/rpc"));
    }
}
