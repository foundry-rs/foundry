use alloy_primitives::U256;
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::types::NameOrAddress;
use eyre::Result;
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils::{self, parse_ether_value},
};
use foundry_config::{figment::Figment, Config};
use std::str::FromStr;

/// CLI arguments for `cast estimate`.
#[derive(Debug, Parser)]
pub struct EstimateArgs {
    /// The destination of the transaction.
    #[clap(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    args: Vec<String>,

    /// The sender account.
    #[clap(
        short,
        long,
        value_parser = NameOrAddress::from_str,
        default_value = "0x0000000000000000000000000000000000000000",
        env = "ETH_FROM",
    )]
    from: NameOrAddress,

    /// Ether to send in the transaction.
    ///
    /// Either specified in wei, or as a string with a unit type:
    ///
    /// Examples: 1ether, 10gwei, 0.01ether
    #[clap(long, value_parser = parse_ether_value)]
    value: Option<U256>,

    #[clap(flatten)]
    rpc: RpcOpts,

    #[clap(flatten)]
    etherscan: EtherscanOpts,

    #[clap(subcommand)]
    command: Option<EstimateSubcommands>,
}

#[derive(Debug, Parser)]
pub enum EstimateSubcommands {
    /// Estimate gas cost to deploy a smart contract
    #[clap(name = "--create")]
    Create {
        /// The bytecode of contract
        code: String,

        /// The signature of the constructor
        sig: Option<String>,

        /// Constructor arguments
        args: Vec<String>,

        /// Ether to send in the transaction
        ///
        /// Either specified in wei, or as a string with a unit type:
        ///
        /// Examples: 1ether, 10gwei, 0.01ether
        #[clap(long, value_parser = parse_ether_value)]
        value: Option<U256>,
    },
}

impl EstimateArgs {
    pub async fn run(self) -> Result<()> {
        let EstimateArgs { from, to, sig, args, value, rpc, etherscan, command } = self;

        let figment = Figment::from(Config::figment()).merge(etherscan).merge(rpc);
        let config = Config::from_provider(figment);

        let provider = utils::get_provider(&config)?;
        let chain = utils::get_chain(config.chain_id, &provider).await?;
        let api_key = config.get_etherscan_api_key(Some(chain));

        let mut builder = TxBuilder::new(&provider, from, to, chain, false).await?;
        builder.etherscan_api_key(api_key);

        match command {
            Some(EstimateSubcommands::Create { code, sig, args, value }) => {
                builder.value(value);

                let mut data = hex::decode(code)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_estimate_value() {
        let args: EstimateArgs = EstimateArgs::parse_from(["foundry-cli", "--value", "100"]);
        assert!(args.value.is_some());
    }
}
