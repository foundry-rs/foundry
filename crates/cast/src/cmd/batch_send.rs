use crate::tx::{CastTxBuilder, CastTxSender, SendTxOpts};
use alloy_ens::NameOrAddress;
use alloy_primitives::{U64, utils::parse_ether};
use alloy_provider::Provider;
use alloy_signer::Signer;
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{opts::TransactionOpts, utils::LoadConfig};
use std::str::FromStr;

/// CLI arguments for `cast batch-send`.
#[derive(Debug, Parser)]
pub struct BatchSendArgs {
    /// Transaction specifications in format: to\[:value\]\[:sig\[:args\]\]
    ///
    /// Examples:
    ///   --tx "0x123:0.1ether"  (ETH transfer)
    ///   --tx "0x456::transfer(address,uint256):0x789,1000"  (contract call)
    ///   --tx "0xabc::0x123def"  (raw data)
    #[arg(long = "tx", value_name = "SPEC", value_delimiter = ',')]
    pub transactions: Vec<String>,

    #[command(flatten)]
    pub send_tx: SendTxOpts,

    #[command(flatten)]
    pub tx: TransactionOpts,

    /// Send via `eth_sendTransaction` using the `--from` argument or $ETH_FROM as sender
    #[arg(long, requires = "from")]
    pub unlocked: bool,

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
    /// Parse transaction spec in format: to\\[:value\\]\\[:sig\\[:args\\]\\]
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

impl BatchSendArgs {
    pub async fn run(self) -> Result<()> {
        let Self { transactions, send_tx, tx, unlocked, start_nonce } = self;

        if transactions.is_empty() {
            return Err(eyre!("No transactions specified. Use --tx flag to specify transactions."));
        }

        sh_println!("Processing {} transactions...", transactions.len())?;

        // Parse all transaction specs
        let tx_specs: Result<Vec<TxSpec>> = transactions
            .iter()
            .enumerate()
            .map(|(i, spec)| TxSpec::parse(spec).map_err(|e| eyre!("Transaction {}: {}", i + 1, e)))
            .collect();
        let tx_specs = tx_specs?;

        // Set up provider and config
        let config = send_tx.eth.load_config()?;
        let provider = foundry_cli::utils::get_provider(&config)?;

        // Get sender address
        let sender_addr =
            if unlocked { config.sender } else { send_tx.eth.wallet.signer().await?.address() };

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

            // Send transaction
            let tx_hash = if unlocked {
                // Use unlocked account
                let (tx, _) = builder.build(sender_addr).await?;
                let cast = CastTxSender::new(&provider);
                let pending_tx = cast.send(tx.into_inner()).await?;
                *pending_tx.inner().tx_hash()
            } else {
                // Use signer
                let signer = send_tx.eth.wallet.signer().await?;
                let (tx_request, _) = builder.build(&signer).await?;

                // Create provider with wallet
                use alloy_network::{AnyNetwork, EthereumWallet};
                use alloy_provider::ProviderBuilder;

                let wallet = EthereumWallet::from(signer);
                let provider_with_wallet = ProviderBuilder::<_, _, AnyNetwork>::default()
                    .wallet(wallet)
                    .connect_provider(&provider);

                let cast = CastTxSender::new(&provider_with_wallet);
                let pending_tx = cast.send(tx_request.into_inner()).await?;
                *pending_tx.inner().tx_hash()
            };

            sh_println!("Transaction {} sent: {:#x}", i + 1, tx_hash)?;
            results.push(tx_hash);
            current_nonce += 1;
        }

        // Print summary
        sh_println!("Batch complete! {} transactions sent:", results.len())?;
        for (i, hash) in results.iter().enumerate() {
            sh_println!("  {}. {:#x}", i + 1, hash)?;
        }

        if !send_tx.cast_async {
            sh_println!("Waiting for receipts...")?;

            let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
            for (i, hash) in results.iter().enumerate() {
                let cast = CastTxSender::new(&provider);
                let receipt = cast
                    .receipt(
                        format!("{hash:#x}"),
                        None,
                        send_tx.confirmations,
                        Some(timeout),
                        false,
                    )
                    .await?;

                sh_println!("Receipt for transaction {}: {}", i + 1, receipt)?;
            }
        }

        Ok(())
    }
}
