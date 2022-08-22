// cast send subcommands
use crate::{
    opts::{cast::parse_name_or_address, EthereumOpts, TransactionOpts, WalletType},
    utils::parse_ether_value,
};
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::{
    providers::Middleware,
    types::{Address, NameOrAddress, U256},
};
use foundry_common::get_http_provider;
use foundry_config::{Chain, Config};
use std::{str::FromStr, sync::Arc};

#[derive(Debug, Parser)]
pub struct SendTxArgs {
    #[clap(
            help = "The destination of the transaction.",
            parse(try_from_str = parse_name_or_address),
            value_name = "TO"
        )]
    to: NameOrAddress,
    #[clap(help = "The signature of the function to call.", value_name = "SIG")]
    sig: Option<String>,
    #[clap(help = "The arguments of the function to call.", value_name = "ARGS")]
    args: Vec<String>,
    #[clap(
        long = "async",
        env = "CAST_ASYNC",
        name = "async",
        alias = "cast-async",
        help = "Only print the transaction hash and exit immediately."
    )]
    cast_async: bool,
    #[clap(flatten, next_help_heading = "TRANSACTION OPTIONS")]
    tx: TransactionOpts,
    #[clap(flatten, next_help_heading = "ETHEREUM OPTIONS")]
    eth: EthereumOpts,
    #[clap(
        short,
        long,
        help = "The number of confirmations until the receipt is fetched.",
        default_value = "1",
        value_name = "CONFIRMATIONS"
    )]
    confirmations: usize,
    #[clap(long = "json", short = 'j', help_heading = "DISPLAY OPTIONS")]
    to_json: bool,
    #[clap(
        long = "resend",
        help = "Reuse the latest nonce for the sender account.",
        conflicts_with = "nonce"
    )]
    resend: bool,
}

impl SendTxArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let SendTxArgs { eth, to, sig, cast_async, args, mut tx, confirmations, to_json, resend } =
            self;
        let config = Config::from(&eth);
        let provider = Arc::new(get_http_provider(
            &config.eth_rpc_url.unwrap_or_else(|| "http://localhost:8545".to_string()),
        ));
        let chain: Chain =
            if let Some(chain) = eth.chain { chain } else { provider.get_chainid().await?.into() };
        let sig = sig.unwrap_or_default();

        if let Ok(Some(signer)) = eth.signer_with(chain.into(), provider.clone()).await {
            let from = match &signer {
                WalletType::Ledger(leger) => leger.address(),
                WalletType::Local(local) => local.address(),
                WalletType::Trezor(trezor) => trezor.address(),
            };

            // prevent misconfigured hwlib from sending a transaction that defies
            // user-specified --from
            if let Some(specified_from) = eth.wallet.from {
                if specified_from != from {
                    eyre::bail!("The specified sender via CLI/env vars does not match the sender configured via the hardware wallet's HD Path. Please use the `--hd-path <PATH>` parameter to specify the BIP32 Path which corresponds to the sender. This will be automatically detected in the future: https://github.com/foundry-rs/foundry/issues/2289")
                }
            }

            if resend {
                tx.nonce = Some(provider.get_transaction_count(from, None).await?);
            }

            match signer {
                WalletType::Ledger(signer) => {
                    cast_send(
                        &signer,
                        from,
                        to,
                        (sig, args),
                        tx.gas_limit,
                        tx.gas_price,
                        tx.priority_gas_price,
                        tx.value,
                        tx.nonce,
                        chain,
                        config.etherscan_api_key,
                        cast_async,
                        tx.legacy,
                        confirmations,
                        to_json,
                    )
                    .await?;
                }
                WalletType::Local(signer) => {
                    cast_send(
                        &signer,
                        from,
                        to,
                        (sig, args),
                        tx.gas_limit,
                        tx.gas_price,
                        tx.priority_gas_price,
                        tx.value,
                        tx.nonce,
                        chain,
                        config.etherscan_api_key,
                        cast_async,
                        tx.legacy,
                        confirmations,
                        to_json,
                    )
                    .await?;
                }
                WalletType::Trezor(signer) => {
                    cast_send(
                        &signer,
                        from,
                        to,
                        (sig, args),
                        tx.gas_limit,
                        tx.gas_price,
                        tx.priority_gas_price,
                        tx.value,
                        tx.nonce,
                        chain,
                        config.etherscan_api_key,
                        cast_async,
                        tx.legacy,
                        confirmations,
                        to_json,
                    )
                    .await?;
                }
            } // Checking if signer isn't the default value
              // 00a329c0648769A73afAc7F9381E08FB43dBEA72.
        } else if config.sender !=
            Address::from_str("00a329c0648769A73afAc7F9381E08FB43dBEA72").unwrap()
        {
            if resend {
                tx.nonce = Some(provider.get_transaction_count(config.sender, None).await?);
            }

            cast_send(
                provider,
                config.sender,
                to,
                (sig, args),
                tx.gas_limit,
                tx.gas_price,
                tx.priority_gas_price,
                tx.value,
                tx.nonce,
                chain,
                config.etherscan_api_key,
                cast_async,
                tx.legacy,
                confirmations,
                to_json,
            )
            .await?;
        } else {
            eyre::bail!("No wallet or sender address provided. Consider passing it via the --from flag or setting the ETH_FROM env variable or setting in the foundry.toml file");
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn cast_send<M: Middleware, F: Into<NameOrAddress>, T: Into<NameOrAddress>>(
    provider: M,
    from: F,
    to: T,
    args: (String, Vec<String>),
    gas: Option<U256>,
    gas_price: Option<U256>,
    priority_gas_price: Option<U256>,
    value: Option<U256>,
    nonce: Option<U256>,
    chain: Chain,
    etherscan_api_key: Option<String>,
    cast_async: bool,
    legacy: bool,
    confs: usize,
    to_json: bool,
) -> eyre::Result<()>
where
    M::Error: 'static,
{
    let sig = args.0;
    let params = args.1;
    let params = if !sig.is_empty() { Some((&sig[..], params)) } else { None };
    let mut builder = TxBuilder::new(&provider, from, Some(to), chain, legacy).await?;
    builder
        .etherscan_api_key(etherscan_api_key)
        .args(params)
        .await?
        .gas(gas)
        .gas_price(gas_price)
        .priority_gas_price(priority_gas_price)
        .value(value)
        .nonce(nonce);
    let builder_output = builder.build();

    let cast = Cast::new(provider);

    let pending_tx = cast.send(builder_output).await?;
    let tx_hash = *pending_tx;

    if cast_async {
        println!("{:#x}", tx_hash);
    } else {
        let receipt = cast.receipt(format!("{:#x}", tx_hash), None, confs, false, to_json).await?;
        println!("{receipt}");
    }

    Ok(())
}
