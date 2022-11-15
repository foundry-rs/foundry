use ethers::prelude::{Http, Middleware, Provider, RetryClient, U256};
use foundry_common::{get_http_provider, RpcUrl};
use foundry_config::Chain;
use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};

#[derive(Default)]
/// Contains a map of RPC urls to single instances of [`ProviderInfo`].
pub struct ProvidersManager {
    pub inner: HashMap<RpcUrl, ProviderInfo>,
}

impl ProvidersManager {
    /// Get or initialize the RPC provider.
    pub async fn get_or_init_provider(
        &mut self,
        rpc: &str,
        is_legacy: bool,
    ) -> eyre::Result<&ProviderInfo> {
        Ok(match self.inner.entry(rpc.to_string()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let info = ProviderInfo::new(rpc, is_legacy).await?;
                entry.insert(info)
            }
        })
    }
}

/// Holds related metadata to each provider RPC.
#[derive(Debug)]
pub struct ProviderInfo {
    pub provider: Arc<Provider<RetryClient<Http>>>,
    pub chain: u64,
    pub gas_price: Option<U256>,
    pub eip1559_fees: Option<(U256, U256)>,
    pub is_legacy: bool,
}

impl ProviderInfo {
    pub async fn new(rpc: &str, mut is_legacy: bool) -> eyre::Result<ProviderInfo> {
        let provider = Arc::new(get_http_provider(rpc));
        let chain = provider.get_chainid().await?.as_u64();

        if let Chain::Named(chain) = Chain::from(chain) {
            is_legacy |= chain.is_legacy();
        };

        let (gas_price, eip1559_fees) = if is_legacy {
            (provider.get_gas_price().await.ok(), None)
        } else {
            (None, provider.estimate_eip1559_fees(None).await.ok())
        };

        Ok(ProviderInfo { provider, chain, gas_price, eip1559_fees, is_legacy })
    }
}
