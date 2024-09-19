//! Provider-related instantiation and usage utilities.

pub mod runtime_transport;

use crate::{
    provider::runtime_transport::RuntimeTransportBuilder, ALCHEMY_FREE_TIER_CUPS, REQUEST_TIMEOUT,
};
use alloy_provider::{
    fillers::{ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, WalletFiller},
    network::{AnyNetwork, EthereumWallet},
    Identity, ProviderBuilder as AlloyProviderBuilder, RootProvider,
};
use alloy_rpc_client::ClientBuilder;
use alloy_transport::{
    layers::{RetryBackoffLayer, RetryBackoffService},
    utils::guess_local_url,
};
use eyre::{Result, WrapErr};
use foundry_config::NamedChain;
use reqwest::Url;
use runtime_transport::RuntimeTransport;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};
use url::ParseError;

/// The assumed block time for unknown chains.
/// We assume that these are chains have a faster block time.
const DEFAULT_UNKNOWN_CHAIN_BLOCK_TIME: Duration = Duration::from_secs(3);

/// The factor to scale the block time by to get the poll interval.
const POLL_INTERVAL_BLOCK_TIME_SCALE_FACTOR: f32 = 0.6;

/// Helper type alias for a retry provider
pub type RetryProvider<N = AnyNetwork> = RootProvider<RetryBackoffService<RuntimeTransport>, N>;

/// Helper type alias for a retry provider with a signer
pub type RetryProviderWithSigner<N = AnyNetwork> = FillProvider<
    JoinFill<
        JoinFill<
            Identity,
            JoinFill<
                GasFiller,
                JoinFill<
                    alloy_provider::fillers::BlobGasFiller,
                    JoinFill<NonceFiller, ChainIdFiller>,
                >,
            >,
        >,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider<RetryBackoffService<RuntimeTransport>, N>,
    RetryBackoffService<RuntimeTransport>,
    N,
>;

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
/// use foundry_common::provider::get_http_provider;
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
    initial_backoff: u64,
    timeout: Duration,
    /// available CUPS
    compute_units_per_second: u64,
    /// JWT Secret
    jwt: Option<String>,
    headers: Vec<String>,
    is_local: bool,
}

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
                    if SocketAddr::from_str(url_str).is_ok() {
                        Url::parse(&format!("http://{url_str}"))
                    } else {
                        let path = Path::new(url_str);

                        if let Ok(path) = resolve_path(path) {
                            Url::parse(&format!("file://{}", path.display()))
                        } else {
                            Err(err)
                        }
                    }
                }
                _ => Err(err),
            })
            .wrap_err_with(|| format!("invalid provider URL: {url_str:?}"));

        // Use the final URL string to guess if it's a local URL.
        let is_local = url.as_ref().map_or(false, |url| guess_local_url(url.as_str()));

        Self {
            url,
            chain: NamedChain::Mainnet,
            max_retry: 8,
            initial_backoff: 800,
            timeout: REQUEST_TIMEOUT,
            // alchemy max cpus <https://docs.alchemy.com/reference/compute-units#what-are-cups-compute-units-per-second>
            compute_units_per_second: ALCHEMY_FREE_TIER_CUPS,
            jwt: None,
            headers: vec![],
            is_local,
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

    /// Sets the provider to be local.
    ///
    /// This is useful for local dev nodes.
    pub fn local(mut self, is_local: bool) -> Self {
        self.is_local = is_local;
        self
    }

    /// Sets aggressive `max_retry` and `initial_backoff` values
    ///
    /// This is only recommend for local dev nodes
    pub fn aggressive(self) -> Self {
        self.max_retry(100).initial_backoff(100).local(true)
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

    /// Constructs the `RetryProvider` taking all configs into account.
    pub fn build(self) -> Result<RetryProvider> {
        let Self {
            url,
            chain,
            max_retry,
            initial_backoff,
            timeout,
            compute_units_per_second,
            jwt,
            headers,
            is_local,
        } = self;
        let url = url?;

        let retry_layer =
            RetryBackoffLayer::new(max_retry, initial_backoff, compute_units_per_second);

        let transport = RuntimeTransportBuilder::new(url)
            .with_timeout(timeout)
            .with_headers(headers)
            .with_jwt(jwt)
            .build();
        let client = ClientBuilder::default().layer(retry_layer).transport(transport, is_local);

        if !is_local {
            client.set_poll_interval(
                chain
                    .average_blocktime_hint()
                    .unwrap_or(DEFAULT_UNKNOWN_CHAIN_BLOCK_TIME)
                    .mul_f32(POLL_INTERVAL_BLOCK_TIME_SCALE_FACTOR),
            );
        }

        let provider = AlloyProviderBuilder::<_, _, AnyNetwork>::default()
            .on_provider(RootProvider::new(client));

        Ok(provider)
    }

    /// Constructs the `RetryProvider` with a wallet.
    pub fn build_with_wallet(self, wallet: EthereumWallet) -> Result<RetryProviderWithSigner> {
        let Self {
            url,
            chain,
            max_retry,
            initial_backoff,
            timeout,
            compute_units_per_second,
            jwt,
            headers,
            is_local,
        } = self;
        let url = url?;

        let retry_layer =
            RetryBackoffLayer::new(max_retry, initial_backoff, compute_units_per_second);

        let transport = RuntimeTransportBuilder::new(url)
            .with_timeout(timeout)
            .with_headers(headers)
            .with_jwt(jwt)
            .build();

        let client = ClientBuilder::default().layer(retry_layer).transport(transport, is_local);

        if !is_local {
            client.set_poll_interval(
                chain
                    .average_blocktime_hint()
                    .unwrap_or(DEFAULT_UNKNOWN_CHAIN_BLOCK_TIME)
                    .mul_f32(POLL_INTERVAL_BLOCK_TIME_SCALE_FACTOR),
            );
        }

        let provider = AlloyProviderBuilder::<_, _, AnyNetwork>::default()
            .with_recommended_fillers()
            .wallet(wallet)
            .on_provider(RootProvider::new(client));

        Ok(provider)
    }
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
