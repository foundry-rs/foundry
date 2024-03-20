use alloy_primitives::U256;
use alloy_provider::{network::Ethereum, Provider};
use alloy_rpc_types::BlockId;
use alloy_transport::Transport;
use cast::{Cast, TxBuilder};
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, handle_traces, parse_ether_value, TraceResult},
};
use foundry_common::{ens::NameOrAddress, types::ToAlloy};
use foundry_compilers::EvmVersion;
use foundry_config::{find_project_root_path, Config};
use foundry_evm::{executors::TracingExecutor, opts::EvmOpts};
use std::str::FromStr;

/// CLI arguments for `cast call`.
#[derive(Debug, Parser)]
pub struct CallArgs {
    /// The destination of the transaction.
    #[arg(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    args: Vec<String>,

    /// Data for the transaction.
    #[arg(
        long,
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    /// Forks the remote rpc, executes the transaction locally and prints a trace
    #[arg(long, default_value_t = false)]
    trace: bool,

    /// Opens an interactive debugger.
    /// Can only be used with `--trace`.
    #[arg(long, requires = "trace")]
    debug: bool,

    /// Labels to apply to the traces; format: `address:label`.
    /// Can only be used with `--trace`.
    #[arg(long, requires = "trace")]
    labels: Vec<String>,

    /// The EVM Version to use.
    /// Can only be used with `--trace`.
    #[arg(long, requires = "trace")]
    evm_version: Option<EvmVersion>,

    /// The block height to query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[arg(long, short)]
    block: Option<BlockId>,

    #[command(subcommand)]
    command: Option<CallSubcommands>,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    eth: EthereumOpts,
}

#[derive(Debug, Parser)]
pub enum CallSubcommands {
    /// ignores the address field and simulates creating a contract
    #[command(name = "--create")]
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
        #[arg(long, value_parser = parse_ether_value)]
        value: Option<U256>,
    },
}

impl CallArgs {
    pub async fn run(self) -> Result<()> {
        let CallArgs {
            to,
            sig,
            args,
            data,
            tx,
            eth,
            command,
            block,
            trace,
            evm_version,
            debug,
            labels,
        } = self;

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;
        let alloy_provider = utils::get_alloy_provider(&config)?;
        let chain = utils::get_chain(config.chain, &provider).await?;
        let sender = eth.wallet.sender().await;

        let to = match to {
            Some(NameOrAddress::Name(name)) => {
                Some(NameOrAddress::Name(name).resolve(&alloy_provider).await?)
            }
            Some(NameOrAddress::Address(addr)) => Some(addr),
            None => None,
        };

        let mut builder = TxBuilder::new(&alloy_provider, sender, to, chain, tx.legacy).await?;

        builder
            .gas(tx.gas_limit)
            .etherscan_api_key(config.get_etherscan_api_key(Some(chain)))
            .gas_price(tx.gas_price)
            .priority_gas_price(tx.priority_gas_price)
            .nonce(tx.nonce);

        match command {
            Some(CallSubcommands::Create { code, sig, args, value }) => {
                if trace {
                    let figment = Config::figment_with_root(find_project_root_path(None).unwrap())
                        .merge(eth.rpc);

                    let evm_opts = figment.extract::<EvmOpts>()?;

                    let (env, fork, chain) =
                        TracingExecutor::get_fork_material(&config, evm_opts).await?;

                    let mut executor = TracingExecutor::new(env, fork, evm_version, debug);

                    let trace = match executor.deploy(
                        sender,
                        code.into_bytes().into(),
                        value.unwrap_or(U256::ZERO),
                        None,
                    ) {
                        Ok(deploy_result) => TraceResult::from(deploy_result),
                        Err(evm_err) => TraceResult::try_from(evm_err)?,
                    };

                    handle_traces(trace, &config, chain, labels, debug).await?;

                    return Ok(());
                }

                // fill the builder after the conditional so we dont move values
                fill_create(&mut builder, value, code, sig, args).await?;
            }
            _ => {
                // fill first here because we need to use the builder in the conditional
                fill_tx(&mut builder, tx.value, sig, args, data).await?;

                if trace {
                    let figment = Config::figment_with_root(find_project_root_path(None).unwrap())
                        .merge(eth.rpc);

                    let evm_opts = figment.extract::<EvmOpts>()?;

                    let (env, fork, chain) =
                        TracingExecutor::get_fork_material(&config, evm_opts).await?;

                    let mut executor = TracingExecutor::new(env, fork, evm_version, debug);

                    let (tx, _) = builder.build();

                    let trace = TraceResult::from(executor.call_raw_committing(
                        sender,
                        tx.to_addr().copied().expect("an address to be here").to_alloy(),
                        tx.data().cloned().unwrap_or_default().to_vec().into(),
                        tx.value().copied().unwrap_or_default().to_alloy(),
                    )?);

                    handle_traces(trace, &config, chain, labels, debug).await?;

                    return Ok(());
                }
            }
        };

        let builder_output = builder.build_alloy();
        println!("{}", Cast::new(provider, alloy_provider).call(builder_output, block).await?);

        Ok(())
    }
}

/// fills the builder from create arg
async fn fill_create<P: Provider<Ethereum, T>, T: Transport + Clone>(
    builder: &mut TxBuilder<'_, P, T>,
    value: Option<U256>,
    code: String,
    sig: Option<String>,
    args: Vec<String>,
) -> Result<()> {
    builder.value(value);

    let mut data = hex::decode(code)?;

    if let Some(s) = sig {
        let (mut sigdata, _func) = builder.create_args(&s, args).await?;
        data.append(&mut sigdata);
    }

    builder.set_data(data);

    Ok(())
}

/// fills the builder from args
async fn fill_tx<P: Provider<Ethereum, T>, T: Transport + Clone>(
    builder: &mut TxBuilder<'_, P, T>,
    value: Option<U256>,
    sig: Option<String>,
    args: Vec<String>,
    data: Option<String>,
) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Address;

    #[test]
    fn can_parse_call_data() {
        let data = hex::encode("hello");
        let args = CallArgs::parse_from(["foundry-cli", "--data", data.as_str()]);
        assert_eq!(args.data, Some(data));

        let data = hex::encode_prefixed("hello");
        let args = CallArgs::parse_from(["foundry-cli", "--data", data.as_str()]);
        assert_eq!(args.data, Some(data));
    }

    #[test]
    fn call_sig_and_data_exclusive() {
        let data = hex::encode("hello");
        let to = Address::ZERO;
        let args = CallArgs::try_parse_from([
            "foundry-cli",
            to.to_string().as_str(),
            "signature",
            "--data",
            format!("0x{data}").as_str(),
        ]);

        assert!(args.is_err());
    }
}
