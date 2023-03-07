// cast estimate subcommands
use crate::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, parse_ether_value},
};
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::types::{BlockId, NameOrAddress, U256};
use eyre::WrapErr;
use foundry_config::Config;
use std::str::FromStr;

#[derive(Debug, Parser)]
pub struct CallArgs {
    #[clap(
        help = "The destination of the transaction.", 
        value_name = "TO",
        value_parser = NameOrAddress::from_str
    )]
    to: Option<NameOrAddress>,

    #[clap(help = "The signature of the function to call.", value_name = "SIG")]
    sig: Option<String>,

    #[clap(help = "The arguments of the function to call.", value_name = "ARGS")]
    args: Vec<String>,

    #[clap(
        long,
        help = "Data for the transaction.",
        value_name = "DATA",
        value_parser = foundry_common::clap_helpers::strip_0x_prefix,
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    #[clap(flatten)]
    tx: TransactionOpts,

    #[clap(flatten)]
    eth: EthereumOpts,

    #[clap(
        long,
        short,
        help = "the block you want to query, can also be earliest/latest/pending",
        value_name = "BLOCK"
    )]
    block: Option<BlockId>,

    #[clap(subcommand)]
    command: Option<CallSubcommands>,
}

#[derive(Debug, Parser)]
pub enum CallSubcommands {
    #[clap(name = "--create", about = "Simulate a contract deployment.")]
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
impl CallArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let CallArgs { to, sig, args, data, tx, eth, command, block } = self;

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;
        let chain = utils::get_chain(config.chain_id, &provider).await?;
        let sender = eth.wallet.sender().await;

        let mut builder = TxBuilder::new(&provider, sender, to, chain, tx.legacy).await?;
        builder
            .gas(tx.gas_limit)
            .etherscan_api_key(config.get_etherscan_api_key(Some(chain)))
            .gas_price(tx.gas_price)
            .priority_gas_price(tx.priority_gas_price)
            .nonce(tx.nonce);
        match command {
            Some(CallSubcommands::Create { code, sig, args, value }) => {
                builder.value(value);

                let mut data = hex::decode(code.strip_prefix("0x").unwrap_or(&code))?;

                if let Some(s) = sig {
                    let (mut sigdata, _func) = builder.create_args(&s, args).await?;
                    data.append(&mut sigdata);
                }

                builder.set_data(data);
            }
            _ => {
                builder.value(tx.value);

                if let Some(sig) = sig {
                    builder.set_args(sig.as_str(), args).await?;
                }
                if let Some(data) = data {
                    // Note: `sig+args` and `data` are mutually exclusive
                    builder.set_data(
                        hex::decode(data).wrap_err("Expected hex encoded function data")?,
                    );
                }
            }
        };

        let builder_output = builder.build();
        println!("{}", Cast::new(provider).call(builder_output, block).await?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::types::Address;

    #[test]
    fn can_parse_call_data() {
        let data = hex::encode("hello");
        let args: CallArgs =
            CallArgs::parse_from(["foundry-cli", "--data", format!("0x{data}").as_str()]);
        assert_eq!(args.data, Some(data.clone()));

        let args: CallArgs = CallArgs::parse_from(["foundry-cli", "--data", data.as_str()]);
        assert_eq!(args.data, Some(data));
    }

    #[test]
    fn call_sig_and_data_exclusive() {
        let data = hex::encode("hello");
        let to = Address::zero();
        let args = CallArgs::try_parse_from([
            "foundry-cli",
            format!("{to:?}").as_str(),
            "signature",
            "--data",
            format!("0x{data}").as_str(),
        ]);

        assert!(args.is_err());
    }
}
