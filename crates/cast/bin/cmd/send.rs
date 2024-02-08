use crate::tx;
use cast::Cast;
use clap::Parser;
use ethers_core::types::NameOrAddress;
use ethers_middleware::MiddlewareBuilder;
use ethers_providers::Middleware;
use ethers_signers::Signer;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils,
};
use foundry_common::{
    cli_warn,
    types::{ToAlloy, ToEthers},
};
use foundry_config::{Chain, Config};
use std::str::FromStr;

/// CLI arguments for `cast send`.
#[derive(Debug, Parser)]
pub struct SendTxArgs {
    /// The destination of the transaction.
    ///
    /// If not provided, you must use cast send --create.
    #[clap(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    args: Vec<String>,

    /// Only print the transaction hash and exit immediately.
    #[clap(name = "async", long = "async", alias = "cast-async", env = "CAST_ASYNC")]
    cast_async: bool,

    /// The number of confirmations until the receipt is fetched.
    #[clap(long, default_value = "1")]
    confirmations: usize,

    /// Print the transaction receipt as JSON.
    #[clap(long, short, help_heading = "Display options")]
    json: bool,

    /// Reuse the latest nonce for the sender account.
    #[clap(long, conflicts_with = "nonce")]
    resend: bool,

    #[clap(subcommand)]
    command: Option<SendTxSubcommands>,

    /// Send via `eth_sendTransaction using the `--from` argument or $ETH_FROM as sender
    #[clap(long, requires = "from")]
    unlocked: bool,

    #[clap(flatten)]
    tx: TransactionOpts,

    #[clap(flatten)]
    eth: EthereumOpts,
}

#[derive(Debug, Parser)]
pub enum SendTxSubcommands {
    /// Use to deploy raw contract bytecode.
    #[clap(name = "--create")]
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

        // Case 1:
        // Default to sending via eth_sendTransaction if the --unlocked flag is passed.
        // This should be the only way this RPC method is used as it requires a local node
        // or remote RPC with unlocked accounts.
        if unlocked {
            // only check current chain id if it was specified in the config
            if let Some(config_chain) = config.chain {
                let current_chain_id = provider.get_chainid().await?.as_u64();
                let config_chain_id = config_chain.id();
                // switch chain if current chain id is not the same as the one specified in the
                // config
                if config_chain_id != current_chain_id {
                    cli_warn!("Switching to chain {}", config_chain);
                    provider
                        .request(
                            "wallet_switchEthereumChain",
                            [serde_json::json!({
                                "chainId": format!("0x{:x}", config_chain_id),
                            })],
                        )
                        .await?;
                }
            }

            if resend {
                tx.nonce = Some(
                    provider
                        .get_transaction_count(config.sender.to_ethers(), None)
                        .await?
                        .to_alloy(),
                );
            }

            cast_send(
                provider,
                config.sender.to_ethers(),
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
            )
            .await
        // Case 2:
        // An option to use a local signer was provided.
        // If we cannot successfully instantiate a local signer, then we will assume we don't have
        // enough information to sign and we must bail.
        } else {
            // Retrieve the signer, and bail if it can't be constructed.
            let signer = eth.wallet.signer(chain.id()).await?;
            let from = signer.address();

            tx::validate_from_address(eth.wallet.from, from.to_alloy())?;

            if resend {
                tx.nonce = Some(provider.get_transaction_count(from, None).await?.to_alloy());
            }

            let provider = provider.with_signer(signer);

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
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn cast_send<M: Middleware, F: Into<NameOrAddress>, T: Into<NameOrAddress>>(
    provider: M,
    from: F,
    to: Option<T>,
    code: Option<String>,
    sig: Option<String>,
    args: Vec<String>,
    tx: TransactionOpts,
    chain: Chain,
    etherscan_api_key: Option<String>,
    cast_async: bool,
    confs: usize,
    to_json: bool,
) -> Result<()>
where
    M::Error: 'static,
{
    let builder_output =
        tx::build_tx(&provider, from, to, code, sig, args, tx, chain, etherscan_api_key).await?;

    let cast = Cast::new(provider);

    let pending_tx = cast.send(builder_output).await?;
    let tx_hash = *pending_tx;

    if cast_async {
        println!("{tx_hash:#x}");
    } else {
        let receipt = cast.receipt(format!("{tx_hash:#x}"), None, confs, false, to_json).await?;
        println!("{receipt}");
    }

    Ok(())
}
