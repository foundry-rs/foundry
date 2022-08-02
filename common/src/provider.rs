//! utils for creating and configuring Providers

use crate::LOCAL_HTTP_POLL_INTERVAL;
use ethers_providers::{Http, Middleware, Provider};
use foundry_config::Chain;
use url::ParseError;

/// Extension trait for `Provider`
#[async_trait::async_trait]
pub trait ProviderExt {
    /// The error type that can occur when creating a provider
    type Error: std::fmt::Debug;

    /// Creates a new instance connected to the given `url`, exit on error
    async fn connect(url: &str) -> Self
    where
        Self: Sized,
    {
        Self::try_connect(url).await.unwrap()
    }

    /// Try to create a new `Provider`
    async fn try_connect(url: &str) -> Result<Self, Self::Error>
    where
        Self: Sized;

    /// Customize `Provider` settings for chain
    ///
    /// Returns the customized `Provider`
    fn for_chain(mut self, chain: impl Into<Chain>) -> Self
    where
        Self: Sized,
    {
        self.set_chain(chain);
        self
    }

    /// Customized `Provider` settings for chain
    fn set_chain(&mut self, chain: impl Into<Chain>) -> &mut Self;
}

#[async_trait::async_trait]
impl ProviderExt for Provider<Http> {
    type Error = ParseError;

    async fn try_connect(url: &str) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        let mut provider = Provider::try_from(url)?;
        if is_local_endpoint(url) {
            provider.set_interval(LOCAL_HTTP_POLL_INTERVAL);
        } else if let Ok(chain_id) = provider.get_chainid().await {
            provider.set_chain(chain_id);
        }

        Ok(provider)
    }

    fn set_chain(&mut self, chain: impl Into<Chain>) -> &mut Self {
        let chain = chain.into();
        if let Some(blocktime) = chain.average_blocktime_hint() {
            self.set_interval(blocktime / 2);
        }
        self
    }
}

/// Returns a http `Provider` that will connect to the given endpoints
///
/// If the endpoints is deemed a local endpoint [`is_local_endpoint()`], then the polling interval
/// is set to [`LOCAL_HTTP_POLL_INTERVAL`]
pub fn http_provider(endpoint: impl AsRef<str>) -> Result<Provider<Http>, ParseError> {
    let url = endpoint.as_ref();
    let provider = Provider::<Http>::try_from(url)?;
    if is_local_endpoint(url) {
        Ok(provider.interval(LOCAL_HTTP_POLL_INTERVAL))
    } else {
        Ok(provider)
    }
}

/// Returns true if the endpoint is local
#[inline]
pub fn is_local_endpoint(url: &str) -> bool {
    url.contains("127.0.0.1") || url.contains("localhost")
}
