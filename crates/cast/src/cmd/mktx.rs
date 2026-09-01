use super::auth::{confirm_auth_rpc_disclosure, confirm_auth_rpc_disclosure_during_build};
#[cfg(feature = "base")]
use crate::cmd::resolve_network;
use crate::{
    tempo,
    tx::{self, CastTxBuilder},
};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_eips::Encodable2718;
use alloy_ens::NameOrAddress;
use alloy_network::{
    Ethereum, EthereumWallet, Network, NetworkTransactionBuilder, TransactionBuilder,
};
use alloy_primitives::{Address, hex};
use alloy_provider::Provider;
use alloy_signer::{Signature, Signer};
#[cfg(feature = "base")]
use base_common_network::Base as BaseNetwork;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    json::print_scalar,
    opts::{EthereumOpts, TransactionOpts},
    utils::{LoadConfig, maybe_print_resolved_lane, resolve_lane},
};
use foundry_common::{
    FoundryTransactionBuilder,
    provider::ProviderBuilder,
    tempo::{maybe_print_fee_token, resolve_and_set_fee_token},
};
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
            return self.run_generic::<TempoNetwork>(signer, access_key).await;
        }

        #[cfg(feature = "base")]
        if resolve_network(&self.eth.load_config()?).await?.is_base() {
            return self.run_generic::<BaseNetwork>(signer, None).await;
        }

        self.run_generic::<Ethereum>(signer, None).await
    }

    pub async fn run_generic<N: Network>(
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
                if !confirm_auth_rpc_disclosure_during_build(
                    &tx_builder,
                    access_key.account(),
                    force,
                )? {
                    return Ok(());
                }
                let (tx, _, prepared) = tx_builder.build_with_tempo_wallet(&access_key).await?;
                (tx, prepared.account())
            } else {
                let signer = match signer {
                    Some(signer) => signer,
                    None => eth.wallet.signer().await?,
                };
                let from = signer.address();
                if !confirm_auth_rpc_disclosure_during_build(&tx_builder, &signer, force)? {
                    return Ok(());
                }
                let (tx, _) = tx_builder.build(&signer).await?;
                (tx, from)
            };
            if let Some(fee_payer) = sponsor_fee_payer {
                resolve_and_set_fee_token(
                    (!config.eth_rpc_curl).then_some(&provider),
                    Some(chain),
                    &mut tx,
                    Some(fee_payer),
                )
                .await?;
            }
            let hash = tx.compute_sponsor_hash(from).ok_or_else(|| {
                eyre::eyre!("This network does not support sponsored transactions")
            })?;
            print_scalar(format!("{hash:?}"))?;
            return Ok(());
        }

        if let Some(ts) = expires_at {
            sh_status!("Transaction expires at unix timestamp {ts}")?;
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

            if !confirm_auth_rpc_disclosure_during_build(&tx_builder, from, force)? {
                return Ok(());
            }
            let (mut tx, _) = tx_builder.build(from).await?;
            maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
            if let Some(sponsor) = &tempo_sponsor {
                sponsor
                    .resolve_and_set_fee_token(
                        (!config.eth_rpc_curl).then_some(&provider),
                        Some(chain),
                        &mut tx,
                    )
                    .await?;
                sponsor.attach_and_print::<N>(&mut tx, from).await?;
            } else {
                let fee_token = resolve_and_set_fee_token(
                    (!config.eth_rpc_curl).then_some(&provider),
                    Some(chain),
                    &mut tx,
                    Some(from),
                )
                .await?;
                maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), fee_token)
                    .await?;
            }
            let raw_tx = hex::encode_prefixed(tx.build_unsigned()?.encoded_for_signing());

            print_scalar(raw_tx)?;
            return Ok(());
        }

        if let Some(signature) = signature {
            let signature = signature.normalized_s();
            let from = eth.wallet.from.expect("required by clap");
            if !confirm_auth_rpc_disclosure_during_build(&tx_builder, from, force)? {
                return Ok(());
            }
            let (mut tx, _) = tx_builder.build(from).await?;
            maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
            if let Some(sponsor) = &tempo_sponsor {
                sponsor
                    .resolve_and_set_fee_token(
                        (!config.eth_rpc_curl).then_some(&provider),
                        Some(chain),
                        &mut tx,
                    )
                    .await?;
                sponsor.attach_and_print::<N>(&mut tx, from).await?;
            } else {
                let fee_token = resolve_and_set_fee_token(
                    (!config.eth_rpc_curl).then_some(&provider),
                    Some(chain),
                    &mut tx,
                    Some(from),
                )
                .await?;
                maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), fee_token)
                    .await?;
            }

            let tx = tx.build_unsigned()?;
            let recovered = signature.recover_address_from_prehash(&tx.signature_hash())?;
            if recovered != from {
                eyre::bail!(
                    "The provided signature recovers to {recovered}, which does not match the specified sender {from}"
                );
            }

            let tx = N::TxEnvelope::from(tx.into_signed(signature));
            print_scalar(hex::encode_prefixed(tx.encoded_2718()))?;
            return Ok(());
        }

        if ethsign {
            // Use "eth_signTransaction" to sign the transaction only works if the node/RPC has
            // unlocked accounts.
            let sender = config.sender.into();
            if tx_builder.has_auth() && !confirm_auth_rpc_disclosure(&tx_builder, &sender, force)? {
                return Ok(());
            }
            let (mut tx, _) = tx_builder.build(config.sender).await?;
            maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
            if let Some(sponsor) = &tempo_sponsor {
                sponsor
                    .resolve_and_set_fee_token(
                        (!config.eth_rpc_curl).then_some(&provider),
                        Some(chain),
                        &mut tx,
                    )
                    .await?;
                sponsor.attach_and_print::<N>(&mut tx, config.sender).await?;
            } else {
                let fee_token = resolve_and_set_fee_token(
                    (!config.eth_rpc_curl).then_some(&provider),
                    Some(chain),
                    &mut tx,
                    Some(config.sender),
                )
                .await?;
                maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), fee_token)
                    .await?;
            }
            let signed_tx = provider.sign_transaction(tx).await?;

            print_scalar(signed_tx)?;
            return Ok(());
        }

        // Default to using the local signer.
        let signed_tx = if let Some(access_key) = access_key {
            if !confirm_auth_rpc_disclosure_during_build(&tx_builder, access_key.account(), force)?
            {
                return Ok(());
            }
            let (mut tx, _, prepared) = tx_builder.build_with_tempo_wallet(&access_key).await?;
            maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
            if let Some(sponsor) = &tempo_sponsor {
                sponsor
                    .resolve_and_set_fee_token(
                        (!config.eth_rpc_curl).then_some(&provider),
                        Some(chain),
                        &mut tx,
                    )
                    .await?;
                sponsor.attach_and_print::<N>(&mut tx, prepared.account()).await?;
            } else {
                let fee_token = resolve_and_set_fee_token(
                    (!config.eth_rpc_curl).then_some(&provider),
                    Some(chain),
                    &mut tx,
                    Some(prepared.account()),
                )
                .await?;
                maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), fee_token)
                    .await?;
            }
            tx.sign_with_tempo_wallet(&prepared).await?
        } else {
            // Get the signer from the wallet, and fail if it can't be constructed.
            let signer = match signer {
                Some(signer) => signer,
                None => eth.wallet.signer().await?,
            };
            let from = signer.address();

            tx::validate_from_address(eth.wallet.from, from)?;

            if !confirm_auth_rpc_disclosure_during_build(&tx_builder, &signer, force)? {
                return Ok(());
            }
            let (mut tx, _) = tx_builder.build(&signer).await?;
            maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
            if let Some(sponsor) = &tempo_sponsor {
                sponsor
                    .resolve_and_set_fee_token(
                        (!config.eth_rpc_curl).then_some(&provider),
                        Some(chain),
                        &mut tx,
                    )
                    .await?;
                sponsor.attach_and_print::<N>(&mut tx, from).await?;
            } else {
                let fee_token = resolve_and_set_fee_token(
                    (!config.eth_rpc_curl).then_some(&provider),
                    Some(chain),
                    &mut tx,
                    Some(from),
                )
                .await?;
                maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), fee_token)
                    .await?;
            }

            tx.build(&EthereumWallet::new(signer)).await?.encoded_2718()
        };

        print_scalar(hex::encode_prefixed(signed_tx))?;
        Ok(())
    }
}
