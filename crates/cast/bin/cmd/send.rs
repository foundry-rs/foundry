use crate::tx::{self, CastTxBuilder};
use alloy_network::{AnyNetwork, EthereumWallet};
use alloy_primitives::U256;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use alloy_signer::Signer;
use alloy_transport::Transport;
use cast::Cast;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils,
};
use foundry_common::{cli_warn, ens::NameOrAddress};
use foundry_config::Config;
use std::{path::PathBuf, str::FromStr};

/// CLI arguments for `cast send`.
#[derive(Debug, Parser)]
pub struct SendTxArgs {
    #[command(flatten)]
    eth: EthereumOpts,

    /// The destination of the transaction.
    ///
    /// If not provided, you must use cast send --create.
    #[arg(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    args: Vec<String>,

    /// Only print the transaction hash and exit immediately.
    #[arg(id = "async", long = "async", alias = "cast-async", env = "CAST_ASYNC")]
    cast_async: bool,

    /// The number of confirmations until the receipt is fetched.
    #[arg(long, default_value = "1")]
    confirmations: u64,

    /// Print the transaction receipt as JSON.
    #[arg(long, short, help_heading = "Display options")]
    json: bool,

    #[command(subcommand)]
    command: Option<SendTxSubcommands>,

    /// Send via `eth_sendTransaction using the `--from` argument or $ETH_FROM as sender
    #[arg(long, requires = "from")]
    unlocked: bool,

    /// Timeout for sending the transaction.
    #[arg(long, env = "ETH_TIMEOUT")]
    pub timeout: Option<u64>,

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
    bump_gas_price: BumpGasPriceArgs,
}

#[derive(Clone, Debug, Parser)]
pub enum SendTxSubcommands {
    /// Use to deploy raw contract bytecode.
    #[command(name = "--create")]
    Create {
        /// The bytecode of the contract to deploy.
        code: String,

        /// The signature of the function to call.
        sig: Option<String>,

        /// The arguments of the function to call.
        args: Vec<String>,
    },
}

#[derive(Debug, Parser)]
#[command(next_help_heading = "Bump gas price options")]
struct BumpGasPriceArgs {
    /// Enable automatic gas price escalation for transactions.
    ///
    /// When set to true, automatically increase the gas price of a pending transaction. It can be
    /// used to replace transactions that are stuck during busy network times.
    #[arg(long, alias = "bump-fee")]
    auto_bump_gas_price: bool,

    // The percentage by which to increase the gas price on each retry.
    #[arg(long, default_value = "10")]
    gas_price_increment_percentage: u64,

    /// The maximum total percentage increase allowed for gas price.
    ///
    /// This sets an upper limit on the gas price across all retry attempts, expressed as a
    /// percentage of the original price. For example, a value of 150 means the gas price will
    /// never exceed 150% of the original price (1.5 times the initial price).
    #[arg(long, default_value = "150")]
    gas_price_bump_limit_percentage: u64,

    /// The maximum number of times to bump the gas price for a transaction.
    #[arg(long, default_value = "3")]
    max_gas_price_bumps: u64,
}

