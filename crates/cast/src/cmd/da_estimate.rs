//! Estimates the data availability size of a block for opstack.

use alloy_consensus::BlockHeader;
use alloy_network::eip2718::Encodable2718;
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use clap::Parser;
use foundry_cli::{
    opts::RpcOpts,
    utils::{self, LoadConfig},
};
use foundry_primitives::FoundryTxEnvelope;

/// CLI arguments for `cast da-estimate`.
#[derive(Debug, Parser)]
pub struct DAEstimateArgs {
    /// The block to estimate the data availability size for.
    pub block: BlockId,
    #[command(flatten)]
    pub rpc: RpcOpts,
}

impl DAEstimateArgs {
    /// Load the RPC URL from the config file.
    pub async fn run(self) -> eyre::Result<()> {
        let Self { block, rpc } = self;
        let config = rpc.load_config()?;
        let provider = utils::get_provider(&config)?;
        let block = provider
            .get_block(block)
            .full()
            .await?
            .ok_or_else(|| eyre::eyre!("Block not found"))?;

        let block_number = block.header.number();
        let tx_count = block.transactions.len();
        let mut da_estimate = 0;
        for tx in block.into_transactions_iter() {
            // convert into FoundryTxEnvelope to support all foundry tx types
            let tx = FoundryTxEnvelope::try_from(tx)?;
            da_estimate += op_alloy_flz::tx_estimated_size_fjord(&tx.encoded_2718());
        }

        sh_println!(
            "Estimated data availability size for block {block_number} with {tx_count} transactions: {da_estimate}"
        )?;

        Ok(())
    }
}
