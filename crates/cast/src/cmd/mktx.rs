use crate::tx::{self, CastTxBuilder};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_eips::Encodable2718;
use alloy_ens::NameOrAddress;
use alloy_network::{
    Ethereum, EthereumWallet, Network, NetworkTransactionBuilder, TransactionBuilder,
};
use alloy_primitives::{Address, hex};
use alloy_provider::Provider;
use alloy_signer::{Signature, Signer};
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{LoadConfig, maybe_print_resolved_lane, resolve_lane},
};
use foundry_common::{FoundryTransactionBuilder, provider::ProviderBuilder};
use std::{path::PathBuf, str::FromStr};
use tempo_alloy::TempoNetwork;

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
        if self.tx.tempo.is_tempo() {
            self.run_generic::<TempoNetwork>().await
        } else {
            self.run_generic::<Ethereum>().await
        }
    }

    pub async fn run_generic<N: Network>(self) -> Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        let Self { to, mut sig, mut args, command, mut tx, path, eth, raw_unsigned, ethsign } =
            self;

        let print_sponsor_hash = tx.tempo.print_sponsor_hash;
        let expires_at = tx.tempo.resolve_expires();
        let tempo_sponsor =
            if print_sponsor_hash { None } else { tx.tempo.sponsor_config().await? };

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

        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;

        // Resolve `--tempo.lane <name>` against the lanes file (default
        // `<root>/tempo.lanes.toml`) and populate `tx.tempo.nonce_key` from the lane.
        // Must happen before `tx.clone()` so the cloned tx carries the resolved nonce_key.
        let resolved_lane = resolve_lane(&mut tx.tempo, &config.root)?;

        let tx_builder = CastTxBuilder::new(&provider, tx.clone(), &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .with_blob_data(blob_data)?;

        // If --tempo.print-sponsor-hash was passed, build the tx, print the hash, and exit.
        if print_sponsor_hash {
            // Resolve the signer to derive the actual sender address, since the
            // sponsor hash commits to the sender.
            let signer = eth.wallet.signer().await?;
            let from = signer.address();
            let (tx, _) = tx_builder.build(from).await?;
            let hash = tx.compute_sponsor_hash(from).ok_or_else(|| {
                eyre::eyre!("This network does not support sponsored transactions")
            })?;
            sh_println!("{hash:?}")?;
            return Ok(());
        }

        if let Some(ts) = expires_at {
            sh_println!("Transaction expires at unix timestamp {ts}")?;
        }

        if raw_unsigned {
            // Build unsigned raw tx
            // Check if nonce is provided when --from is not specified
            // See: <https://github.com/foundry-rs/foundry/issues/11110>
            if eth.wallet.from.is_none() && tx.nonce.is_none() {
                eyre::bail!(
                    "Missing required parameters for raw unsigned transaction. When --from is not provided, you must specify: --nonce"
                );
            }
            if tempo_sponsor.is_some() && eth.wallet.from.is_none() {
                eyre::bail!(
                    "--tempo.sponsor requires --from for --raw-unsigned because the sponsor digest commits to the sender"
                );
            }

            // Use zero address as placeholder for unsigned transactions
            let from = eth.wallet.from.unwrap_or(Address::ZERO);

            let (mut tx, _) = tx_builder.build(from).await?;
            maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
            if let Some(sponsor) = &tempo_sponsor {
                sponsor.attach_and_print::<N>(&mut tx, from).await?;
            }
            let raw_tx = hex::encode_prefixed(tx.build_unsigned()?.encoded_for_signing());

            sh_println!("{raw_tx}")?;
            return Ok(());
        }

        if ethsign {
            // Use "eth_signTransaction" to sign the transaction only works if the node/RPC has
            // unlocked accounts.
            let (mut tx, _) = tx_builder.build(config.sender).await?;
            maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
            if let Some(sponsor) = &tempo_sponsor {
                sponsor.attach_and_print::<N>(&mut tx, config.sender).await?;
            }
            let signed_tx = provider.sign_transaction(tx).await?;

            sh_println!("{signed_tx}")?;
            return Ok(());
        }

        // Default to using the local signer.
        // Get the signer from the wallet, and fail if it can't be constructed.
        let signer = eth.wallet.signer().await?;
        let from = signer.address();

        tx::validate_from_address(eth.wallet.from, from)?;

        let (mut tx, _) = tx_builder.build(&signer).await?;
        maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
        if let Some(sponsor) = &tempo_sponsor {
            sponsor.attach_and_print::<N>(&mut tx, from).await?;
        }

        let tx = tx.build(&EthereumWallet::new(signer)).await?;

        let signed_tx = hex::encode(tx.encoded_2718());
        sh_println!("0x{signed_tx}")?;

        Ok(())
    }
}
