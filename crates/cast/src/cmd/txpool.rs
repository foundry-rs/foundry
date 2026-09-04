use crate::cmd::rpc_provider;
use alloy_primitives::Address;
use alloy_provider::ext::TxPoolApi;
use clap::Parser;
use foundry_cli::{json::print_json_object, opts::RpcOpts};

/// CLI arguments for `cast tx-pool`.
#[derive(Debug, Parser, Clone)]
pub enum TxPoolSubcommands {
    /// Fetches the content of the transaction pool.
    Content {
        #[command(flatten)]
        args: RpcOpts,
    },
    /// Fetches the content of the transaction pool filtered by a specific address.
    ContentFrom {
        /// The Signer to filter the transactions by.
        #[arg(short, long)]
        from: Address,
        #[command(flatten)]
        args: RpcOpts,
    },
    /// Fetches a textual summary of each transaction in the pool.
    Inspect {
        #[command(flatten)]
        args: RpcOpts,
    },
    /// Fetches the current status of the transaction pool.
    Status {
        #[command(flatten)]
        args: RpcOpts,
    },
}

impl TxPoolSubcommands {
    pub async fn run(self) -> eyre::Result<()> {
        let args = match &self {
            Self::Content { args }
            | Self::ContentFrom { args, .. }
            | Self::Inspect { args }
            | Self::Status { args } => args,
        };
        let provider = rpc_provider(args)?;
        match self {
            Self::Content { .. } => print_json_object(provider.txpool_content().await?),
            Self::ContentFrom { from, .. } => {
                print_json_object(provider.txpool_content_from(from).await?)
            }
            Self::Inspect { .. } => print_json_object(provider.txpool_inspect().await?),
            Self::Status { .. } => print_json_object(provider.txpool_status().await?),
        }
    }
}
