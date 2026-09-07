use super::auth::{confirm_and_build, confirm_and_build_with_tempo_wallet};
use crate::{
    tempo,
    tx::{self, CastTxBuilder},
};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_eips::Encodable2718;
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, EthereumWallet, Network, NetworkTransactionBuilder};
use alloy_primitives::{Address, hex};
use alloy_provider::Provider;
use alloy_signer::{Signature, Signer};
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    json::print_scalar,
    opts::{EthereumOpts, TransactionOpts},
    utils::{LoadConfig, resolve_lane},
};
use foundry_common::{FoundryTransactionBuilder, provider::ProviderBuilder};
use foundry_wallets::{TempoAccountsWallet, WalletSigner};
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

    /// Skip the EIP-7702 authorization disclosure confirmation.
    #[arg(long)]
    force: bool,

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

    /// Generate a raw signed transaction using the provided 65-byte signature.
    #[arg(
        long,
        value_name = "SIGNATURE",
        requires = "from",
        conflicts_with_all = ["raw_unsigned", "ethsign"]
    )]
    signature: Option<Signature>,
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
        if self.tx.tempo.sponsor_url.is_some() {
            eyre::bail!(
                "--sponsor-url is not supported by cast mktx; use --tempo.sponsor with \
                 --tempo.sponsor-signer or --tempo.sponsor-sig"
            );
        }

        if self.tx.tempo.session_id()?.is_some() {
            return self.run_generic::<TempoNetwork>(None, None).await;
        }

        let (is_tempo, signer, access_key) =
            tempo::resolve_transaction_network_and_signer(&self.tx.tempo, &self.eth).await?;
        if is_tempo {
            self.run_generic::<TempoNetwork>(signer, access_key).await
        } else {
            self.run_generic::<Ethereum>(signer, None).await
        }
    }

    async fn run_generic<N: Network>(
        self,
        pre_resolved_signer: Option<WalletSigner>,
        pre_resolved_access_key: Option<TempoAccountsWallet>,
    ) -> Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        let Self {
            to,
            mut sig,
            mut args,
            command,
            mut tx,
            force,
            path,
            eth,
            raw_unsigned,
            ethsign,
            signature,
        } = self;
        let has_session = tx.tempo.session_id()?.is_some();

        let print_sponsor_hash = tx.tempo.print_sponsor_hash;
        let sponsor_fee_payer = tx.tempo.sponsor;
        let expires_at = tx.tempo.resolve_expires();
        let tempo_sponsor =
            if print_sponsor_hash { None } else { tx.tempo.sponsor_config().await? };

        let blob_data = path.map(std::fs::read).transpose()?;

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
        // The provider is not consulted for fee tokens in `--curl` mode.
        let fee_provider = (!config.eth_rpc_curl).then_some(&provider);

        // Populate `tx.tempo.nonce_key` from `--tempo.lane` before the options are cloned into
        // the builder.
        let resolved_lane = resolve_lane(&mut tx.tempo, &config.root)?;
        let lane = resolved_lane.as_ref();

        let tx_builder = CastTxBuilder::new(&provider, tx.clone(), &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .with_blob_data(blob_data)?;
        let chain = tx_builder.chain();
        let (signer, access_key) = if has_session || pre_resolved_access_key.is_some() {
            tempo::resolve_session_or_wallet_signer(&tx.tempo, &eth.wallet, chain.id()).await?
        } else {
            (pre_resolved_signer, None)
        };

        // If --tempo.print-sponsor-hash was passed, build the tx, print the hash, and exit.
        if print_sponsor_hash {
            // Resolve the actual sender because the sponsor hash commits to it. Tempo access-key
            // transactions must also be prepared before hashing so a pending authorization is
            // included in the digest.
            let (mut tx, from) = if let Some(access_key) = access_key {
                let Some((tx, prepared)) =
                    confirm_and_build_with_tempo_wallet(tx_builder, &access_key, force, None)
                        .await?
                else {
                    return Ok(());
                };
                (tx, prepared.account())
            } else {
                let signer = match signer {
                    Some(signer) => signer,
                    None => eth.wallet.signer().await?,
                };
                let Some(tx) = confirm_and_build(tx_builder, &signer, force, None, false).await?
                else {
                    return Ok(());
                };
                (tx, signer.address())
            };
            let hash =
                tempo::sponsor_hash(fee_provider, chain, &mut tx, from, sponsor_fee_payer).await?;
            return print_scalar(format!("{hash:?}"));
        }

        tempo::print_expires(expires_at)?;

        if raw_unsigned {
            // Without a sender the nonce cannot be fetched.
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
            let Some(mut tx) = confirm_and_build(tx_builder, from, force, lane, false).await?
            else {
                return Ok(());
            };
            tempo::apply_fee_payment::<N, _>(
                tempo_sponsor.as_ref(),
                fee_provider,
                chain,
                &mut tx,
                from,
            )
            .await?;
            return print_scalar(hex::encode_prefixed(tx.build_unsigned()?.encoded_for_signing()));
        }

        if let Some(signature) = signature {
            let signature = signature.normalized_s();
            let from = eth.wallet.from.expect("required by clap");
            let Some(mut tx) = confirm_and_build(tx_builder, from, force, lane, false).await?
            else {
                return Ok(());
            };
            tempo::apply_fee_payment::<N, _>(
                tempo_sponsor.as_ref(),
                fee_provider,
                chain,
                &mut tx,
                from,
            )
            .await?;

            let tx = tx.build_unsigned()?;
            let recovered = signature.recover_address_from_prehash(&tx.signature_hash())?;
            if recovered != from {
                eyre::bail!(
                    "The provided signature recovers to {recovered}, which does not match the specified sender {from}"
                );
            }

            let tx = N::TxEnvelope::from(tx.into_signed(signature));
            return print_scalar(hex::encode_prefixed(tx.encoded_2718()));
        }

        if ethsign {
            // Use "eth_signTransaction" to sign the transaction only works if the node/RPC has
            // unlocked accounts.
            let Some(mut tx) =
                confirm_and_build(tx_builder, config.sender, force, lane, true).await?
            else {
                return Ok(());
            };
            tempo::apply_fee_payment::<N, _>(
                tempo_sponsor.as_ref(),
                fee_provider,
                chain,
                &mut tx,
                config.sender,
            )
            .await?;
            return print_scalar(provider.sign_transaction(tx).await?);
        }

        // Default to using the local signer.
        let signed_tx = if let Some(access_key) = access_key {
            let Some((mut tx, prepared)) =
                confirm_and_build_with_tempo_wallet(tx_builder, &access_key, force, lane).await?
            else {
                return Ok(());
            };
            tempo::apply_fee_payment::<N, _>(
                tempo_sponsor.as_ref(),
                fee_provider,
                chain,
                &mut tx,
                prepared.account(),
            )
            .await?;
            tx.sign_with_tempo_wallet(&prepared).await?
        } else {
            let (signer, from) = tx::resolve_send_signer(signer, &eth).await?;
            let Some(mut tx) = confirm_and_build(tx_builder, &signer, force, lane, false).await?
            else {
                return Ok(());
            };
            tempo::apply_fee_payment::<N, _>(
                tempo_sponsor.as_ref(),
                fee_provider,
                chain,
                &mut tx,
                from,
            )
            .await?;
            tx.build(&EthereumWallet::new(signer)).await?.encoded_2718()
        };

        print_scalar(hex::encode_prefixed(signed_tx))
    }
}
