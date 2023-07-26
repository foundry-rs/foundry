// cast estimate subcommands
use crate::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, parse_ether_value},
};
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::solc::EvmVersion;
use ethers::types::{BlockId, NameOrAddress, U256};
use eyre::WrapErr;
use forge::executor::{opts::EvmOpts, Backend, ExecutorBuilder};
use foundry_config::{find_project_root_path, Config};
use foundry_evm::utils::evm_spec;
use std::str::FromStr;

type Provider =
    ethers::providers::Provider<ethers::providers::RetryClient<ethers::providers::Http>>;

/// CLI arguments for `cast call`.
#[derive(Debug, Parser)]
pub struct CallArgs {
    /// The destination of the transaction.
    #[clap(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    args: Vec<String>,

    /// Data for the transaction.
    #[clap(
        long,
        value_parser = foundry_common::clap_helpers::strip_0x_prefix,
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    #[clap(long, default_value_t = false)]
    trace: bool,

    #[clap(long, required_if_eq("trace", "true"))]
    evm_version: Option<EvmVersion>,

    /// The block height to query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[clap(long, short)]
    block: Option<BlockId>,

    #[clap(subcommand)]
    command: Option<CallSubcommands>,

    #[clap(flatten)]
    tx: TransactionOpts,

    #[clap(flatten)]
    eth: EthereumOpts,
}

#[derive(Debug, Parser)]
pub enum CallSubcommands {
    /// ignores the address field and simulates creating a contract
    #[clap(name = "--create")]
    Create {
        /// Bytecode of contract.
        code: String,

        /// The signature of the constructor.
        sig: Option<String>,

        /// The arguments of the constructor.
        args: Vec<String>,

        /// Ether to send in the transaction.
        ///
        /// Either specified in wei, or as a string with a unit type.
        ///
        /// Examples: 1ether, 10gwei, 0.01ether
        #[clap(long, value_parser = parse_ether_value)]
        value: Option<U256>,
    },
}

impl CallArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let CallArgs { to, sig, args, data, tx, eth, command, block, trace, evm_version } = self;

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;
        let chain = utils::get_chain(config.chain_id, &provider).await?;
        let sender = eth.wallet.sender().await;

        let mut builder: TxBuilder<'_, Provider> =
            TxBuilder::new(&provider, sender, to, chain, tx.legacy).await?;

        builder
            .gas(tx.gas_limit)
            .etherscan_api_key(config.get_etherscan_api_key(Some(chain)))
            .gas_price(tx.gas_price)
            .priority_gas_price(tx.priority_gas_price)
            .nonce(tx.nonce);
        match command {
            Some(CallSubcommands::Create { code, sig, args, value }) => {
                build_create(&mut builder, value, code, sig, args).await?;
            }
            _ => {
                build_tx(&mut builder, tx.value, sig, args, data).await?;
            }
        };

        if trace {
            // todo:n find out what this is
            let figment =
                Config::figment_with_root(find_project_root_path(None).unwrap()).merge(eth.rpc);

            let mut evm_opts = figment.extract::<EvmOpts>()?;

            evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());

            // Set up the execution environment
            let mut env = evm_opts.evm_env().await;

            let db = Backend::spawn(evm_opts.get_fork(&config, env.clone())).await;

            // configures a bare version of the evm executor: no cheatcode inspector is enabled,
            // tracing will be enabled only for the targeted transaction
            let builder = ExecutorBuilder::default()
                .with_config(env)
                .with_spec(evm_spec(&evm_version.unwrap_or(config.evm_version)));

            let mut executor = builder.build(db);
        } else {
            let builder_output = builder.build();
            println!("{}", Cast::new(provider).call(builder_output, block).await?);
        }
        Ok(())
    }
}

async fn build_create(
    builder: &mut TxBuilder<'_, Provider>,
    value: Option<U256>,
    code: String,
    sig: Option<String>,
    args: Vec<String>,
) -> eyre::Result<()> {
    builder.value(value);

    let mut data = hex::decode(code.strip_prefix("0x").unwrap_or(&code))?;

    if let Some(s) = sig {
        let (mut sigdata, _func) = builder.create_args(&s, args).await?;
        data.append(&mut sigdata);
    }

    builder.set_data(data);

    Ok(())
}

async fn build_tx(
    builder: &mut TxBuilder<'_, Provider>,
    value: Option<U256>,
    sig: Option<String>,
    args: Vec<String>,
    data: Option<String>,
) -> eyre::Result<()> {
    builder.value(value);

    if let Some(sig) = sig {
        builder.set_args(sig.as_str(), args).await?;
    }

    if let Some(data) = data {
        // Note: `sig+args` and `data` are mutually exclusive
        builder.set_data(hex::decode(data).wrap_err("Expected hex encoded function data")?);
    }

    Ok(())
}

fn typed_transaction_to_transaction() -> () {}

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
