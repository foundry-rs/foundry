use crate::tx::{self, CastTxBuilder};
use alloy_network::{eip2718::Encodable2718, EthereumWallet, TransactionBuilder};
use alloy_primitives::hex;
use alloy_signer::Signer;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, get_provider},
};
use foundry_common::ens::NameOrAddress;
use foundry_config::Config;
use std::{path::PathBuf, str::FromStr};

/// CLI arguments for `cast mktx`.
#[derive(Debug, Parser)]
pub struct MakeTxArgs {
    /// The destination of the transaction.
    ///
    /// If not provided, you must use `cast mktx --create`.
    #[arg(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    args: Vec<String>,

    #[command(subcommand)]
    command: Option<MakeTxSubcommands>,

    #[command(flatten)]
    tx: TransactionOpts,

    /// The path of blob data to be sent.
    #[arg(
        long,
        value_name = "BLOB_DATA_PATH",
        conflicts_with = "legacy",
        requires = "blob",
        help_heading = "Transaction options"
    )]
    path: Option<PathBuf>,

    #[command(flatten)]
    eth: EthereumOpts,
}

#[derive(Debug, Parser)]
pub enum MakeTxSubcommands {
    /// Use to deploy raw contract bytecode.
    #[command(name = "--create")]
    Create {
        /// The initialization bytecode of the contract to deploy.
        code: String,

        /// The signature of the constructor.
        sig: Option<String>,

        /// The constructor arguments.
        args: Vec<String>,
    },
}

impl MakeTxArgs {
    pub async fn run(self) -> Result<()> {
        let Self { to, mut sig, mut args, command, tx, path, eth } = self;

        let blob_data = if let Some(path) = path { Some(std::fs::read(path)?) } else { None };

        let code = if let Some(MakeTxSubcommands::Create {
            code,
            sig: constructor_sig,
            args: constructor_args,
        }) = command
        {
            sig = constructor_sig;
            args = constructor_args;
            Some(code)
        } else {
            None
        };

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;

        let tx_kind = tx::resolve_tx_kind(&provider, &code, &to).await?;

        // Retrieve the signer, and bail if it can't be constructed.
        let signer = eth.wallet.signer().await?;
        let from = signer.address();

        tx::validate_from_address(eth.wallet.from, from)?;

        let provider = get_provider(&config)?;

        let (tx, _) = CastTxBuilder::new(provider, tx, &config)
            .await?
            .with_tx_kind(tx_kind)
            .with_code_sig_and_args(code, sig, args)
            .await?
            .with_blob_data(blob_data)?
            .build(&signer)
            .await?;

        let tx = tx.build(&EthereumWallet::new(signer)).await?;

        let signed_tx = hex::encode(tx.encoded_2718());
        println!("0x{signed_tx}");

        Ok(())
    }
}
