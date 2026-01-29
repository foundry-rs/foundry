//! Provider-related instantiation and usage utilities.

pub mod curl_transport;
pub mod runtime_transport;

use crate::{
    ALCHEMY_FREE_TIER_CUPS, REQUEST_TIMEOUT,
    provider::{curl_transport::CurlTransport, runtime_transport::RuntimeTransportBuilder},
};
use alloy_chains::NamedChain;
use alloy_network::{Network, NetworkWallet};
use alloy_provider::{
    Identity, ProviderBuilder as AlloyProviderBuilder, RootProvider,
    fillers::{FillProvider, JoinFill, RecommendedFillers, WalletFiller},
    network::{AnyNetwork, EthereumWallet},
};
use alloy_rpc_client::ClientBuilder;
use alloy_transport::{layers::RetryBackoffLayer, utils::guess_local_url};
use eyre::{Result, WrapErr};
use foundry_config::Config;
use reqwest::Url;
use std::{
    marker::PhantomData,
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
pub type RetryProvider<N = AnyNetwork> = RootProvider<N>;

/// Helper type alias for a retry provider with a signer
pub type RetryProviderWithSigner<N = AnyNetwork, W = EthereumWallet> = FillProvider<
    JoinFill<JoinFill<Identity, <N as RecommendedFillers>::RecommendedFillers>, WalletFiller<W>>,
    RootProvider<N>,
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
///
/// This builder is generic over the network type `N`, defaulting to `AnyNetwork`.
#[derive(Debug)]
pub struct ProviderBuilder<N: Network = AnyNetwork> {
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
    /// Whether to accept invalid certificates.
    accept_invalid_certs: bool,
    /// Whether to disable automatic proxy detection.
    no_proxy: bool,
    /// Whether to output curl commands instead of making requests.
    curl_mode: bool,
    /// Phantom data for the network type.
    _network: PhantomData<N>,
}

impl<N: Network> ProviderBuilder<N> {
    /// Creates a new ProviderBuilder helper instance.
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
        let is_local = url.as_ref().is_ok_and(|url| guess_local_url(url.as_str()));

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
            accept_invalid_certs: false,
            no_proxy: false,
            curl_mode: false,
            _network: PhantomData,
        }
    }

    /// Constructs a [ProviderBuilder] instantiated using [Config] values.
    ///
    /// Defaults to `http://localhost:8545` and `Mainnet`.
    pub fn from_config(config: &Config) -> Result<Self> {
        let url = config.get_rpc_url_or_localhost_http()?;
        let mut builder = Self::new(url.as_ref());

        builder = builder.accept_invalid_certs(config.eth_rpc_accept_invalid_certs);

        if let Ok(chain) = config.chain.unwrap_or_default().try_into() {
            builder = builder.chain(chain);
        }

        if let Some(jwt) = config.get_rpc_jwt_secret()? {
            builder = builder.jwt(jwt.as_ref());
        }

        if let Some(rpc_timeout) = config.eth_rpc_timeout {
            builder = builder.timeout(Duration::from_secs(rpc_timeout));
        }

        if let Some(rpc_headers) = config.eth_rpc_headers.clone() {
            builder = builder.headers(rpc_headers);
        }

        Ok(builder)
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

    /// Sets http headers. If `None`, defaults to the already-set value.
    pub fn maybe_headers(mut self, headers: Option<Vec<String>>) -> Self {
        self.headers = headers.unwrap_or(self.headers);
        self
    }

    /// Sets whether to accept invalid certificates.
    pub fn accept_invalid_certs(mut self, accept_invalid_certs: bool) -> Self {
        self.accept_invalid_certs = accept_invalid_certs;
        self
    }

    /// Sets whether to disable automatic proxy detection.
    ///
    /// This can help in sandboxed environments (e.g., Cursor IDE sandbox, macOS App Sandbox)
    /// where system proxy detection via SCDynamicStore causes crashes.
    pub fn no_proxy(mut self, no_proxy: bool) -> Self {
        self.no_proxy = no_proxy;
        self
    }

    /// Sets whether to output curl commands instead of making requests.
    ///
    /// When enabled, the provider will print equivalent curl commands to stdout
    /// instead of actually executing the RPC requests.
    pub fn curl_mode(mut self, curl_mode: bool) -> Self {
        self.curl_mode = curl_mode;
        self
    }

    /// Constructs the `RetryProvider` taking all configs into account.
    pub fn build(self) -> Result<RetryProvider<N>> {
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
            accept_invalid_certs,
            no_proxy,
            curl_mode,
            ..
        } = self;
        let url = url?;

        let retry_layer =
            RetryBackoffLayer::new(max_retry, initial_backoff, compute_units_per_second);

        // If curl_mode is enabled, use CurlTransport instead of RuntimeTransport
        if curl_mode {
            let transport = CurlTransport::new(url).with_headers(headers).with_jwt(jwt);
            let client = ClientBuilder::default().layer(retry_layer).transport(transport, is_local);

            let provider = AlloyProviderBuilder::<_, _, N>::default()
                .connect_provider(RootProvider::new(client));

            return Ok(provider);
        }

        let transport = RuntimeTransportBuilder::new(url)
            .with_timeout(timeout)
            .with_headers(headers)
            .with_jwt(jwt)
            .accept_invalid_certs(accept_invalid_certs)
            .no_proxy(no_proxy)
            .build();
        let client = ClientBuilder::default().layer(retry_layer).transport(transport, is_local);

        if !is_local {
            client.set_poll_interval(
                chain
                    .average_blocktime_hint()
                    // we cap the poll interval because if not provided, chain would default to
                    // mainnet
                    .map(|hint| hint.min(DEFAULT_UNKNOWN_CHAIN_BLOCK_TIME))
                    .unwrap_or(DEFAULT_UNKNOWN_CHAIN_BLOCK_TIME)
                    .mul_f32(POLL_INTERVAL_BLOCK_TIME_SCALE_FACTOR),
            );
        }

        let provider =
            AlloyProviderBuilder::<_, _, N>::default().connect_provider(RootProvider::new(client));

        Ok(provider)
    }
}

