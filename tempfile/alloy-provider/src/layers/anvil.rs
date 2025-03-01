use alloy_network::Ethereum;
use alloy_node_bindings::{Anvil, AnvilInstance};
use reqwest::Url;
use std::sync::{Arc, OnceLock};

use crate::{Provider, ProviderLayer, RootProvider};

/// A layer that wraps an [`Anvil`] config.
///
/// The config will be used to spawn an [`AnvilInstance`] when the layer is applied, or when the
/// user requests any information about the anvil node (e.g. via the [`AnvilLayer::ws_endpoint_url`]
/// method).
#[derive(Debug, Clone, Default)]
pub struct AnvilLayer {
    anvil: Anvil,
    instance: OnceLock<Arc<AnvilInstance>>,
}

impl AnvilLayer {
    /// Starts the anvil instance, or gets a reference to the existing instance.
    pub fn instance(&self) -> &Arc<AnvilInstance> {
        self.instance.get_or_init(|| Arc::new(self.anvil.clone().spawn()))
    }

    /// Get the instance http endpoint.
    #[doc(alias = "http_endpoint_url")]
    pub fn endpoint_url(&self) -> Url {
        self.instance().endpoint_url()
    }

    /// Get the instance ws endpoint.
    pub fn ws_endpoint_url(&self) -> Url {
        self.instance().ws_endpoint_url()
    }
}

impl From<Anvil> for AnvilLayer {
    fn from(anvil: Anvil) -> Self {
        Self { anvil, instance: OnceLock::new() }
    }
}

impl<P> ProviderLayer<P, Ethereum> for AnvilLayer
where
    P: Provider,
{
    type Provider = AnvilProvider<P>;

    fn layer(&self, inner: P) -> Self::Provider {
        let anvil = self.instance();
        AnvilProvider::new(inner, anvil.clone())
    }
}

/// A provider that wraps an [`AnvilInstance`], preventing the instance from
/// being dropped while the provider is in use.
#[derive(Clone, Debug)]
pub struct AnvilProvider<P> {
    inner: P,
    anvil: Arc<AnvilInstance>,
}

impl<P> AnvilProvider<P>
where
    P: Provider,
{
    /// Creates a new `AnvilProvider` with the given inner provider and anvil
    /// instance.
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(inner: P, anvil: Arc<AnvilInstance>) -> Self {
        Self { inner, anvil }
    }

    /// Expose inner anvil instance.
    pub fn anvil(&self) -> &Arc<AnvilInstance> {
        &self.anvil
    }
}

impl<P> Provider for AnvilProvider<P>
where
    P: Provider,
{
    #[inline(always)]
    fn root(&self) -> &RootProvider {
        self.inner.root()
    }
}
