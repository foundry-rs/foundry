// cast estimate subcommands
use crate::{
    opts::EthereumOpts,
    utils::{self, parse_ether_value},
};
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::types::{NameOrAddress, U256};
use foundry_common::try_get_http_provider;
use foundry_config::Config;
use std::str::FromStr;

/// CLI arguments for `cast estimate`.
#[derive(Debug, Parser)]
pub struct EstimateArgs {
    #[clap(
        help = "The destination of the transaction.",
        value_parser = NameOrAddress::from_str,
        value_name = "TO",
    )]
    to: Option<NameOrAddress>,
    #[clap(help = "The signature of the function to call.", value_name = "SIG")]
    sig: Option<String>,
    #[clap(help = "The arguments of the function to call.", value_name = "ARGS")]
    args: Vec<String>,
    #[clap(
        long,
        help = "Ether to send in the transaction.",
        long_help = r#"Ether to send in the transaction, either specified in wei, or as a string with a unit type.

Examples: 1ether, 10gwei, 0.01ether"#,
        value_parser = parse_ether_value,
        value_name = "VALUE"
    )]
    value: Option<U256>,
    #[clap(flatten)]
    // TODO: We only need RPC URL and Etherscan API key here.
    eth: EthereumOpts,
    #[clap(subcommand)]
    command: Option<EstimateSubcommands>,
}

#[derive(Debug, Parser)]
pub enum EstimateSubcommands {
    #[clap(name = "--create", about = "Estimate gas cost to deploy a smart contract")]
    Create {
        #[clap(help = "Bytecode of contract.", value_name = "CODE")]
        code: String,
        #[clap(help = "The signature of the constructor.", value_name = "SIG")]
        sig: Option<String>,
        #[clap(help = "Constructor arguments", value_name = "ARGS")]
        args: Vec<String>,
        #[clap(
            long,
            help = "Ether to send in the transaction.",
            long_help = r#"Ether to send in the transaction, either specified in wei, or as a string with a unit type.

Examples: 1ether, 10gwei, 0.01ether"#,
            value_parser = parse_ether_value,
            value_name = "VALUE"
        )]
        value: Option<U256>,
    },
}
impl EstimateArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let EstimateArgs { to, sig, args, value, eth, command } = self;
        let config = Config::from(&eth);
        let provider = try_get_http_provider(config.get_rpc_url_or_localhost_http()?)?;

        let chain = utils::get_chain(config.chain_id, &provider).await?;
        let sender = eth.wallet.sender().await;
        let mut builder = TxBuilder::new(&provider, sender, to, chain, false).await?;
        builder.etherscan_api_key(config.get_etherscan_api_key(Some(chain)));
        match command {
            Some(EstimateSubcommands::Create { code, sig, args, value }) => {
                builder.value(value);

                let mut data = hex::decode(code.strip_prefix("0x").unwrap_or(&code))?;

                if let Some(s) = sig {
                    let (mut sigdata, _func) = builder.create_args(&s, args).await?;
                    data.append(&mut sigdata);
                }

                builder.set_data(data);
            }
            _ => {
                let sig = sig.ok_or_else(|| eyre::eyre!("Function signature must be provided."))?;
                builder.value(value).set_args(sig.as_str(), args).await?;
            }
        };

        let builder_output = builder.peek();
        let gas = Cast::new(&provider).estimate(builder_output).await?;
        println!("{gas}");
        Ok(())
    }
}