impl<N: Network> ProviderBuilder<N> {
    /// Constructs the `RetryProvider` with a wallet.
    pub fn build_with_wallet<W: NetworkWallet<N> + Clone>(
        self,
        wallet: W,
    ) -> Result<RetryProviderWithSigner<N, W>>
    where
        N: RecommendedFillers,
    {
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
            accept_invalid_certs,
            no_proxy,
            curl_mode,
            ..
        } = self;
        let url = url?;

        let retry_layer =
            RetryBackoffLayer::new(max_retry, initial_backoff, compute_units_per_second);

        // If curl_mode is enabled, use CurlTransport instead of RuntimeTransport
        if curl_mode {
            let transport = CurlTransport::new(url).with_headers(headers).with_jwt(jwt);
            let client = ClientBuilder::default().layer(retry_layer).transport(transport, is_local);

            let provider = AlloyProviderBuilder::<_, _, N>::default()
                .with_recommended_fillers()
                .wallet(wallet)
                .connect_provider(RootProvider::new(client));

            return Ok(provider);
        }

        let transport = RuntimeTransportBuilder::new(url)
            .with_timeout(timeout)
            .with_headers(headers)
            .with_jwt(jwt)
            .accept_invalid_certs(accept_invalid_certs)
            .no_proxy(no_proxy)
            .build();

        let client = ClientBuilder::default().layer(retry_layer).transport(transport, is_local);

        if !is_local {
            client.set_poll_interval(
                chain
                    .average_blocktime_hint()
                    // we cap the poll interval because if not provided, chain would default to
                    // mainnet
                    .map(|hint| hint.min(DEFAULT_UNKNOWN_CHAIN_BLOCK_TIME))
                    .unwrap_or(DEFAULT_UNKNOWN_CHAIN_BLOCK_TIME)
                    .mul_f32(POLL_INTERVAL_BLOCK_TIME_SCALE_FACTOR),
            );
        }

        let provider = AlloyProviderBuilder::<_, _, N>::default()
            .with_recommended_fillers()
            .wallet(wallet)
            .connect_provider(RootProvider::new(client));

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
    if let Some(s) = path.to_str()
        && s.starts_with(r"\\.\pipe\")
    {
        return Ok(path.to_path_buf());
    }
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_auto_correct_missing_prefix() {
        let builder = ProviderBuilder::<AnyNetwork>::new("localhost:8545");
        assert!(builder.url.is_ok());

        let url = builder.url.unwrap();
        assert_eq!(url, Url::parse("http://localhost:8545").unwrap());
    }
}
