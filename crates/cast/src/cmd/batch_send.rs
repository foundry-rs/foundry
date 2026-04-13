//! `cast batch-send` command implementation.
//!
//! Sends a batch of calls as a single Tempo transaction using native call batching.
//! Unlike upstream Foundry's sequential transactions, this uses a single type 0x76
//! transaction with multiple calls executed atomically.

use crate::{
    call_spec::CallSpec,
    cmd::send::cast_send,
    tx::{self, CastTxBuilder, CastTxSender, SendTxOpts},
};
use alloy_network::EthereumWallet;
use alloy_primitives::Bytes;
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_signer::Signer;
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{
    opts::TransactionOpts,
    utils::{self, LoadConfig, parse_function_args},
};
use foundry_common::{FoundryTransactionBuilder, provider::ProviderBuilder};
use std::time::Duration;
use tempo_alloy::TempoNetwork;
use tempo_primitives::transaction::Call;

/// CLI arguments for `cast batch-send`.
///
/// Sends multiple calls as a single atomic Tempo transaction.
#[derive(Debug, Parser)]
pub struct BatchSendArgs {
    /// Call specifications in format: `to[:<value>][:<sig>[:<args>]]` or `to[:<value>][:<0xdata>]`
    ///
    /// Examples:
    ///   --call "0x123:0.1ether" (ETH transfer)
    ///   --call "0x456::transfer(address,uint256):0x789,1000" (ERC20 transfer)
    ///   --call "0xabc::0x123def" (raw calldata)
    ///   --call "0x123:1ether:deposit()" (value + function call)
    #[arg(long = "call", value_name = "SPEC", required = true)]
    pub calls: Vec<String>,

    #[command(flatten)]
    pub send_tx: SendTxOpts,

    #[command(flatten)]
    pub tx: TransactionOpts,

    /// Send via `eth_sendTransaction` using the `--from` argument or $ETH_FROM as sender
    #[arg(long, requires = "from")]
    pub unlocked: bool,
}

impl BatchSendArgs {
    pub async fn run(self) -> Result<()> {
        let Self { calls, send_tx, tx, unlocked } = self;

        if calls.is_empty() {
            return Err(eyre!("No calls specified. Use --call to specify at least one call."));
        }

        let config = send_tx.eth.load_config()?;
        let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

        if let Some(interval) = send_tx.poll_interval {
            provider.client().set_poll_interval(Duration::from_secs(interval))
        }

        // Resolve signer to detect keychain mode
        let (signer, tempo_access_key) = send_tx.eth.wallet.maybe_signer().await?;

        // Parse all call specs
        let call_specs: Vec<CallSpec> =
            calls.iter().map(|s| CallSpec::parse(s)).collect::<Result<Vec<_>>>()?;

        // Get chain for parsing function args
        let chain = utils::get_chain(config.chain, &provider).await?;
        let etherscan_api_key = config.get_etherscan_api_key(Some(chain));

        // Build Vec<Call> from specs
        let mut tempo_calls = Vec::with_capacity(call_specs.len());
        for (i, spec) in call_specs.iter().enumerate() {
            let input = if let Some(data) = &spec.data {
                data.clone()
            } else if let Some(sig) = &spec.sig {
                let (encoded, _) = parse_function_args(
                    sig,
                    spec.args.clone(),
                    Some(spec.to),
                    chain,
                    &provider,
                    etherscan_api_key.as_deref(),
                )
                .await
                .map_err(|e| eyre!("Failed to encode call {}: {}", i + 1, e))?;
                Bytes::from(encoded)
            } else {
                Bytes::new()
            };

            tempo_calls.push(Call { to: spec.to.into(), value: spec.value, input });
        }

        sh_println!("Building batch transaction with {} call(s)...", tempo_calls.len())?;

        // Build transaction request with calls
        let mut builder = CastTxBuilder::<TempoNetwork, _, _>::new(&provider, tx, &config).await?;

        // Set key_id for access key transactions
        if let Some(ref access_key) = tempo_access_key {
            builder.tx.set_key_id(access_key.key_address);
        }

        // Access the inner tx and set calls
        builder.tx.calls = tempo_calls;

        // We need to set a dummy "to" to satisfy the state machine, but the calls field
        // will be used by build_aa. Set to first call's target.
        let first_call_to = call_specs.first().map(|s| s.to);
        let builder = builder.with_to(first_call_to.map(Into::into)).await?;

        // Use empty sig/args since we're using calls directly
        let builder = builder.with_code_sig_and_args(None, None, vec![]).await?;

        let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);

        if unlocked {
            let (tx, _) = builder.build(config.sender).await?;
            cast_send(
                provider,
                tx,
                send_tx.cast_async,
                send_tx.sync,
                send_tx.confirmations,
                timeout,
            )
            .await
        } else {
            let signer = match signer {
                Some(s) => s,
                None => send_tx.eth.wallet.signer().await?,
            };
            let from = if let Some(ref access_key) = tempo_access_key {
                access_key.wallet_address
            } else {
                Signer::address(&signer)
            };

            if tempo_access_key.is_none() {
                tx::validate_from_address(send_tx.eth.wallet.from, from)?;
            }

            let (tx_request, _) = if tempo_access_key.is_some() {
                builder.build(from).await?
            } else {
                builder.build(&signer).await?
            };

            if let Some(ref access_key) = tempo_access_key {
                let raw_tx = tx_request
                    .sign_with_access_key(
                        &provider,
                        &signer,
                        access_key.wallet_address,
                        access_key.key_address,
                        access_key.key_authorization.as_ref(),
                    )
                    .await?;

                let cast = CastTxSender::new(&provider);
                let tx_hash = *provider.send_raw_transaction(&raw_tx).await?.tx_hash();
                cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout)
                    .await?;
            } else {
                let wallet = EthereumWallet::from(signer);
                let provider = AlloyProviderBuilder::<_, _, TempoNetwork>::default()
                    .wallet(wallet)
                    .connect_provider(&provider);

                cast_send(
                    provider,
                    tx_request,
                    send_tx.cast_async,
                    send_tx.sync,
                    send_tx.confirmations,
                    timeout,
                )
                .await?;
            }

            Ok(())
        }
    }
}
