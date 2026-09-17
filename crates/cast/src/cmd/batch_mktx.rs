//! `cast batch-mktx` command implementation.
//!
//! Creates a signed or unsigned batch transaction using Tempo's native call batching.
//! Outputs the RLP-encoded transaction hex.

use crate::{
    cmd::{
        auth::{confirm_and_build, confirm_and_build_with_tempo_wallet},
        batch_send::with_batch_calls,
    },
    tempo,
    tx::{self, CastTxBuilder},
};
use alloy_consensus::SignableTransaction;
use alloy_eips::eip2718::Encodable2718;
use alloy_network::{EthereumWallet, NetworkTransactionBuilder};
use alloy_primitives::{Address, hex};
use alloy_provider::Provider;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, resolve_lane},
};
use foundry_common::FoundryTransactionBuilder;
use tempo_alloy::TempoNetwork;

/// CLI arguments for `cast batch-mktx`.
///
/// Creates a signed (or unsigned) batch transaction.
#[derive(Debug, Parser)]
pub struct BatchMakeTxArgs {
    /// Call specifications in format: `to[:<value>][:<sig>[:<args>]]` or `to[:<value>][:<0xdata>]`
    ///
    /// Examples:
    ///   --call "0x123:0.1ether" (ETH transfer)
    ///   --call "0x456::transfer(address,uint256):0x789,1000" (ERC20 transfer)
    ///   --call "0xabc::0x123def" (raw calldata)
    #[arg(long = "call", value_name = "SPEC", required = true)]
    pub calls: Vec<String>,

    #[command(flatten)]
    pub tx: TransactionOpts,

    /// Skip the EIP-7702 authorization disclosure confirmation.
    #[arg(long)]
    pub force: bool,

    #[command(flatten)]
    pub eth: EthereumOpts,

    /// Generate a raw RLP-encoded unsigned transaction.
    #[arg(long)]
    pub raw_unsigned: bool,

    /// Call `eth_signTransaction` using the `--from` argument or $ETH_FROM as sender
    #[arg(long, requires = "from", conflicts_with = "raw_unsigned")]
    pub ethsign: bool,
}

impl BatchMakeTxArgs {
    pub async fn run(self) -> Result<()> {
        let Self { calls, mut tx, force, eth, raw_unsigned, ethsign } = self;
        let has_nonce = tx.nonce.is_some();
        let has_session = tx.tempo.session_id()?.is_some();
        let expires_at = tx.tempo.resolve_expires();

        if has_session && raw_unsigned {
            eyre::bail!("--tempo.session/TEMPO_SESSION_ID cannot be combined with --raw-unsigned");
        }
        if has_session && ethsign {
            eyre::bail!("--tempo.session/TEMPO_SESSION_ID cannot be combined with --ethsign");
        }

        let (config, provider) = tempo::tempo_provider(&eth)?;
        // The provider is not consulted for fee tokens in `--curl` mode.
        let fee_provider = (!config.eth_rpc_curl).then_some(&provider);
        let resolved_lane = resolve_lane(&mut tx.tempo, &config.root)?;
        let lane = resolved_lane.as_ref();

        let chain = utils::get_chain(config.chain, &provider).await?;
        // A raw unsigned transaction needs no signer, but the access-key metadata still shapes
        // the request.
        let (signer, tempo_access_key) = if raw_unsigned {
            (None, eth.wallet.maybe_signer_for_chain(chain.id()).await?.1)
        } else {
            tempo::resolve_session_or_wallet_signer(&tx.tempo, &eth.wallet, chain.id()).await?
        };

        // Preserve key_id for modes that do not call build_with_tempo_wallet, such as raw unsigned.
        if let Some(access_key) = &tempo_access_key {
            tx.tempo.key_id = Some(access_key.key_id()?);
        }

        let builder = CastTxBuilder::<TempoNetwork, _, _>::new(&provider, tx, &config).await?;
        let tx_builder = with_batch_calls(&calls, builder, &provider).await?;
        tempo::print_expires(expires_at)?;

        if raw_unsigned {
            if eth.wallet.from.is_none() && !has_nonce {
                eyre::bail!(
                    "Missing required parameters for raw unsigned transaction. When --from is not provided, you must specify: --nonce"
                );
            }

            let from = eth.wallet.from.unwrap_or(Address::ZERO);
            let Some(mut tx) = confirm_and_build(tx_builder, from, force, lane, false).await?
            else {
                return Ok(());
            };
            tempo::resolve_and_print_fee_token(fee_provider, Some(chain), &mut tx, Some(from))
                .await?;
            let raw_tx = hex::encode_prefixed(tx.build_unsigned()?.encoded_for_signing());
            sh_println!("{raw_tx}")?;
            return Ok(());
        }

        if ethsign {
            let Some(mut tx) =
                confirm_and_build(tx_builder, config.sender, force, lane, true).await?
            else {
                return Ok(());
            };
            tempo::resolve_and_print_fee_token(
                fee_provider,
                Some(chain),
                &mut tx,
                Some(config.sender),
            )
            .await?;
            let signed_tx = provider.sign_transaction(tx).await?;
            sh_println!("{signed_tx}")?;
            return Ok(());
        }

        let signed_tx = if let Some(access_key) = &tempo_access_key {
            let Some((mut tx, prepared)) =
                confirm_and_build_with_tempo_wallet(tx_builder, access_key, force, lane).await?
            else {
                return Ok(());
            };
            tempo::resolve_and_print_fee_token(
                fee_provider,
                Some(chain),
                &mut tx,
                Some(prepared.account()),
            )
            .await?;
            tx.sign_with_tempo_wallet(&prepared).await?
        } else {
            let (signer, from) = tx::resolve_send_signer(signer, &eth).await?;
            let Some(mut tx) = confirm_and_build(tx_builder, &signer, force, lane, false).await?
            else {
                return Ok(());
            };
            tempo::resolve_and_print_fee_token(fee_provider, Some(chain), &mut tx, Some(from))
                .await?;
            tx.build(&EthereumWallet::new(signer)).await?.encoded_2718()
        };

        sh_println!("{}", hex::encode_prefixed(signed_tx))?;

        Ok(())
    }
}
