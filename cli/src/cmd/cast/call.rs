// cast estimate subcommands
use crate::{
    opts::{
        cast::{parse_block_id, parse_name_or_address},
        EthereumOpts, TransactionOpts,
    },
    utils::parse_ether_value,
};
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::{
    providers::Middleware,
    types::{BlockId, NameOrAddress, U256},
};
use foundry_common::get_http_provider;
use foundry_config::{Chain, Config};

#[derive(Debug, Parser)]
pub struct CallArgs {
    #[clap(help = "The destination of the transaction.", parse(try_from_str = parse_name_or_address), value_name = "TO")]
    to: Option<NameOrAddress>,
    #[clap(help = "The signature of the function to call.", value_name = "SIG")]
    sig: Option<String>,
    #[clap(help = "The arguments of the function to call.", value_name = "ARGS")]
    args: Vec<String>,
    #[clap(flatten, next_help_heading = "TRANSACTION OPTIONS")]
    tx: TransactionOpts,
    #[clap(flatten)]
    // TODO: We only need RPC URL and Etherscan API key here.
    eth: EthereumOpts,
    #[clap(long, short, help = "the block you want to query, can also be earliest/latest/pending", parse(try_from_str = parse_block_id), value_name = "BLOCK")]
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
            parse(try_from_str = parse_ether_value),
            value_name = "VALUE"
        )]
        value: Option<U256>,
    },
}
impl CallArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let CallArgs { to, sig, args, tx, eth, command, block } = self;
        let config = Config::from(&eth);
        let provider = get_http_provider(config.get_rpc_url_or_localhost_http()?);

        let chain: Chain =
            if let Some(chain) = eth.chain { chain } else { provider.get_chainid().await?.into() };

        let from = eth.sender().await;
        let mut builder = TxBuilder::new(&provider, from, to, chain, tx.legacy).await?;
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
                builder.value(tx.value).set_args(sig.unwrap().as_str(), args).await?;
            }
        };

        let builder_output = builder.build();
        println!("{}", Cast::new(provider).call(builder_output, block).await?);
        Ok(())
    }
}
