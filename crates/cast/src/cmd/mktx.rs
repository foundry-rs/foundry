use crate::tx::{self, CastTxBuilder};
use alloy_ens::NameOrAddress;
use alloy_network::{EthereumWallet, TransactionBuilder, eip2718::Encodable2718};
use alloy_primitives::{Address, hex};
use alloy_provider::Provider;
use alloy_signer::Signer;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{LoadConfig, get_provider},
};
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
    #[arg(allow_negative_numbers = true)]
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

    /// Generate a raw RLP-encoded unsigned transaction.
    ///
    /// Relaxes the wallet requirement.
    #[arg(long)]
    raw_unsigned: bool,

    /// Call `eth_signTransaction` using the `--from` argument or $ETH_FROM as sender
    #[arg(long, requires = "from", conflicts_with = "raw_unsigned")]
    ethsign: bool,
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
        #[arg(allow_negative_numbers = true)]
        args: Vec<String>,
    },
}

impl MakeTxArgs {
    pub async fn run(self) -> Result<()> {
        let Self { to, mut sig, mut args, command, tx, path, eth, raw_unsigned, ethsign } = self;

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

        let config = eth.load_config()?;

        let provider = get_provider(&config)?;

        let tx_builder = CastTxBuilder::new(&provider, tx.clone(), &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .with_blob_data(blob_data)?;

        if raw_unsigned {
            // Build unsigned raw tx
            // Check if nonce is provided when --from is not specified
            // See: <https://github.com/foundry-rs/foundry/issues/11110>
            if eth.wallet.from.is_none() && tx.nonce.is_none() {
                eyre::bail!(
                    "Missing required parameters for raw unsigned transaction. When --from is not provided, you must specify: --nonce"
                );
            }

            // Use zero address as placeholder for unsigned transactions
            let from = eth.wallet.from.unwrap_or(Address::ZERO);

            let raw_tx = tx_builder.build_unsigned_raw(from).await?;

            sh_println!("{raw_tx}")?;
            return Ok(());
        }

        if ethsign {
            // Use "eth_signTransaction" to sign the transaction only works if the node/RPC has
            // unlocked accounts.
            let (tx, _) = tx_builder.build(config.sender).await?;
            let signed_tx = provider.sign_transaction(tx).await?;

            sh_println!("{signed_tx}")?;
            return Ok(());
        }

        // Default to using the local signer.
        // Get the signer from the wallet, and fail if it can't be constructed.
        let signer = eth.wallet.signer().await?;
        let from = signer.address();

        tx::validate_from_address(eth.wallet.from, from)?;

        let (tx, _) = tx_builder.build(&signer).await?;

        let tx = tx.build(&EthereumWallet::new(signer)).await?;

        let signed_tx = hex::encode(tx.encoded_2718());
        sh_println!("0x{signed_tx}")?;

        Ok(())
    }
}
