use alloy_network::TransactionBuilder;
use alloy_primitives::{TxKind, U256};
use alloy_rpc_types::{BlockId, TransactionRequest, WithOtherFields};
use cast::Cast;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, handle_traces, parse_ether_value, parse_function_args, TraceResult},
};
use foundry_common::ens::NameOrAddress;
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
        let chain = utils::get_chain(config.chain, &provider).await?;
        let sender = eth.wallet.sender().await;
        let etherscan_api_key = config.get_etherscan_api_key(Some(chain));

        let to = match to {
            Some(to) => Some(to.resolve(&provider).await?),
            None => None,
        };

        let mut req = WithOtherFields::<TransactionRequest>::default()
            .with_to(to.unwrap_or_default())
            .with_from(sender)
            .with_value(tx.value.unwrap_or_default());

        if let Some(nonce) = tx.nonce {
            req.set_nonce(nonce.to());
        }

        let (data, func) = match command {
            Some(CallSubcommands::Create { code, sig, args, value }) => {
                if let Some(value) = value {
                    req.set_value(value);
                }

                let mut data = hex::decode(code)?;

                if let Some(s) = sig {
                    let (mut constructor_args, _) = parse_function_args(
                        &s,
                        args,
                        None,
                        chain,
                        &provider,
                        etherscan_api_key.as_deref(),
                    )
                    .await?;
                    data.append(&mut constructor_args);
                }

                if trace {
                    let figment = Config::figment_with_root(find_project_root_path(None).unwrap())
                        .merge(eth.rpc);

                    let evm_opts = figment.extract::<EvmOpts>()?;

                    let (env, fork, chain) =
                        TracingExecutor::get_fork_material(&config, evm_opts).await?;

                    let mut executor = TracingExecutor::new(env, fork, evm_version, debug);

                    let trace = match executor.deploy(
                        sender,
                        data.into(),
                        req.value.unwrap_or_default(),
                        None,
                    ) {
                        Ok(deploy_result) => TraceResult::from(deploy_result),
                        Err(evm_err) => TraceResult::try_from(evm_err)?,
                    };

                    handle_traces(trace, &config, chain, labels, debug).await?;

                    return Ok(());
                }

                (data, None)
            }
            _ => {
                // fill first here because we need to use the builder in the conditional
                let (data, func) = if let Some(sig) = sig {
                    parse_function_args(
                        &sig,
                        args,
                        to,
                        chain,
                        &provider,
                        etherscan_api_key.as_deref(),
                    )
                    .await?
                } else if let Some(data) = data {
                    // Note: `sig+args` and `data` are mutually exclusive
                    (hex::decode(data)?, None)
                } else {
                    (Vec::new(), None)
                };

                if trace {
                    let figment = Config::figment_with_root(find_project_root_path(None).unwrap())
                        .merge(eth.rpc);

                    let evm_opts = figment.extract::<EvmOpts>()?;

                    let (env, fork, chain) =
                        TracingExecutor::get_fork_material(&config, evm_opts).await?;

                    let mut executor = TracingExecutor::new(env, fork, evm_version, debug);

                    let to = if let Some(TxKind::Call(to)) = req.to { Some(to) } else { None };
                    let trace = TraceResult::from(executor.call_raw_committing(
                        sender,
                        to.expect("an address to be here"),
                        data.into(),
                        req.value.unwrap_or_default(),
                    )?);

                    handle_traces(trace, &config, chain, labels, debug).await?;

                    return Ok(());
                }

                (data, func)
            }
        };

        req.set_input(data);

        println!("{}", Cast::new(provider).call(&req, func.as_ref(), block).await?);

        Ok(())
    }
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
