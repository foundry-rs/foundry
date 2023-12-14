//! Commonly used helpers to construct `Provider`s

use crate::{
    provider::runtime_transport::RuntimeTransportBuilder, ALCHEMY_FREE_TIER_CUPS, REQUEST_TIMEOUT,
};
use alloy_primitives::U256;
use alloy_providers::provider::{Provider, TempProvider};
use alloy_rpc_client::ClientBuilder;
use alloy_transport::BoxTransport;
use ethers_middleware::gas_oracle::{GasCategory, GasOracle, Polygon};
use eyre::{Result, WrapErr};
use foundry_common::types::ToAlloy;
use foundry_config::NamedChain;
use reqwest::Url;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use url::ParseError;

use super::tower::RetryBackoffLayer;

/// Helper type alias for a retry provider
pub type RetryProvider = Provider<BoxTransport>;

/// Helper type alias for a rpc url
pub type RpcUrl = String;

/// Constructs a provider with a 100 millisecond interval poll if it's a localhost URL (most likely
/// an anvil or other dev node) and with the default, or 7 second otherwise.
///
/// See [`try_get_http_provider`] for more details.
///
/// # Panics
///
/// Panics if the URL is invalid.
///
/// # Examples
///
/// ```
/// use foundry_common::provider::alloy::get_http_provider;
///
/// let retry_provider = get_http_provider("http://localhost:8545");
/// ```
#[inline]
#[track_caller]
pub fn get_http_provider(builder: impl AsRef<str>) -> RetryProvider {
    try_get_http_provider(builder).unwrap()
}

/// Constructs a provider with a 100 millisecond interval poll if it's a localhost URL (most likely
/// an anvil or other dev node) and with the default, or 7 second otherwise.
#[inline]
pub fn try_get_http_provider(builder: impl AsRef<str>) -> Result<RetryProvider> {
    ProviderBuilder::new(builder.as_ref()).build()
}

/// Helper type to construct a `RetryProvider`
#[derive(Debug)]
pub struct ProviderBuilder {
    // Note: this is a result, so we can easily chain builder calls
    url: Result<Url>,
    chain: NamedChain,
    max_retry: u32,
    timeout_retry: u32,
    initial_backoff: u64,
    timeout: Duration,
    /// available CUPS
    compute_units_per_second: u64,
    /// JWT Secret
    jwt: Option<String>,
    headers: Vec<String>,
}

// === impl ProviderBuilder ===

