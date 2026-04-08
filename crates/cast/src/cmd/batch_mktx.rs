//! `cast batch-mktx` command implementation.
//!
//! Creates a signed or unsigned batch transaction using Tempo's native call batching.
//! Outputs the RLP-encoded transaction hex.

use crate::{
    call_spec::CallSpec,
    tx::{self, CastTxBuilder},
};
use alloy_consensus::SignableTransaction;
use alloy_eips::eip2718::Encodable2718;
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_signer::Signer;
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, LoadConfig, parse_function_args},
};
use foundry_common::{FoundryTransactionBuilder, provider::ProviderBuilder};
use tempo_alloy::TempoNetwork;
use tempo_primitives::transaction::Call;

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
        let Self { calls, tx, eth, raw_unsigned, ethsign } = self;
        let has_nonce = tx.nonce.is_some();

        if calls.is_empty() {
            return Err(eyre!("No calls specified. Use --call to specify at least one call."));
        }

        let config = eth.load_config()?;
        let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

        // Resolve signer to detect keychain mode
        let (signer, tempo_access_key) = eth.wallet.maybe_signer().await?;

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

        // Set calls on the transaction
        builder.tx.calls = tempo_calls;

        // Set dummy "to" from first call
        let first_call_to = call_specs.first().map(|s| s.to);
        let builder = builder.with_to(first_call_to.map(Into::into)).await?;
        let mut tx_builder = builder.with_code_sig_and_args(None, None, vec![]).await?;
        tx_builder.tx.clear_kind();
        tx_builder.tx.set_value(U256::ZERO);

        if raw_unsigned {
            if eth.wallet.from.is_none() && !has_nonce {
                eyre::bail!(
                    "Missing required parameters for raw unsigned transaction. When --from is not provided, you must specify: --nonce"
                );
            }

            let from = eth.wallet.from.unwrap_or(Address::ZERO);
            let (tx, _) = tx_builder.build(from).await?;
            let raw_tx =
                alloy_primitives::hex::encode_prefixed(tx.build_unsigned()?.encoded_for_signing());
            sh_println!("{raw_tx}")?;
            return Ok(());
        }

        if ethsign {
            let (tx, _) = tx_builder.build(config.sender).await?;
            let signed_tx = provider.sign_transaction(tx).await?;
            sh_println!("{signed_tx}")?;
            return Ok(());
        }

        // Default: use local signer
        let signer = match signer {
            Some(s) => s,
            None => eth.wallet.signer().await?,
        };
        let from = if let Some(ref access_key) = tempo_access_key {
            access_key.wallet_address
        } else {
            Signer::address(&signer)
        };

        if tempo_access_key.is_none() {
            tx::validate_from_address(eth.wallet.from, from)?;
        }

        let (tx, _) = if tempo_access_key.is_some() {
            tx_builder.build(from).await?
        } else {
            tx_builder.build(&signer).await?
        };

        let signed_tx = if let Some(ref access_key) = tempo_access_key {
            let raw_tx = tx.sign_with_access_key(&signer, access_key.wallet_address).await?;
            alloy_primitives::hex::encode(raw_tx)
        } else {
            let envelope = tx.build(&EthereumWallet::new(signer)).await?;
            alloy_primitives::hex::encode(envelope.encoded_2718())
        };

        sh_println!("0x{signed_tx}")?;

        Ok(())
    }
}