impl SendTxArgs {
    #[allow(unknown_lints, dependency_on_unit_never_type_fallback)]
    pub async fn run(self) -> Result<(), eyre::Report> {
        let Self {
            eth,
            to,
            sig,
            args,
            cast_async,
            confirmations,
            json: to_json,
            command,
            unlocked,
            timeout,
            tx,
            path,
            bump_gas_price,
        } = self;

        const INITIAL_BASE_FEE: u64 = 1000000000;
        let initial_gas_price = tx.gas_price.unwrap_or(U256::from(INITIAL_BASE_FEE));

        let bump_amount = initial_gas_price
            .saturating_mul(U256::from(bump_gas_price.gas_price_increment_percentage))
            .wrapping_div(U256::from(100));

        let gas_price_limit = initial_gas_price
            .saturating_mul(U256::from(bump_gas_price.gas_price_bump_limit_percentage))
            .wrapping_div(U256::from(100));

        let mut current_gas_price = initial_gas_price;
        let mut retry_count = 0;
        loop {
            let mut new_tx = tx.clone();
            new_tx.gas_price = Some(current_gas_price);

            match prepare_and_send_transaction(
                eth.clone(),
                to.clone(),
                sig.clone(),
                args.clone(),
                cast_async,
                confirmations,
                to_json,
                command.clone(),
                unlocked,
                timeout,
                new_tx,
                path.clone(),
            )
            .await
            {
                Ok(_) => return Ok(()),
                Err(err) => {
                    let is_underpriced =
                        err.to_string().contains("replacement transaction underpriced");
                    let is_already_imported =
                        err.to_string().contains("transaction already imported");

                    if bump_gas_price.auto_bump_gas_price && (is_underpriced || is_already_imported)
                    {
                        if !to_json {
                            if is_underpriced {
                                println!("Error: transaction underpriced.");
                            } else if is_already_imported {
                                println!("Error: transaction already imported.");
                            }
                        }

                        retry_count += 1;
                        if retry_count > bump_gas_price.max_gas_price_bumps {
                            return Err(eyre::eyre!(
                                "Max gas price bump attempts reached. Transaction still stuck."
                            ));
                        }

                        let old_gas_price = current_gas_price;
                        current_gas_price =
                            initial_gas_price + (bump_amount * U256::from(retry_count));

                        if current_gas_price >= gas_price_limit {
                            return Err(eyre::eyre!("Unable to bump more the gas price. Hit the limit of {}% of the original price ({} wei)",
                                bump_gas_price.gas_price_bump_limit_percentage,
                                gas_price_limit
                            ));
                        }

                        if !to_json {
                            println!();
                            println!(
                                "Retrying with a {}% gas price increase (attempt {}/{}).",
                                bump_gas_price.gas_price_increment_percentage,
                                retry_count,
                                bump_gas_price.max_gas_price_bumps
                            );
                            println!("- Old gas price: {old_gas_price} wei");
                            println!("- New gas price: {current_gas_price} wei");
                        }
                        continue;
                    }

                    return Err(err);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments, dependency_on_unit_never_type_fallback)]
async fn prepare_and_send_transaction(
    eth: EthereumOpts,
    to: Option<NameOrAddress>,
    mut sig: Option<String>,
    mut args: Vec<String>,
    cast_async: bool,
    confirmations: u64,
    to_json: bool,
    command: Option<SendTxSubcommands>,
    unlocked: bool,
    timeout: Option<u64>,
    tx: TransactionOpts,
    path: Option<PathBuf>,
) -> Result<()> {
    let blob_data = if let Some(path) = path { Some(std::fs::read(path)?) } else { None };

    let code = if let Some(SendTxSubcommands::Create {
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

    let config = Config::from(&eth);
    let provider = utils::get_provider(&config)?;

    let builder = CastTxBuilder::new(&provider, tx, &config)
        .await?
        .with_to(to)
        .await?
        .with_code_sig_and_args(code, sig, args)
        .await?
        .with_blob_data(blob_data)?;

    let timeout = timeout.unwrap_or(config.transaction_timeout);

    // Case 1:
    // Default to sending via eth_sendTransaction if the --unlocked flag is passed.
    // This should be the only way this RPC method is used as it requires a local node
    // or remote RPC with unlocked accounts.
    if unlocked {
        // Only check current chain id if it was specified in the config.
        if let Some(config_chain) = config.chain {
            let current_chain_id = provider.get_chain_id().await?;
            let config_chain_id = config_chain.id();
            // Switch chain if current chain id is not the same as the one specified in the
            // config.
            if config_chain_id != current_chain_id {
                cli_warn!("Switching to chain {}", config_chain);
                provider
                    .raw_request(
                        "wallet_switchEthereumChain".into(),
                        [serde_json::json!({
                            "chainId": format!("0x{:x}", config_chain_id),
                        })],
                    )
                    .await?;
            }
        }

        let (tx, _) = builder.build(config.sender).await?;

        send_and_monitor_transaction(provider, tx, cast_async, confirmations, timeout, to_json)
            .await
    // Case 2:
    // An option to use a local signer was provided.
    // If we cannot successfully instantiate a local signer, then we will assume we don't have
    // enough information to sign and we must bail.
    } else {
        // Retrieve the signer, and bail if it can't be constructed.
        let signer = eth.wallet.signer().await?;
        let from = signer.address();

        tx::validate_from_address(eth.wallet.from, from)?;

        let (tx, _) = builder.build(&signer).await?;

        let wallet = EthereumWallet::from(signer);
        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().wallet(wallet).on_provider(&provider);

        send_and_monitor_transaction(provider, tx, cast_async, confirmations, timeout, to_json)
            .await
    }
}

async fn send_and_monitor_transaction<P: Provider<T, AnyNetwork>, T: Transport + Clone>(
    provider: P,
    tx: WithOtherFields<TransactionRequest>,
    cast_async: bool,
    confs: u64,
    timeout: u64,
    to_json: bool,
) -> Result<()> {
    let cast = Cast::new(provider);
    let pending_tx = cast.send(tx).await?;

    let tx_hash = pending_tx.inner().tx_hash();

    if cast_async {
        println!("{tx_hash:#x}");
    } else {
        let receipt = cast
            .receipt(format!("{tx_hash:#x}"), None, confs, Some(timeout), false, to_json)
            .await?;
        println!("{receipt}");
    }

    Ok(())
}
