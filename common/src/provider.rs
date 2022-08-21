//! Commonly used helpers to construct `Provider`s

use crate::REQUEST_TIMEOUT;
use ethers_core::types::Chain;
use ethers_providers::{
    is_local_endpoint, Http, HttpRateLimitRetryPolicy, Middleware, Provider, RetryClient,
    DEFAULT_LOCAL_POLL_INTERVAL,
};
use reqwest::{IntoUrl, Url};
use std::time::Duration;

/// Helper type alias for a retry provider
pub type RetryProvider = Provider<RetryClient<Http>>;

/// Same as `try_get_http_provider`
///
/// # Panics
///
/// If invalid URL
///
/// # Example
///
/// ```
/// use foundry_common::get_http_provider;
/// # fn f() {
///  let retry_provider = get_http_provider("http://localhost:8545");
/// # }
/// ```
pub fn get_http_provider(builder: impl Into<ProviderBuilder>) -> RetryProvider {
    try_get_http_provider(builder).unwrap()
}

/// Gives out a provider with a `100ms` interval poll if it's a localhost URL (most likely an anvil
/// or other dev node) and with the default, `7s` if otherwise.
pub fn try_get_http_provider(builder: impl Into<ProviderBuilder>) -> eyre::Result<RetryProvider> {
    builder.into().build()
}

/// Helper type to construct a `RetryProvider`
#[derive(Debug)]
pub struct ProviderBuilder {
    // Note: this is a result so we can easily chain builder calls
    url: reqwest::Result<Url>,
    chain: Chain,
    max_retry: u32,
    initial_backoff: u64,
    timeout: Duration,
}

// === impl ProviderBuilder ===

impl ProviderBuilder {
    /// Creates a new builder instance
    pub fn new(url: impl IntoUrl) -> Self {
        Self {
            url: url.into_url(),
            chain: Chain::Mainnet,
            max_retry: 100,
            initial_backoff: 100,
            timeout: REQUEST_TIMEOUT,
        }
    }

    /// Enables a request timeout.
    ///
    /// The timeout is applied from when the request starts connecting until the
    /// response body has finished.
    ///
    /// Default is no timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the chain of the node the provider will connect to
    pub fn chain(mut self, chain: impl Into<foundry_config::Chain>) -> Self {
        if let foundry_config::Chain::Named(chain) = chain.into() {
            self.chain = chain;
        }
        self
    }

    /// How often to retry a failed request
    pub fn max_retry(mut self, max_retry: u32) -> Self {
        self.max_retry = max_retry;
        self
    }

    /// The starting backoff delay to use after the first failed request
    pub fn initial_backoff(mut self, initial_backoff: u64) -> Self {
        self.initial_backoff = initial_backoff;
        self
    }

    /// Sets aggressive `max_retry` and `initial_backoff` values
    ///
    /// This is only recommend for local dev nodes
    pub fn aggressive(self) -> Self {
        self.max_retry(100).initial_backoff(100)
    }

    /// Same as [`Self:build()`] but also retrieves the `chainId` in order to derive an appropriate
    /// interval
    pub async fn connect(self) -> eyre::Result<RetryProvider> {
        let mut provider = self.build()?;
        if let Some(blocktime) = provider.get_chainid().await.ok().and_then(|id| {
            Chain::try_from(id).ok().and_then(|chain| chain.average_blocktime_hint())
        }) {
            provider = provider.interval(blocktime / 2);
        }
        Ok(provider)
    }

    /// Constructs the `RetryProvider` taking all configs into account
    pub fn build(self) -> eyre::Result<RetryProvider> {
        let ProviderBuilder { url, chain, max_retry, initial_backoff, timeout } = self;
        let url = url?;

        let client = reqwest::Client::builder().timeout(timeout).build()?;
        let is_local = is_local_endpoint(url.as_str());

        let provider = Http::new_with_client(url, client);

        let mut provider = Provider::new(RetryClient::new(
            provider,
            Box::new(HttpRateLimitRetryPolicy::default()),
            max_retry,
            initial_backoff,
        ));

        if is_local {
            provider = provider.interval(DEFAULT_LOCAL_POLL_INTERVAL);
        } else if let Some(blocktime) = chain.average_blocktime_hint() {
            provider = provider.interval(blocktime / 2);
        }
        Ok(provider)
    }
}

impl<'a> From<&'a str> for ProviderBuilder {
    fn from(url: &'a str) -> Self {
        Self::new(url)
    }
}

impl<'a> From<&'a String> for ProviderBuilder {
    fn from(url: &'a String) -> Self {
        url.as_str().into()
    }
}

impl From<String> for ProviderBuilder {
    fn from(url: String) -> Self {
        url.as_str().into()
    }
}
