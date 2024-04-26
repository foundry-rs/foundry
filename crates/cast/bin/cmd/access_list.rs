use alloy_network::{AnyNetwork, TransactionBuilder};
use alloy_primitives::{Address, Bytes};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest, WithOtherFields};
use alloy_transport::Transport;
use cast::Cast;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, parse_function_args},
};
use foundry_common::ens::NameOrAddress;
use foundry_config::{Chain, Config};
use std::str::FromStr;

/// CLI arguments for `cast access-list`.
#[derive(Debug, Parser)]
pub struct AccessListArgs {
    /// The destination of the transaction.
    #[arg(
        value_name = "TO",
        value_parser = NameOrAddress::from_str
    )]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    #[arg(value_name = "SIG")]
    sig: Option<String>,

    /// The arguments of the function to call.
    #[arg(value_name = "ARGS")]
    args: Vec<String>,

    /// The data for the transaction.
    #[arg(
        long,
        value_name = "DATA",
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    /// The block height to query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[arg(long, short = 'B')]
    block: Option<BlockId>,

    /// Print the access list as JSON.
    #[arg(long, short, help_heading = "Display options")]
    json: bool,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    eth: EthereumOpts,
}

impl AccessListArgs {
    pub async fn run(self) -> Result<()> {
        let AccessListArgs { to, sig, args, data, tx, eth, block, json: to_json } = self;

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;
        let chain = utils::get_chain(config.chain, &provider).await?;
        let sender = eth.wallet.sender().await;
        let etherscan_api_key = config.get_etherscan_api_key(Some(chain));

        let to = match to {
            Some(to) => Some(to.resolve(&provider).await?),
            None => None,
        };

        access_list(
            &provider,
            etherscan_api_key.as_deref(),
            sender,
            to,
            sig,
            args,
            data,
            tx,
            chain,
            block,
            to_json,
        )
        .await?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn access_list<P: Provider<T, AnyNetwork>, T: Transport + Clone>(
    provider: P,
    etherscan_api_key: Option<&str>,
    from: Address,
    to: Option<Address>,
    sig: Option<String>,
    args: Vec<String>,
    data: Option<String>,
    tx: TransactionOpts,
    chain: Chain,
    block: Option<BlockId>,
    to_json: bool,
) -> Result<()> {
    let mut req = WithOtherFields::<TransactionRequest>::default()
        .with_to(to.unwrap_or_default())
        .with_from(from)
        .with_value(tx.value.unwrap_or_default())
        .with_chain_id(chain.id());

    if let Some(gas_limit) = tx.gas_limit {
        req.set_gas_limit(gas_limit.to());
    }

    if let Some(nonce) = tx.nonce {
        req.set_nonce(nonce.to());
    }

    let data = if let Some(sig) = sig {
        parse_function_args(&sig, args, to, chain, &provider, etherscan_api_key).await?.0
    } else if let Some(data) = data {
        // Note: `sig+args` and `data` are mutually exclusive
        hex::decode(data)?
    } else {
        Vec::new()
    };

    req.set_input::<Bytes>(data.into());

    let cast = Cast::new(&provider);

    let access_list: String = cast.access_list(&req, block, to_json).await?;

    println!("{}", access_list);

    Ok(())
}
