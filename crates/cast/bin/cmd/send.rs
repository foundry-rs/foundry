use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::{
    prelude::MiddlewareBuilder, providers::Middleware, signers::Signer, types::NameOrAddress,
};
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils,
};
use foundry_common::cli_warn;
use foundry_config::{Chain, Config};
use foundry_utils::types::ToAlloy;
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
            sig,
            cast_async,
            mut args,
            mut tx,
            confirmations,
            json: to_json,
            resend,
            command,
            unlocked,
        } = self;
        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;
        let chain = utils::get_chain(config.chain_id, &provider).await?;
        let api_key = config.get_etherscan_api_key(Some(chain));
        let mut sig = sig.unwrap_or_default();

        let code = if let Some(SendTxSubcommands::Create {
            code,
            sig: constructor_sig,
            args: constructor_args,
        }) = command
        {
            sig = constructor_sig.unwrap_or_default();
            args = constructor_args;
            Some(code)
        } else {
            None
        };

        // Case 1:
        // Default to sending via eth_sendTransaction if the --unlocked flag is passed.
        // This should be the only way this RPC method is used as it requires a local node
        // or remote RPC with unlocked accounts.
        if unlocked {
            // only check current chain id if it was specified in the config
            if let Some(config_chain) = config.chain_id {
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
                tx.nonce = Some(provider.get_transaction_count(config.sender, None).await?);
            }

            cast_send(
                provider,
                config.sender,
                to,
                code,
                (sig, args),
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

            // prevent misconfigured hwlib from sending a transaction that defies
            // user-specified --from
            if let Some(specified_from) = eth.wallet.from {
                if specified_from != from {
                    eyre::bail!(
                        "\
The specified sender via CLI/env vars does not match the sender configured via
the hardware wallet's HD Path.
Please use the `--hd-path <PATH>` parameter to specify the BIP32 Path which
corresponds to the sender, or let foundry automatically detect it by not specifying any sender address."
                    )
                }
            }

            if resend {
                tx.nonce = Some(provider.get_transaction_count(from, None).await?);
            }

            let provider = provider.with_signer(signer);

            cast_send(
                provider,
                from,
                to,
                code,
                (sig, args),
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
    args: (String, Vec<String>),
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
    let (sig, params) = args;
    let params = if !sig.is_empty() { Some((&sig[..], params)) } else { None };
    let mut builder = TxBuilder::new(&provider, from, to, chain, tx.legacy).await?;
    builder
        .etherscan_api_key(etherscan_api_key)
        .gas(tx.gas_limit.map(|g| g.to_alloy()))
        .gas_price(tx.gas_price.map(|g| g.to_alloy()))
        .priority_gas_price(tx.priority_gas_price.map(|p| p.to_alloy()))
        .value(tx.value.map(|v| v.to_alloy()))
        .nonce(tx.nonce.map(|n| n.to_alloy()));

    if let Some(code) = code {
        let mut data = hex::decode(code)?;

        if let Some((sig, args)) = params {
            let (mut sigdata, _) = builder.create_args(sig, args).await?;
            data.append(&mut sigdata);
        }

        builder.set_data(data);
    } else {
        builder.args(params).await?;
    };
    let builder_output = builder.build();

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
