//! `cast batch-mktx` command implementation.
//!
//! Creates a signed or unsigned batch transaction using Tempo's native call batching.
//! Outputs the RLP-encoded transaction hex.

use crate::{
    call_spec::CallSpec,
    tempo,
    tx::{self, CastTxBuilder},
};
use alloy_consensus::SignableTransaction;
use alloy_eips::eip2718::Encodable2718;
use alloy_network::{EthereumWallet, NetworkTransactionBuilder, TransactionBuilder};
use alloy_primitives::Address;
use alloy_provider::Provider;
use alloy_signer::Signer;
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{
    opts::{EthereumOpts, TempoOpts, TransactionOpts},
    utils::{self, LoadConfig, maybe_print_resolved_lane, resolve_lane},
};
use foundry_common::{
    FoundryTransactionBuilder,
    provider::ProviderBuilder,
    tempo::{maybe_print_fee_token, resolve_and_set_fee_token},
};
use foundry_wallets::{TempoAccessKeyConfig, WalletOpts, WalletSigner};
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
        let Self { calls, mut tx, eth, raw_unsigned, ethsign } = self;
        let has_nonce = tx.nonce.is_some();
        let has_session = tx.tempo.session_id()?.is_some();
        let expires_at = tx.tempo.resolve_expires();

        if calls.is_empty() {
            return Err(eyre!("No calls specified. Use --call to specify at least one call."));
        }

        if has_session && raw_unsigned {
            eyre::bail!("--tempo.session/TEMPO_SESSION_ID cannot be combined with --raw-unsigned");
        }
        if has_session && ethsign {
            eyre::bail!("--tempo.session/TEMPO_SESSION_ID cannot be combined with --ethsign");
        }

        let config = eth.load_config()?;
        let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

        // Resolve `--tempo.lane <name>` against the lanes file (default
        // `<root>/tempo.lanes.toml`) and populate `tx.tempo.nonce_key` from the lane.
        let resolved_lane = resolve_lane(&mut tx.tempo, &config.root)?;

        // Parse all call specs
        let call_specs: Vec<CallSpec> =
            calls.iter().map(|s| CallSpec::parse(s)).collect::<Result<Vec<_>>>()?;

        // Get chain for parsing function args
        let chain = utils::get_chain(config.chain, &provider).await?;
        let (signer, tempo_access_key) =
            resolve_signer(&tx.tempo, &eth.wallet, chain.id(), raw_unsigned).await?;
        let etherscan_config = config.get_etherscan_config_with_chain(Some(chain)).ok().flatten();
        let etherscan_api_key = etherscan_config.as_ref().map(|c| c.key.clone());
        let etherscan_api_url = etherscan_config.map(|c| c.api_url);

        let mut tempo_calls = Vec::with_capacity(call_specs.len());
        for (i, spec) in call_specs.iter().enumerate() {
            tempo_calls.push(
                spec.resolve(
                    i,
                    chain,
                    &provider,
                    etherscan_api_key.as_deref(),
                    etherscan_api_url.as_deref(),
                )
                .await?,
            );
        }

        sh_status!("Building batch transaction with {} call(s)...", tempo_calls.len())?;
        tempo::print_expires(expires_at)?;

        // Preserve key_id for modes that do not call build_with_access_key, such as raw unsigned.
        if let Some(ref access_key) = tempo_access_key {
            tx.tempo.key_id = Some(access_key.key_address);
        }

        // Build transaction request with calls
        let mut builder = CastTxBuilder::<TempoNetwork, _, _>::new(&provider, tx, &config).await?;

        // Set calls on the transaction
        builder.tx.calls = tempo_calls;

        // Set dummy "to" from first call
        let first_call_to = call_specs.first().map(|s| s.to);
        let builder = builder.with_to(first_call_to.map(Into::into)).await?;
        let tx_builder = builder.with_code_sig_and_args(None, None, vec![]).await?;

        if raw_unsigned {
            if eth.wallet.from.is_none() && !has_nonce {
                eyre::bail!(
                    "Missing required parameters for raw unsigned transaction. When --from is not provided, you must specify: --nonce"
                );
            }

            let from = eth.wallet.from.unwrap_or(Address::ZERO);
            let (mut tx, _) = tx_builder.build(from).await?;
            maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
            resolve_and_set_fee_token(
                (!config.eth_rpc_curl).then_some(&provider),
                Some(chain),
                &mut tx,
                Some(from),
            )
            .await?;
            maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), tx.fee_token())
                .await?;
            let raw_tx =
                alloy_primitives::hex::encode_prefixed(tx.build_unsigned()?.encoded_for_signing());
            sh_println!("{raw_tx}")?;
            return Ok(());
        }

        if ethsign {
            let (mut tx, _) = tx_builder.build(config.sender).await?;
            maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
            resolve_and_set_fee_token(
                (!config.eth_rpc_curl).then_some(&provider),
                Some(chain),
                &mut tx,
                Some(config.sender),
            )
            .await?;
            maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), tx.fee_token())
                .await?;
            let signed_tx = provider.sign_transaction(tx).await?;
            sh_println!("{signed_tx}")?;
            return Ok(());
        }

        // Default: use local signer
        let signer = match signer {
            Some(s) => s,
            None => eth.wallet.signer().await?,
        };

        let signed_tx = if let Some(ref access_key) = tempo_access_key {
            let (mut tx, _) =
                tx_builder.build_with_access_key(access_key.wallet_address, access_key).await?;
            maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
            resolve_and_set_fee_token(
                (!config.eth_rpc_curl).then_some(&provider),
                Some(chain),
                &mut tx,
                Some(access_key.wallet_address),
            )
            .await?;
            maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), tx.fee_token())
                .await?;
            let raw_tx = tx
                .sign_with_access_key(
                    &provider,
                    &signer,
                    access_key.wallet_address,
                    access_key.key_address,
                    access_key.key_authorization.as_ref(),
                )
                .await?;
            alloy_primitives::hex::encode(raw_tx)
        } else {
            tx::validate_from_address(eth.wallet.from, Signer::address(&signer))?;
            let (mut tx, _) = tx_builder.build(&signer).await?;
            maybe_print_resolved_lane(resolved_lane.as_ref(), tx.nonce().unwrap_or_default())?;
            resolve_and_set_fee_token(
                (!config.eth_rpc_curl).then_some(&provider),
                Some(chain),
                &mut tx,
                Some(Signer::address(&signer)),
            )
            .await?;
            maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), tx.fee_token())
                .await?;
            let envelope = tx.build(&EthereumWallet::new(signer)).await?;
            alloy_primitives::hex::encode(envelope.encoded_2718())
        };

        sh_println!("0x{signed_tx}")?;

        Ok(())
    }
}

async fn resolve_signer(
    tempo: &TempoOpts,
    wallet: &WalletOpts,
    chain_id: u64,
    raw_unsigned: bool,
) -> Result<(Option<WalletSigner>, Option<TempoAccessKeyConfig>)> {
    if raw_unsigned {
        let (_, access_key) = wallet.maybe_signer().await?;
        return Ok((None, access_key));
    }

    tempo::resolve_session_or_wallet_signer(tempo, wallet, chain_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn raw_unsigned_resolver_discards_signer_but_keeps_access_key_metadata() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let wallet = WalletOpts {
                tempo_access_key: Some(
                    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
                        .to_string(),
                ),
                tempo_root_account: Some(address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266")),
                ..Default::default()
            };

            let (signer, access_key) =
                resolve_signer(&TempoOpts::default(), &wallet, 31337, true).await.unwrap();

            assert!(signer.is_none());
            let access_key = access_key.expect("access-key metadata");
            assert_eq!(
                access_key.wallet_address,
                address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266")
            );
            assert_eq!(
                access_key.key_address,
                address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8")
            );
        });
    }
}
