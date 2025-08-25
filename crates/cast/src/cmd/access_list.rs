use crate::{
    Cast,
    tx::{CastTxBuilder, SenderKind, TransactionInputKind},
};
use alloy_ens::NameOrAddress;
use alloy_rpc_types::BlockId;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, LoadConfig},
};
use std::str::FromStr;

/// CLI arguments for `cast access-list`.
#[derive(Debug, Parser)]
pub struct AccessListArgs {
    /// The destination of the transaction.
    #[arg(
        value_name = "TO",
        value_parser = NameOrAddress::from_str
    )]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    #[arg(value_name = "SIG")]
    sig: Option<String>,

    /// The arguments of the function to call.
    #[arg(value_name = "ARGS", allow_negative_numbers = true)]
    args: Vec<String>,

    /// The block height to query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[arg(long, short = 'B')]
    block: Option<BlockId>,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    eth: EthereumOpts,

    /// When calling an RPC with optionally an "input" or a "data" field,
    /// by default both are populated.  Set this to populate one or the
    /// other.  Some nodes do not allow both to exist, hardhat "fork a network"
    /// is an example.
    #[arg(long = "transaction-input-kind", help_heading = "explicitly use \"input\" or \"data\" when calling an rpc where required")]
    pub transaction_input_kind: Option<TransactionInputKind>,
}

impl AccessListArgs {
    pub async fn run(self) -> Result<()> {
        let Self { to, sig, args, tx, eth, block, transaction_input_kind } = self;

        let config = eth.load_config()?;
        let provider = utils::get_provider(&config)?;
        let sender = SenderKind::from_wallet_opts(eth.wallet).await?;

        let (tx, _) = CastTxBuilder::new(&provider, tx, &config, transaction_input_kind)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(None, sig, args)
            .await?
            .build_raw(sender)
            .await?;

        let cast = Cast::new(&provider);

        let access_list: String = cast.access_list(&tx, block).await?;

        sh_println!("{access_list}")?;

        Ok(())
    }
}
