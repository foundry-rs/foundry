//! Estimates the data availability size of a block for opstack.

use alloy_consensus::BlockHeader;
use alloy_network::{AnyNetwork, BlockResponse, Ethereum, Network, eip2718::Encodable2718};
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use clap::Parser;
use eyre::Result;
use foundry_cli::{opts::RpcOpts, utils::LoadConfig};
use foundry_common::provider::ProviderBuilder;
use foundry_config::Config;
use foundry_evm_networks::NetworkVariant;
use op_alloy_network::Optimism;

/// CLI arguments for `cast da-estimate`.
#[derive(Debug, Parser)]
pub struct DAEstimateArgs {
    /// The block to estimate the data availability size for.
    pub block: BlockId,
    #[command(flatten)]
    pub rpc: RpcOpts,
    /// Specify the Network for correct encoding.
    #[arg(long, short, num_args = 1, value_name = "NETWORK")]
    network: Option<NetworkVariant>,
}

impl DAEstimateArgs {
    /// Load the RPC URL from the config file.
    pub async fn run(self) -> Result<()> {
        let Self { block, rpc, network } = self;
        let config = rpc.load_config()?;
        let network = match network {
            Some(n) => n,
            None => {
                let provider = ProviderBuilder::<AnyNetwork>::from_config(&config)?.build()?;
                provider.get_chain_id().await?.into()
            }
        };
        match network {
            NetworkVariant::Optimism => da_estimate::<Optimism>(&config, block).await,
            NetworkVariant::Ethereum => da_estimate::<Ethereum>(&config, block).await,
            #[cfg(feature = "monad")]
            NetworkVariant::Monad => unsupported_da_estimation("Monad"),
            NetworkVariant::Tempo => unsupported_da_estimation("Tempo"),
        }
    }
}

fn unsupported_da_estimation(network: &str) -> Result<()> {
    Err(eyre::eyre!(
        "DA estimation is not supported for {network}: EIP-4844 blob transactions are not available on this network"
    ))
}

pub async fn da_estimate<N: Network>(config: &Config, block_id: BlockId) -> Result<()> {
    let provider = ProviderBuilder::<N>::from_config(config)?.build()?;
    let block =
        provider.get_block(block_id).full().await?.ok_or_else(|| eyre::eyre!("Block not found"))?;

    let block_number = block.header().number();
    let tx_count = block.transactions().len();
    let mut da_estimate = 0;
    for tx in block.transactions().txns() {
        da_estimate += op_alloy_flz::tx_estimated_size_fjord(&tx.as_ref().encoded_2718());
    }
    sh_status!(
        "Estimated data availability size for block {block_number} with {tx_count} transactions:"
    )?;
    sh_println!("{da_estimate}")?;
    Ok(())
}

#[cfg(all(test, feature = "monad"))]
mod tests {
    use super::*;

    #[tokio::test]
    #[cfg(feature = "monad")]
    async fn monad_da_estimate_is_unsupported() {
        let args = DAEstimateArgs {
            block: BlockId::latest(),
            rpc: RpcOpts::default(),
            network: Some(NetworkVariant::Monad),
        };

        let err = args.run().await.unwrap_err().to_string();
        assert!(err.contains("Monad"), "{err}");
        assert!(err.contains("EIP-4844"), "{err}");
    }
}