impl ProviderBuilder {
    /// Creates a new builder instance
    pub fn new(url_str: &str) -> Self {
        // a copy is needed for the next lines to work
        let mut url_str = url_str;

        // invalid url: non-prefixed URL scheme is not allowed, so we prepend the default http
        // prefix
        let storage;
        if url_str.starts_with("localhost:") {
            storage = format!("http://{url_str}");
            url_str = storage.as_str();
        }

        let url = Url::parse(url_str)
            .or_else(|err| match err {
                ParseError::RelativeUrlWithoutBase => {
                    let path = Path::new(url_str);

                    if let Ok(path) = resolve_path(path) {
                        Url::parse(&format!("file://{}", path.display()))
                    } else {
                        Err(err)
                    }
                }
                _ => Err(err),
            })
            .wrap_err_with(|| format!("invalid provider URL: {url_str:?}"));

        Self {
            url,
            chain: NamedChain::Mainnet,
            max_retry: 8,
            timeout_retry: 8,
            initial_backoff: 800,
            timeout: REQUEST_TIMEOUT,
            // alchemy max cpus <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
            compute_units_per_second: ALCHEMY_FREE_TIER_CUPS,
            jwt: None,
            headers: vec![],
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
    pub fn chain(mut self, chain: NamedChain) -> Self {
        self.chain = chain;
        self
    }

    /// How often to retry a failed request
    pub fn max_retry(mut self, max_retry: u32) -> Self {
        self.max_retry = max_retry;
        self
    }

    /// How often to retry a failed request. If `None`, defaults to the already-set value.
    pub fn maybe_max_retry(mut self, max_retry: Option<u32>) -> Self {
        self.max_retry = max_retry.unwrap_or(self.max_retry);
        self
    }

    /// The starting backoff delay to use after the first failed request. If `None`, defaults to
    /// the already-set value.
    pub fn maybe_initial_backoff(mut self, initial_backoff: Option<u64>) -> Self {
        self.initial_backoff = initial_backoff.unwrap_or(self.initial_backoff);
        self
    }

    /// How often to retry a failed request due to connection issues
    pub fn timeout_retry(mut self, timeout_retry: u32) -> Self {
        self.timeout_retry = timeout_retry;
        self
    }

    /// The starting backoff delay to use after the first failed request
    pub fn initial_backoff(mut self, initial_backoff: u64) -> Self {
        self.initial_backoff = initial_backoff;
        self
    }

    /// Sets the number of assumed available compute units per second
    ///
    /// See also, <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
    pub fn compute_units_per_second(mut self, compute_units_per_second: u64) -> Self {
        self.compute_units_per_second = compute_units_per_second;
        self
    }

    /// Sets the number of assumed available compute units per second
    ///
    /// See also, <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
    pub fn compute_units_per_second_opt(mut self, compute_units_per_second: Option<u64>) -> Self {
        if let Some(cups) = compute_units_per_second {
            self.compute_units_per_second = cups;
        }
        self
    }

    /// Sets aggressive `max_retry` and `initial_backoff` values
    ///
    /// This is only recommend for local dev nodes
    pub fn aggressive(self) -> Self {
        self.max_retry(100).initial_backoff(100)
    }

    /// Sets the JWT secret
    pub fn jwt(mut self, jwt: impl Into<String>) -> Self {
        self.jwt = Some(jwt.into());
        self
    }

    /// Sets http headers
    pub fn headers(mut self, headers: Vec<String>) -> Self {
        self.headers = headers;

        self
    }

    /// Same as [`Self:build()`] but also retrieves the `chainId` in order to derive an appropriate
    /// interval.
    pub async fn connect(self) -> Result<RetryProvider> {
        let provider = self.build()?;
        // todo: port poll interval hint
        /*if let Some(blocktime) = provider.get_chainid().await.ok().and_then(|id| {
        }) {
            provider = provider.interval(blocktime / 2);
            }*/
        Ok(provider)
    }

    /// Constructs the `RetryProvider` taking all configs into account.
    pub fn build(self) -> Result<RetryProvider> {
        let ProviderBuilder {
            url,
            chain: _,
            max_retry,
            timeout_retry,
            initial_backoff,
            timeout,
            compute_units_per_second,
            jwt,
            headers,
        } = self;
        let url = url?;

        let retry_layer = RetryBackoffLayer::new(
            max_retry,
            timeout_retry,
            initial_backoff,
            compute_units_per_second,
        );
        let transport = RuntimeTransportBuilder::new(url.clone())
            .with_timeout(timeout)
            .with_headers(headers)
            .with_jwt(jwt)
            .build();
        let client = ClientBuilder::default().layer(retry_layer).transport(transport, false);

        // todo: provider polling interval
        Ok(Provider::new_with_client(client.boxed()))
    }
}

/// Estimates EIP1559 fees depending on the chain
///
/// Uses custom gas oracles for
///   - polygon
///
/// Fallback is the default [`Provider::estimate_eip1559_fees`] implementation
pub async fn estimate_eip1559_fees<P: TempProvider>(
    provider: &P,
    chain: Option<u64>,
) -> Result<(U256, U256)> {
    let chain = if let Some(chain) = chain {
        chain
    } else {
        provider.get_chain_id().await.wrap_err("Failed to get chain id")?.to()
    };

    if let Ok(chain) = NamedChain::try_from(chain) {
        // handle chains that deviate from `eth_feeHistory` and have their own oracle
        match chain {
            NamedChain::Polygon | NamedChain::PolygonMumbai => {
                // TODO: phase this out somehow
                let chain = match chain {
                    NamedChain::Polygon => ethers_core::types::Chain::Polygon,
                    NamedChain::PolygonMumbai => ethers_core::types::Chain::PolygonMumbai,
                    _ => unreachable!(),
                };
                let estimator = Polygon::new(chain)?.category(GasCategory::Standard);
                let (a, b) = estimator.estimate_eip1559_fees().await?;
                return Ok((a.to_alloy(), b.to_alloy()));
            }
            _ => {}
        }
    }
    provider.estimate_eip1559_fees(None).await.wrap_err("Failed fetch EIP1559 fees")
}

#[cfg(not(windows))]
fn resolve_path(path: &Path) -> Result<PathBuf, ()> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir().map(|d| d.join(path)).map_err(drop)
    }
}

#[cfg(windows)]
fn resolve_path(path: &Path) -> Result<PathBuf, ()> {
    if let Some(s) = path.to_str() {
        if s.starts_with(r"\\.\pipe\") {
            return Ok(path.to_path_buf());
        }
    }
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_auto_correct_missing_prefix() {
        let builder = ProviderBuilder::new("localhost:8545");
        assert!(builder.url.is_ok());

        let url = builder.url.unwrap();
        assert_eq!(url, Url::parse("http://localhost:8545").unwrap());
    }
}
