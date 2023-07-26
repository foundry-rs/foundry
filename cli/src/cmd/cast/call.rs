// cast estimate subcommands
use crate::{
    opts::{EthereumOpts, RpcOpts, TransactionOpts},
    utils::{self, parse_ether_value},
};
use cast::{Cast, TxBuilder};
use clap::Parser;
use ethers::{
    solc::{artifacts::ContractBytecodeSome, ArtifactId, EvmVersion},
    types::{BlockId, NameOrAddress, U256},
};
use eyre::WrapErr;
use forge::executor::{opts::EvmOpts, Backend, ExecutorBuilder};
use foundry_config::{find_project_root_path, Config};
use foundry_evm::{
    debug::DebugArena,
    executor::{DeployResult, EvmError, ExecutionErr, Executor, RawCallResult},
    trace::{
        identifier::{EtherscanIdentifier, SignaturesIdentifier},
        CallTraceDecoder, CallTraceDecoderBuilder, TraceKind, Traces,
    },
    utils::evm_spec,
};
use std::{collections::BTreeMap, ops::DerefMut, str::FromStr};
use ui::{TUIExitReason, Tui, Ui};
use yansi::Paint;

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

    /// only for tracing mode
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

/// a default executor with tracing enabled
struct TracingExecutor {
    executor: Executor,
}

impl TracingExecutor {
    pub async fn new(
        config: &foundry_config::Config,
        version: Option<EvmVersion>,
        rpc_opts: RpcOpts,
        debug: bool,
    ) -> eyre::Result<(Self, EvmOpts)> {
        // todo:n find out what this is
        let figment =
            Config::figment_with_root(find_project_root_path(None).unwrap()).merge(rpc_opts);

        let mut evm_opts = figment.extract::<EvmOpts>()?;

        evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());
        evm_opts.fork_block_number = config.fork_block_number;

        // Set up the execution environment
        let env = evm_opts.evm_env().await;

        let db = Backend::spawn(evm_opts.get_fork(&config, env.clone())).await;

        // configures a bare version of the evm executor: no cheatcode inspector is enabled,
        // tracing will be enabled only for the targeted transaction
        let builder = ExecutorBuilder::default()
            .with_config(env)
            .with_spec(evm_spec(&version.unwrap_or(config.evm_version)));

        let mut executor = builder.build(db);

        executor.set_tracing(true).set_debugger(debug);

        Ok((Self { executor }, evm_opts))
    }
}

impl std::ops::Deref for TracingExecutor {
    type Target = Executor;

    fn deref(&self) -> &Self::Target {
        &self.executor
    }
}

impl DerefMut for TracingExecutor {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.executor
    }
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
                    let (mut executor, opts) =
                        TracingExecutor::new(&config, evm_version, eth.rpc, debug).await?;

                    let trace = match executor.deploy(
                        sender,
                        code.into(),
                        value.unwrap_or(U256::zero()),
                        None,
                    ) {
                        Ok(DeployResult { gas_used, traces, debug: run_debug, .. }) => {
                            TraceResult {
                                success: true,
                                traces: vec![(TraceKind::Execution, traces.unwrap_or_default())],
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
                                traces: vec![(TraceKind::Execution, traces.unwrap_or_default())],
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

                    handle_traces(trace, config, opts, labels, verbose, debug).await?;

                    return Ok(());
                }

                // build it last because we dont need anything from the built one
                build_create_tx(&mut builder, value, code, sig, args).await?;
            }
            _ => {
                // build it first becasue builder parses args / addr
                build_tx(&mut builder, tx.value, sig, args, data).await?;

                if trace {
                    let (mut executor, opts) =
                        TracingExecutor::new(&config, evm_version, eth.rpc, debug).await?;

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
                                traces: vec![(TraceKind::Execution, traces.unwrap_or_default())],
                                debug: debug.unwrap_or_default(),
                                gas_used,
                            }
                        }
                        Err(e) => {
                            eyre::bail!("unexpected error when running call transaction: {:?}", e)
                        }
                    };

                    handle_traces(trace, config, opts, labels, verbose, debug).await?;

                    return Ok(());
                }
            }
        };

        let builder_output = builder.build();

        println!("{}", Cast::new(provider).call(builder_output, block).await?);

        Ok(())
    }
}

/// builds the transaction from create args
async fn build_create_tx(
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

/// builds the tx from args
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

/// labels the traces, conditonally prints them or opens the debugger
async fn handle_traces(
    mut result: TraceResult,
    config: Config,
    evm_opts: EvmOpts,
    labels: Vec<String>,
    verbose: bool,
    debug: bool,
) -> eyre::Result<()> {
    let mut etherscan_identifier =
        EtherscanIdentifier::new(&config, evm_opts.get_remote_chain_id())?;

    let labeled_addresses = labels.iter().filter_map(|label_str| {
        let mut iter = label_str.split(':');

        if let Some(addr) = iter.next() {
            if let (Ok(address), Some(label)) =
                (ethers::types::Address::from_str(addr), iter.next())
            {
                return Some((address, label.to_string()));
            }
        }
        None
    });

    let mut decoder = CallTraceDecoderBuilder::new().with_labels(labeled_addresses).build();

    decoder.add_signature_identifier(SignaturesIdentifier::new(
        Config::foundry_cache_dir(),
        config.offline,
    )?);

    for (_, trace) in &mut result.traces {
        decoder.identify(trace, &mut etherscan_identifier);
    }

    if debug {
        let (sources, bytecode) = etherscan_identifier.get_compiled_contracts().await?;
        run_debugger(result, decoder, bytecode, sources)?;
    } else {
        print_traces(&mut result, decoder, verbose).await?;
    }

    Ok(())
}

async fn print_traces(
    result: &mut TraceResult,
    decoder: CallTraceDecoder,
    verbose: bool,
) -> eyre::Result<()> {
    if result.traces.is_empty() {
        eyre::bail!("Unexpected error: No traces. Please report this as a bug: https://github.com/foundry-rs/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
    }

    println!("Traces:");
    for (_, trace) in &mut result.traces {
        decoder.decode(trace).await;
        if !verbose {
            println!("{trace}");
        } else {
            println!("{trace:#}");
        }
    }
    println!();

    if result.success {
        println!("{}", Paint::green("Transaction successfully executed."));
    } else {
        println!("{}", Paint::red("Transaction failed."));
    }

    println!("Gas used: {}", result.gas_used);
    Ok(())
}

fn run_debugger(
    result: TraceResult,
    decoder: CallTraceDecoder,
    known_contracts: BTreeMap<ArtifactId, ContractBytecodeSome>,
    sources: BTreeMap<ArtifactId, String>,
) -> eyre::Result<()> {
    let calls: Vec<DebugArena> = vec![result.debug];
    let flattened = calls.last().expect("we should have collected debug info").flatten(0);
    let tui = Tui::new(
        flattened,
        0,
        decoder.contracts,
        known_contracts.into_iter().map(|(id, artifact)| (id.name, artifact)).collect(),
        sources
            .into_iter()
            .map(|(id, source)| {
                let mut sources = BTreeMap::new();
                sources.insert(0, source);
                (id.name, sources)
            })
            .collect(),
        Default::default(),
    )?;
    match tui.start().expect("Failed to start tui") {
        TUIExitReason::CharExit => Ok(()),
    }
}

/// taken from cast run, should find common place
struct TraceResult {
    pub success: bool,
    pub traces: Traces,
    pub debug: DebugArena,
    pub gas_used: u64,
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
