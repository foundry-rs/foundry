use crate::cmd::rpc_provider;
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    json::{print_json_object, print_scalar},
    opts::RpcOpts,
};

/// CLI arguments for `cast bal`.
#[derive(Debug, Parser)]
pub struct BalArgs {
    /// The block height or hash to query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    block: Option<BlockId>,

    /// Print the raw RLP encoded block access list.
    #[arg(long)]
    raw: bool,

    #[command(flatten)]
    rpc: RpcOpts,
}

impl BalArgs {
    pub async fn run(self) -> Result<()> {
        let provider = rpc_provider(&self.rpc)?;
        let block = self.block.unwrap_or_default();

        if self.raw {
            let bal = provider
                .get_block_access_list_raw(block)
                .await?
                .ok_or_else(|| missing_bal(block))?;
            print_scalar(bal)
        } else {
            let bal =
                provider.get_block_access_list(block).await?.ok_or_else(|| missing_bal(block))?;
            print_json_object(bal)
        }
    }
}

/// The error for a block whose access list the node did not return.
fn missing_bal(block: BlockId) -> eyre::Report {
    eyre::eyre!("block access list for {block} not found")
}
