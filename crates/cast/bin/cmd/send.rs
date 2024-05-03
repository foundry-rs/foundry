use crate::tx;
use alloy_network::{AnyNetwork, EthereumSigner};
use alloy_primitives::{Address, U64};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::BlockId;
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
use foundry_config::{Chain, Config};
use std::str::FromStr;

/// CLI arguments for `cast send`.
#[derive(Debug, Parser)]
pub struct SendTxArgs {
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

    /// Reuse the latest nonce for the sender account.
    #[arg(long, conflicts_with = "nonce")]
    resend: bool,

    #[command(subcommand)]
    command: Option<SendTxSubcommands>,

    /// Send via `eth_sendTransaction using the `--from` argument or $ETH_FROM as sender
    #[arg(long, requires = "from")]
    unlocked: bool,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    eth: EthereumOpts,
}

#[derive(Debug, Parser)]
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

impl SendTxArgs {
    pub async fn run(self) -> Result<()> {
        let SendTxArgs {
            eth,
            to,
            mut sig,
            cast_async,
            mut args,
            mut tx,
            confirmations,
            json: to_json,
            resend,
            command,
            unlocked,
        } = self;

        if tx.legacy && tx.blob {
            eyre::bail!("Cannot send a legacy transaction with a blob");
        }
        let blob_data = if tx.blob { Some("blob data".to_string()) } else { None };

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

        tx::validate_to_address(&code, &to)?;

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;
        let chain = utils::get_chain(config.chain, &provider).await?;
        let api_key = config.get_etherscan_api_key(Some(chain));

        let to = match to {
            Some(to) => Some(to.resolve(&provider).await?),
            None => None,
        };

        // Case 1:
        // Default to sending via eth_sendTransaction if the --unlocked flag is passed.
        // This should be the only way this RPC method is used as it requires a local node
        // or remote RPC with unlocked accounts.
        if unlocked {
            // only check current chain id if it was specified in the config
            if let Some(config_chain) = config.chain {
                let current_chain_id = provider.get_chain_id().await?;
                let config_chain_id = config_chain.id();
                // switch chain if current chain id is not the same as the one specified in the
                // config
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

            if resend {
                tx.nonce = Some(U64::from(
                    provider.get_transaction_count(config.sender, BlockId::latest()).await?,
                ));
            }

            cast_send(
                provider,
                config.sender,
                to,
                code,
                sig,
                args,
                tx,
                chain,
                api_key,
                cast_async,
                confirmations,
                to_json,
                blob_data,
            )
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

            if resend {
                tx.nonce =
                    Some(U64::from(provider.get_transaction_count(from, BlockId::latest()).await?));
            }

            let signer = EthereumSigner::from(signer);
            let provider =
                ProviderBuilder::<_, _, AnyNetwork>::default().signer(signer).on_provider(provider);

            cast_send(
                provider,
                from,
                to,
                code,
                sig,
                args,
                tx,
                chain,
                api_key,
                cast_async,
                confirmations,
                to_json,
                blob_data,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn cast_send<P: Provider<T, AnyNetwork>, T: Transport + Clone>(
    provider: P,
    from: Address,
    to: Option<Address>,
    code: Option<String>,
    sig: Option<String>,
    args: Vec<String>,
    tx: TransactionOpts,
    chain: Chain,
    etherscan_api_key: Option<String>,
    cast_async: bool,
    confs: u64,
    to_json: bool,
    blob_data: Option<String>,
) -> Result<()> {
    let (tx, _) =
        tx::build_tx(&provider, from, to, code, sig, args, tx, chain, etherscan_api_key, blob_data)
            .await?;

    let cast = Cast::new(provider);
    let pending_tx = cast.send(tx).await?;

    let tx_hash = pending_tx.inner().tx_hash();

    if cast_async {
        println!("{tx_hash:#x}");
    } else {
        let receipt = cast.receipt(format!("{tx_hash:#x}"), None, confs, false, to_json).await?;
        println!("{receipt}");
    }

    Ok(())
}
