// cast estimate subcommands
use crate::opts::EthereumOpts;
use crate::utils::parse_ether_value;
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::{
    providers::{Middleware, Provider},
    types::{NameOrAddress, U256},
};
use foundry_config::{Chain, Config};

#[derive(Debug, Parser)]
pub enum EstimateSubcommands {
    #[clap(name = "--create", about = "Estimate gas cost to deploy a smart contract")]
    Create {
        #[clap(help = "Bytecode of contract.", value_name = "CODE")]
        code: String,
        #[clap(help = "The signature of the function to call.", value_name = "SIG")]
        sig: Option<String>,
        #[clap(help = "The arguments of the function to call.", value_name = "ARGS")]
        args: Vec<String>,
        #[clap(
            long,
            help = "Ether to send in the transaction.",
            long_help = r#"Ether to send in the transaction, either specified in wei, or as a string with a unit type.

Examples: 1ether, 10gwei, 0.01ether"#,
            parse(try_from_str = parse_ether_value),
            value_name = "VALUE"
        )]
        value: Option<U256>,
    },
}

pub async fn run(
    to: Option<NameOrAddress>,
    sig: Option<String>,
    args: Vec<String>,
    value: Option<U256>,
    eth: EthereumOpts,
    command: Option<EstimateSubcommands>,
) -> eyre::Result<()> {
    let config = Config::from(&eth);
    let provider = Provider::try_from(
        config.eth_rpc_url.unwrap_or_else(|| "http://localhost:8545".to_string()),
    )?;

    let chain: Chain = if let Some(chain) = eth.chain {
        chain.into()
    } else {
        provider.get_chainid().await?.into()
    };

    let from = eth.sender().await;
    let mut builder = TxBuilder::new(&provider, from, to, chain, false).await?;
    match command {
        Some(EstimateSubcommands::Create { code, sig, args, value }) => {
            builder.etherscan_api_key(config.etherscan_api_key).value(value);

            let mut data = hex::decode(code.strip_prefix("0x").unwrap_or(&code))?;

            match sig {
                Some(sig) => {
                    let (mut sigdata, _func) = builder.create_args(&sig, args).await?;
                    data.append(&mut sigdata);
                }
                None => {}
            };

            builder.set_data(data);
        }
        _ => {
            builder
                .etherscan_api_key(config.etherscan_api_key)
                .value(value)
                .set_args(sig.unwrap().as_str(), args)
                .await?;
        }
    };

    let builder_output = builder.peek();
    let gas = Cast::new(&provider).estimate(builder_output).await?;
    println!("{gas}");
    Ok(())
}
