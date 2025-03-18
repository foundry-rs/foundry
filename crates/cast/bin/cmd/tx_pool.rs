use alloy_primitives::Address;
use alloy_provider::Provider;
use alloy_rpc_types_txpool::{TxpoolContent, TxpoolContentFrom, TxpoolInspect, TxpoolStatus};
use clap::Parser;
use foundry_cli::{
    opts::RpcOpts,
    utils::{self, LoadConfig},
};

/// CLI arguments for `cast txpool`.
#[derive(Debug, Parser, Clone)]
pub enum TxPoolSubcommands {
    Content {
        #[command(flatten)]
        args: RpcOpts,
    },
    ContentFrom {
        /// The Signer to filter the transactions by.
        #[arg(short, long)]
        from: Address,
        #[command(flatten)]
        args: RpcOpts,
    },
    Inspect {
        #[command(flatten)]
        args: RpcOpts,
    },
    Status {
        #[command(flatten)]
        args: RpcOpts,
    },
}

impl TxPoolSubcommands {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            TxPoolSubcommands::Content { args } => {
                let config = args.load_config()?;
                let provider = utils::get_provider(&config)?;

                sh_println!(
                    "{:#?}",
                    provider
                        .client()
                        .request::<_, TxpoolContent>("txpool_content", "")
                        .boxed()
                        .await?
                )?;
            }
            TxPoolSubcommands::ContentFrom { from, args } => {
                let config = args.load_config()?;
                let provider = utils::get_provider(&config)?;
                sh_println!(
                    "{:#?}",
                    provider
                        .client()
                        .request::<_, TxpoolContentFrom>("txpool_contentFrom", [from])
                        .boxed()
                        .await?
                )?;
            }
            TxPoolSubcommands::Inspect { args } => {
                let config = args.load_config()?;
                let provider = utils::get_provider(&config)?;
                sh_println!(
                    "{:#?}",
                    provider
                        .client()
                        .request::<_, TxpoolInspect>("txpool_inspect", "")
                        .boxed()
                        .await?
                )?;
            }
            TxPoolSubcommands::Status { args } => {
                let config = args.load_config()?;
                let provider = utils::get_provider(&config)?;
                sh_println!(
                    "{:#?}",
                    provider
                        .client()
                        .request::<_, TxpoolStatus>("txpool_status", "")
                        .boxed()
                        .await?
                )?;
            }
        };

        Ok(())
    }
}
