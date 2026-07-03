use alloy_network::Network;
use alloy_primitives::map::{HashMap, hash_map::Entry};
use alloy_provider::{Provider, RootProvider};
use eyre::{Result, WrapErr};
use foundry_common::provider::{
    ProviderBuilder,
    fee::{ResolvedEip1559Fees, estimate_eip1559_fees},
};
use foundry_config::{Chain, Eip1559FeeEstimatePreset};
use std::{ops::Deref, sync::Arc};

/// Contains a map of RPC urls to single instances of [`ProviderInfo`].
pub struct ProvidersManager<N: Network> {
    pub inner: HashMap<String, ProviderInfo<N>>,
}

impl<N: Network> Default for ProvidersManager<N> {
    fn default() -> Self {
        Self { inner: Default::default() }
    }
}

impl<N: Network> ProvidersManager<N> {
    /// Get or initialize the RPC provider.
    pub async fn get_or_init_provider(
        &mut self,
        rpc: &str,
        is_legacy: bool,
        fee_estimate: Eip1559FeeEstimatePreset,
    ) -> Result<&ProviderInfo<N>> {
        Ok(match self.inner.entry(rpc.to_string()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let info = ProviderInfo::new(rpc, is_legacy, fee_estimate).await?;
                entry.insert(info)
            }
        })
    }
}

impl<N: Network> Deref for ProvidersManager<N> {
    type Target = HashMap<String, ProviderInfo<N>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Holds related metadata to each provider RPC.
#[derive(Debug)]
pub struct ProviderInfo<N: Network> {
    pub provider: Arc<RootProvider<N>>,
    pub chain: u64,
    pub gas_price: GasPrice,
}

/// Represents the outcome of a gas price request
#[derive(Debug)]
pub enum GasPrice {
    Legacy(Result<u128>),
    EIP1559(Result<ResolvedEip1559Fees>),
}

impl<N: Network> ProviderInfo<N> {
    pub async fn new(
        rpc: &str,
        mut is_legacy: bool,
        fee_estimate: Eip1559FeeEstimatePreset,
    ) -> Result<Self> {
        let provider = Arc::new(ProviderBuilder::new(rpc).build()?);
        let chain = provider.get_chain_id().await?;

        if let Some(chain) = Chain::from(chain).named() {
            is_legacy |= chain.is_legacy();
        };

        let gas_price = if is_legacy {
            GasPrice::Legacy(
                provider.get_gas_price().await.wrap_err("Failed to get legacy gas price"),
            )
        } else {
            GasPrice::EIP1559(
                estimate_eip1559_fees(&provider, fee_estimate)
                    .await
                    .wrap_err("Failed to get EIP-1559 fees"),
            )
        };

        Ok(Self { provider, chain, gas_price })
    }

    /// Returns the gas price to use.
    ///
    /// For EIP-1559 chains this is the estimated `maxFeePerGas`.
    pub fn gas_price(&self) -> Result<u128> {
        match &self.gas_price {
            GasPrice::Legacy(res) => match res {
                Ok(val) => Ok(*val),
                Err(err) => Err(eyre::eyre!("{}", err)),
            },
            GasPrice::EIP1559(res) => match res {
                Ok(fees) => Ok(fees.max_fee_per_gas),
                Err(err) => Err(eyre::eyre!("{}", err)),
            },
        }
    }

    /// Returns the resolved EIP-1559 fee breakdown, if available.
    pub const fn eip1559_fees(&self) -> Option<&ResolvedEip1559Fees> {
        match &self.gas_price {
            GasPrice::EIP1559(Ok(fees)) => Some(fees),
            _ => None,
        }
    }
}
