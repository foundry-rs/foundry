use alloy_network::TransactionBuilder;
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts},
    utils::{self, parse_ether_value, parse_function_args},
};
use foundry_common::ens::NameOrAddress;
use foundry_config::{figment::Figment, Config};
use std::str::FromStr;

/// CLI arguments for `cast estimate`.
#[derive(Debug, Parser)]
pub struct EstimateArgs {
    /// The destination of the transaction.
    #[arg(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    args: Vec<String>,

    /// The sender account.
    #[arg(
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
    #[arg(long, value_parser = parse_ether_value)]
    value: Option<U256>,

    #[command(flatten)]
    rpc: RpcOpts,

    #[command(flatten)]
    etherscan: EtherscanOpts,

    #[command(subcommand)]
    command: Option<EstimateSubcommands>,
}

#[derive(Debug, Parser)]
pub enum EstimateSubcommands {
    /// Estimate gas cost to deploy a smart contract
    #[command(name = "--create")]
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
        #[arg(long, value_parser = parse_ether_value)]
        value: Option<U256>,
    },
}

impl EstimateArgs {
    pub async fn run(self) -> Result<()> {
        let EstimateArgs { from, to, sig, args, value, rpc, etherscan, command } = self;

        let figment = Figment::from(Config::figment()).merge(etherscan).merge(rpc);
        let config = Config::try_from(figment)?;
        let provider = utils::get_provider(&config)?;
        let chain = utils::get_chain(config.chain, &provider).await?;
        let api_key = config.get_etherscan_api_key(Some(chain));

        let from = from.resolve(&provider).await?;
        let to = match to {
            Some(to) => Some(to.resolve(&provider).await?),
            None => None,
        };

        let mut req = TransactionRequest::default()
            .with_to(to.into())
            .with_from(from)
            .with_value(value.unwrap_or_default());

        let data = match command {
            Some(EstimateSubcommands::Create { code, sig, args, value }) => {
                if let Some(value) = value {
                    req.set_value(value);
                }

                let mut data = hex::decode(code)?;

                if let Some(s) = sig {
                    let (mut constructor_args, _) =
                        parse_function_args(&s, args, to, chain, &provider, api_key.as_deref())
                            .await?;
                    data.append(&mut constructor_args);
                }

                data
            }
            _ => {
                let sig = sig.ok_or_else(|| eyre::eyre!("Function signature must be provided."))?;
                parse_function_args(&sig, args, to, chain, &provider, api_key.as_deref()).await?.0
            }
        };

        req.set_input(data.into());

        let gas = provider.estimate_gas(&req, None).await?;
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
