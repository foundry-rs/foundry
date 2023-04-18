// cast send subcommands
use crate::{
    opts::{EthereumOpts, TransactionOpts},
    utils,
};
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::{
    prelude::MiddlewareBuilder, providers::Middleware, signers::Signer, types::NameOrAddress,
};
use foundry_config::{Chain, Config};
use std::str::FromStr;

/// CLI arguments for `cast send`.
#[derive(Debug, Parser)]
pub struct SendTxArgs {
    /// The destination of the transaction.
    ///
    /// If not provided, you must use cast send --create.
    #[clap(
        value_parser = NameOrAddress::from_str,
        value_name = "TO",
    )]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    args: Vec<String>,

    /// Only print the transaction hash and exit immediately.
    #[clap(name = "async", long = "async", alias = "cast-async", env = "CAST_ASYNC")]
    cast_async: bool,

    #[clap(flatten)]
    tx: TransactionOpts,

    #[clap(flatten)]
    eth: EthereumOpts,

    /// The number of confirmations until the receipt is fetched.
    #[clap(long, default_value = "1", value_name = "CONFIRMATIONS")]
    confirmations: usize,

    /// Print the transaction receipt as JSON.
    #[clap(long, short, help_heading = "Display options")]
    json: bool,

    /// Reuse the latest nonce for the sender account.
    #[clap(long, conflicts_with = "nonce")]
    resend: bool,

    #[clap(subcommand)]
    command: Option<SendTxSubcommands>,
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
    pub async fn run(self) -> eyre::Result<()> {
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
        } = self;
        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;
        let chain = utils::get_chain(config.chain_id, &provider).await?;
        let api_key = config.get_etherscan_api_key(Some(chain));
        let mut sig = sig.unwrap_or_default();

        if let Ok(signer) = eth.wallet.signer(chain.id()).await {
            let from = signer.address();

            // prevent misconfigured hwlib from sending a transaction that defies
            // user-specified --from
            if let Some(specified_from) = eth.wallet.from {
                if specified_from != from {
                    eyre::bail!("\
                        The specified sender via CLI/env vars does not match the sender configured via\n\
                        the hardware wallet's HD Path.\n\
                        Please use the `--hd-path <PATH>` parameter to specify the BIP32 Path which\n\
                        corresponds to the sender.\n\
                        This will be automatically detected in the future: https://github.com/foundry-rs/foundry/issues/2289\
                    ")
                }
            }

            if resend {
                tx.nonce = Some(provider.get_transaction_count(from, None).await?);
            }

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
        } else if config.sender != Config::DEFAULT_SENDER {
            // Checking if signer isn't the default value
            // 00a329c0648769A73afAc7F9381E08FB43dBEA72.
            if resend {
                tx.nonce = Some(provider.get_transaction_count(config.sender, None).await?);
            }

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
        } else {
            Err(eyre::eyre!(
                "\
                    No wallet or sender address provided. Consider passing it via the --from flag or\n\
                    setting the ETH_FROM env variable or setting in foundry.toml\
                "
            ))
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
) -> eyre::Result<()>
where
    M::Error: 'static,
{
    let (sig, params) = args;
    let params = if !sig.is_empty() { Some((&sig[..], params)) } else { None };
    let mut builder = TxBuilder::new(&provider, from, to, chain, tx.legacy).await?;
    builder
        .etherscan_api_key(etherscan_api_key)
        .gas(tx.gas_limit)
        .gas_price(tx.gas_price)
        .priority_gas_price(tx.priority_gas_price)
        .value(tx.value)
        .nonce(tx.nonce);

    if let Some(code) = code {
        let mut data = hex::decode(code.strip_prefix("0x").unwrap_or(&code))?;

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
