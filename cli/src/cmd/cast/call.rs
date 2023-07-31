// cast estimate subcommands
use crate::{
    cmd::utils::{handle_traces, TraceResult},
    opts::{EthereumOpts, TransactionOpts},
    utils::{self, parse_ether_value},
};
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::{
    solc::EvmVersion,
    types::{BlockId, NameOrAddress, U256},
};
use eyre::WrapErr;
use forge::executor::opts::EvmOpts;
use foundry_config::{find_project_root_path, Config};
use foundry_evm::{
    executor::{DeployResult, EvmError, ExecutionErr, RawCallResult},
    trace::{TraceKind, TracingExecutor},
};
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

    /// Forks the remote rpc, executes the transaction locally and prints a trace
    #[clap(long, default_value_t = false)]
    trace: bool,

    /// Can only be used with "--trace"
    ///
    /// opens an interactive debugger
    #[clap(long, requires = "trace")]
    debug: bool,

    /// Can only be used with "--trace"
    ///
    /// prints a more verbose trace
    #[clap(long, requires = "trace")]
    verbose: bool,

    /// Can only be used with "--trace"
    /// Labels to apply to the traces.
    ///
    /// Format: `address:label`
    #[clap(long, requires = "trace")]
    labels: Vec<String>,

    #[clap(long)]
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
            verbose,
            labels,
        } = self;

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
                if trace {
                    let figment = Config::figment_with_root(find_project_root_path(None).unwrap())
                        .merge(eth.rpc);

                    let evm_opts = figment.extract::<EvmOpts>()?;

                    let (env, fork, chain) =
                        TracingExecutor::get_fork_material(&config, evm_opts).await?;

                    let mut executor =
                        foundry_evm::trace::TracingExecutor::new(env, fork, evm_version, debug)
                            .await;

                    let trace = match executor.deploy(
                        sender,
                        code.into(),
                        value.unwrap_or(U256::zero()),
                        None,
                    ) {
                        Ok(DeployResult { gas_used, traces, debug: run_debug, .. }) => {
                            TraceResult {
                                success: true,
                                traces: vec![(
                                    TraceKind::Execution,
                                    traces.ok_or_else(|| eyre::eyre!("no traces recorded"))?,
                                )],
                                debug: run_debug.unwrap_or_default(),
                                gas_used,
                            }
                        }
                        Err(EvmError::Execution(inner)) => {
                            let ExecutionErr {
                                reverted, gas_used, traces, debug: run_debug, ..
                            } = *inner;
                            TraceResult {
                                success: !reverted,
                                traces: vec![(
                                    TraceKind::Execution,
                                    traces.ok_or_else(|| eyre::eyre!("no traces recorded"))?,
                                )],
                                debug: run_debug.unwrap_or_default(),
                                gas_used,
                            }
                        }
                        Err(err) => {
                            eyre::bail!(
                                "unexpected error when running create transaction: {:?}",
                                err
                            )
                        }
                    };

                    handle_traces(trace, &config, chain, labels, verbose, debug).await?;

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

                    let mut executor =
                        foundry_evm::trace::TracingExecutor::new(env, fork, evm_version, debug)
                            .await;

                    let (tx, _) = builder.build();

                    let trace = match executor.call_raw_committing(
                        sender,
                        tx.to_addr().map(|a| a.clone()).expect("an address to be here"),
                        tx.data().map(|d| d.clone()).unwrap_or_default().to_vec().into(),
                        tx.value().map(|v| v.clone()).unwrap_or_default(),
                    ) {
                        Ok(RawCallResult { gas_used, traces, reverted, debug, .. }) => {
                            TraceResult {
                                success: !reverted,
                                traces: vec![(
                                    TraceKind::Execution,
                                    traces.ok_or_else(|| eyre::eyre!("no traces recorded"))?,
                                )],
                                debug: debug.unwrap_or_default(),
                                gas_used,
                            }
                        }
                        Err(e) => {
                            eyre::bail!("unexpected error when running call transaction: {:?}", e)
                        }
                    };

                    handle_traces(trace, &config, chain, labels, verbose, debug).await?;

                    return Ok(());
                }
            }
        };

        let builder_output: (
            ethers::types::transaction::eip2718::TypedTransaction,
            Option<ethers::abi::Function>,
        ) = builder.build();

        println!("{}", Cast::new(provider).call(builder_output, block).await?);

        Ok(())
    }
}

/// fills the builder from create arg
async fn fill_create(
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

/// fills the builder from args
async fn fill_tx(
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
