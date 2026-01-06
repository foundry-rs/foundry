use crate::{tx::CastTxBuilder, tx_spec::TxSpec};
use alloy_eips::Encodable2718;
use alloy_ens::NameOrAddress;
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{U64, hex, utils::parse_ether};
use alloy_provider::Provider;
use alloy_signer::Signer;
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::LoadConfig,
};
use std::str::FromStr;

/// CLI arguments for `cast batch-sign`.
#[derive(Debug, Parser)]
pub struct BatchSignArgs {
    /// Transaction specifications in format: to\[:value\]\[:sig\[:args\]\]
    ///
    /// Examples:
    ///   --tx "0x123:0.1ether"  (ETH transfer)
    ///   --tx "0x456::transfer(address,uint256):0x789,1000"  (contract call)
    ///   --tx "0xabc::0x123def"  (raw data)
    #[arg(long = "tx", value_name = "SPEC", value_delimiter = ';')]
    pub transactions: Vec<String>,

    #[command(flatten)]
    pub tx: TransactionOpts,

    #[command(flatten)]
    pub eth: EthereumOpts,

    /// Starting nonce (auto-detected if not provided)
    #[arg(long)]
    pub start_nonce: Option<u64>,
}

impl BatchSignArgs {
    pub async fn run(self) -> Result<()> {
        let Self { transactions, tx, eth, start_nonce } = self;

        if transactions.is_empty() {
            return Err(eyre!("No transactions specified. Use --tx flag to specify transactions."));
        }

        sh_println!("Signing {} transactions...", transactions.len())?;

        // Parse all transaction specs
        let tx_specs: Result<Vec<TxSpec>> = transactions
            .iter()
            .enumerate()
            .map(|(i, spec)| TxSpec::parse(spec).map_err(|e| eyre!("Transaction {}: {}", i + 1, e)))
            .collect();
        let tx_specs = tx_specs?;

        // Set up provider and config
        let config = eth.load_config()?;
        let provider = foundry_cli::utils::get_provider(&config)?;

        // Get signer and create wallet once
        let signer = eth.wallet.signer().await?;
        let sender_addr = signer.address();
        let wallet = EthereumWallet::new(signer);

        // Get starting nonce
        let mut current_nonce = if let Some(nonce) = start_nonce {
            nonce
        } else {
            provider.get_transaction_count(sender_addr).await?
        };

        sh_println!("Starting nonce: {}", current_nonce)?;

        let mut results = Vec::new();

        // Process each transaction
        for (i, tx_spec) in tx_specs.iter().enumerate() {
            sh_println!(
                "Signing transaction {} of {} (nonce: {})...",
                i + 1,
                tx_specs.len(),
                current_nonce
            )?;

            // Parse destination
            let to = NameOrAddress::from_str(&tx_spec.to)
                .map_err(|e| eyre!("Invalid destination address '{}': {}", tx_spec.to, e))?;

            // Parse value
            let value = if let Some(ref value_str) = tx_spec.value {
                Some(parse_ether(value_str)?)
            } else {
                None
            };

            // Create transaction builder
            let mut tx_opts = tx.clone();
            tx_opts.nonce = Some(U64::from(current_nonce));
            if let Some(v) = value {
                tx_opts.value = Some(v);
            }

            let builder = CastTxBuilder::new(&provider, tx_opts, &config)
                .await?
                .with_to(Some(to))
                .await?
                .with_code_sig_and_args(None, tx_spec.sig.clone(), tx_spec.args.clone())
                .await?;

            // Sign the transaction
            let (tx, _) = builder.build(sender_addr).await?;
            let signed_tx = tx.into_inner().build(&wallet).await?;
            let signed_tx_hex = format!("0x{}", hex::encode(signed_tx.encoded_2718()));

            sh_println!("Transaction {} signed", i + 1)?;
            results.push(signed_tx_hex);
            current_nonce += 1;
        }

        // Print all signed transactions
        sh_println!("Batch complete! {} transactions signed:", results.len())?;
        for (i, signed_tx_hex) in results.iter().enumerate() {
            sh_println!("{}. {}", i + 1, signed_tx_hex)?;
        }

        Ok(())
    }
}
