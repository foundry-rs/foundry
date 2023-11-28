use alloy_primitives::U256;
use ethers_providers::{Middleware, Provider};
use eyre::{Result, WrapErr};
use foundry_common::{
    provider::ethers::{get_http_provider, RpcUrl},
    runtime_client::RuntimeClient,
    types::ToAlloy,
};
use foundry_config::Chain;
use std::{
    collections::{hash_map::Entry, HashMap},
    ops::Deref,
    sync::Arc,
};

/// Contains a map of RPC urls to single instances of [`ProviderInfo`].
#[derive(Default)]
pub struct ProvidersManager {
    pub inner: HashMap<RpcUrl, ProviderInfo>,
}

impl ProvidersManager {
    /// Get or initialize the RPC provider.
    pub async fn get_or_init_provider(
        &mut self,
        rpc: &str,
        is_legacy: bool,
    ) -> Result<&ProviderInfo> {
        Ok(match self.inner.entry(rpc.to_string()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let info = ProviderInfo::new(rpc, is_legacy).await?;
                entry.insert(info)
            }
        })
    }
}

impl Deref for ProvidersManager {
    type Target = HashMap<RpcUrl, ProviderInfo>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Holds related metadata to each provider RPC.
#[derive(Debug)]
pub struct ProviderInfo {
    pub provider: Arc<Provider<RuntimeClient>>,
    pub chain: u64,
    pub gas_price: GasPrice,
    pub is_legacy: bool,
}

/// Represents the outcome of a gas price request
#[derive(Debug)]
pub enum GasPrice {
    Legacy(Result<U256>),
    EIP1559(Result<(U256, U256)>),
}

impl ProviderInfo {
    pub async fn new(rpc: &str, mut is_legacy: bool) -> Result<ProviderInfo> {
        let provider = Arc::new(get_http_provider(rpc));
        let chain = provider.get_chainid().await?.as_u64();

        if let Some(chain) = Chain::from(chain).named() {
            is_legacy |= chain.is_legacy();
        };

        let gas_price = if is_legacy {
            GasPrice::Legacy(
                provider
                    .get_gas_price()
                    .await
                    .wrap_err("Failed to get legacy gas price")
                    .map(|p| p.to_alloy()),
            )
        } else {
            GasPrice::EIP1559(
                provider
                    .estimate_eip1559_fees(None)
                    .await
                    .wrap_err("Failed to get EIP-1559 fees")
                    .map(|p| (p.0.to_alloy(), p.1.to_alloy())),
            )
        };

        Ok(ProviderInfo { provider, chain, gas_price, is_legacy })
    }

    /// Returns the gas price to use
    pub fn gas_price(&self) -> Result<U256> {
        let res = match &self.gas_price {
            GasPrice::Legacy(res) => res.as_ref(),
            GasPrice::EIP1559(res) => res.as_ref().map(|res| &res.0),
        };
        match res {
            Ok(val) => Ok(*val),
            Err(err) => Err(eyre::eyre!("{}", err)),
        }
    }
}
