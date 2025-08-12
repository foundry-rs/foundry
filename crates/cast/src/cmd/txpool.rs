use alloy_primitives::Address;
use alloy_provider::ext::TxPoolApi;
use clap::Parser;
use foundry_cli::{
    opts::RpcOpts,
    utils::{self, LoadConfig},
};

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
        match self {
            Self::Content { args } => {
                let config = args.load_config()?;
                let provider = utils::get_provider(&config)?;
                sh_println!("{:#?}", provider.txpool_content().await?)?;
            }
            Self::ContentFrom { from, args } => {
                let config = args.load_config()?;
                let provider = utils::get_provider(&config)?;
                sh_println!("{:#?}", provider.txpool_content_from(from).await?)?;
            }
            Self::Inspect { args } => {
                let config = args.load_config()?;
                let provider = utils::get_provider(&config)?;
                sh_println!("{:#?}", provider.txpool_inspect().await?)?;
            }
            Self::Status { args } => {
                let config = args.load_config()?;
                let provider = utils::get_provider(&config)?;
                sh_println!("{:#?}", provider.txpool_status().await?)?;
            }
        };

        Ok(())
    }
}
