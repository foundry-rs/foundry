use crate::tx::CastTxBuilder;
use alloy_eips::Encodable2718;
use alloy_ens::NameOrAddress;
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Bytes, U64, hex, utils::parse_ether};
use alloy_provider::Provider;
use alloy_signer::Signer;
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::LoadConfig,
};
use std::str::FromStr;

/// CLI arguments for `cast batch-mktx`.
#[derive(Debug, Parser)]
pub struct BatchMakeTxArgs {
    /// Transaction specifications in format: to[:value][:sig[:args]]
    ///
    /// Examples:
    ///   --tx "0x123:0.1ether"  (ETH transfer)
    ///   --tx "0x456::transfer(address,uint256):0x789,1000"  (contract call)
    ///   --tx "0xabc::0x123def"  (raw data)
    #[arg(long = "tx", value_name = "SPEC", value_delimiter = ',')]
    pub transactions: Vec<String>,

    #[command(flatten)]
    pub tx: TransactionOpts,

    #[command(flatten)]
    pub eth: EthereumOpts,

    /// Generate raw RLP-encoded unsigned transactions.
    #[arg(long)]
    pub raw_unsigned: bool,

    /// Call `eth_signTransaction` using the `--from` argument or $ETH_FROM as sender
    #[arg(long, requires = "from", conflicts_with = "raw_unsigned")]
    pub ethsign: bool,

    /// Starting nonce (auto-detected if not provided)
    #[arg(long)]
    pub start_nonce: Option<u64>,
}

/// Parsed transaction specification
#[derive(Debug, Clone)]
pub struct TxSpec {
    pub to: String,
    pub value: Option<String>,
    pub sig: Option<String>,
    pub args: Vec<String>,
}

impl TxSpec {
    /// Parse transaction spec in format: to[:value][:sig[:args]]
    fn parse(spec: &str) -> Result<Self> {
        let parts: Vec<&str> = spec.split(':').collect();

        if parts.is_empty() {
            return Err(eyre!("Empty transaction specification"));
        }

        let to = parts[0].to_string();
        if to.is_empty() {
            return Err(eyre!("Missing destination address"));
        }

        let mut value = None;
        let mut sig = None;
        let mut args = Vec::new();

        match parts.len() {
            1 => {
                // Just address: "0x123"
            }
            2 => {
                // Address + value OR raw data: "0x123:0.1ether" or "0x123:0x123abc"
                let second = parts[1];
                if second.starts_with("0x") && second.len() > 10 {
                    // Looks like raw data
                    sig = Some(second.to_string());
                } else if !second.is_empty() {
                    // Looks like value
                    value = Some(second.to_string());
                }
            }
            3 => {
                // Address + value + sig: "0x123:0.1ether:transfer(address,uint256)"
                // OR Address + empty + sig: "0x123::transfer(address,uint256)"
                if !parts[1].is_empty() {
                    value = Some(parts[1].to_string());
                }
                if !parts[2].is_empty() {
                    sig = Some(parts[2].to_string());
                }
            }
            4 => {
                // Address + value + sig + args:
                // "0x123:0.1ether:transfer(address,uint256):0x789,1000"
                if !parts[1].is_empty() {
                    value = Some(parts[1].to_string());
                }
                if !parts[2].is_empty() {
                    sig = Some(parts[2].to_string());
                }
                if !parts[3].is_empty() {
                    args = parts[3].split(',').map(|s| s.trim().to_string()).collect();
                }
            }
            _ => {
                return Err(eyre!(
                    "Invalid transaction specification format. Expected: to[:value][:sig[:args]]"
                ));
            }
        }

        Ok(Self { to, value, sig, args })
    }
}

impl BatchMakeTxArgs {
    pub async fn run(self) -> Result<()> {
        let Self { transactions, tx, eth, raw_unsigned, ethsign, start_nonce } = self;

        if transactions.is_empty() {
            return Err(eyre!("No transactions specified. Use --tx flag to specify transactions."));
        }

        sh_println!("Building {} transactions...", transactions.len())?;

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

        // Get sender address
        let sender_addr = if raw_unsigned && eth.wallet.from.is_none() {
            return Err(eyre!("--raw-unsigned requires --from when start-nonce is not provided"));
        } else if eth.wallet.from.is_some() {
            eth.wallet.from.unwrap()
        } else if !raw_unsigned {
            eth.wallet.signer().await?.address()
        } else {
            alloy_primitives::Address::ZERO
        };

        // Get starting nonce
        let mut current_nonce = if let Some(nonce) = start_nonce {
            nonce
        } else if raw_unsigned && eth.wallet.from.is_none() {
            return Err(eyre!("--raw-unsigned requires either --from or --start-nonce"));
        } else {
            provider.get_transaction_count(sender_addr).await?
        };

        sh_println!("Starting nonce: {}", current_nonce)?;

        let mut results = Vec::new();

        // Process each transaction
        for (i, tx_spec) in tx_specs.iter().enumerate() {
            sh_println!(
                "Building transaction {} of {} (nonce: {})...",
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

            // Build transaction based on mode
            let tx_result = if raw_unsigned {
                // Build unsigned raw tx
                let raw_tx = builder.build_unsigned_raw(sender_addr).await?;
                format!("0x{raw_tx}")
            } else if ethsign {
                // Use eth_signTransaction
                let (tx, _) = builder.build(config.sender).await?;
                let signed_tx: Bytes = provider.sign_transaction(tx.into_inner()).await?;
                format!("0x{signed_tx}")
            } else {
                // Use local signer
                let signer = eth.wallet.signer().await?;
                let (tx, _) = builder.build(&signer).await?;
                let tx = tx.into_inner().build(&EthereumWallet::new(signer)).await?;
                format!("0x{}", hex::encode(tx.encoded_2718()))
            };

            sh_println!("Transaction {} built", i + 1)?;
            results.push(tx_result);
            current_nonce += 1;
        }

        // Print all results
        sh_println!("Batch complete! {} transactions built:", results.len())?;
        for (i, tx_hex) in results.iter().enumerate() {
            sh_println!("{}. {}", i + 1, tx_hex)?;
        }

        Ok(())
    }
}
