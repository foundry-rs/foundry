use crate::{
    cmd::{forge::build::CoreBuildArgs, Cmd},
    utils,
};
use ansi_term::Colour;
use clap::Parser;
use ethers::{
    abi::RawLog,
    prelude::{Middleware, Provider},
    types::H256,
};
use forge::{
    debug::DebugArena,
    decode::decode_console_logs,
    executor::{builder::Backend, opts::EvmOpts, DeployResult, ExecutorBuilder, RawCallResult},
    trace::{identifier::EtherscanIdentifier, CallTraceArena, CallTraceDecoderBuilder, TraceKind},
};
use foundry_common::evm::EvmArgs;
use foundry_config::{figment::Figment, Config};
use foundry_utils::RuntimeOrHandle;
use std::{
    collections::{BTreeMap, HashMap},
    str::FromStr,
    time::Duration,
};
use ui::{TUIExitReason, Tui, Ui};

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(SimArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct SimArgs {
    /// Open the transaction in the debugger.
    #[clap(long)]
    pub debug: bool,

    #[clap(flatten, next_help_heading = "BUILD OPTIONS")]
    pub opts: CoreBuildArgs,

    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
    pub evm_opts: EvmArgs,

    pub tx: String,

    #[clap(long)]
    pub rpc: String,
}

impl Cmd for SimArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        let figment: Figment = From::from(&self);
        let mut evm_opts = figment.extract::<EvmOpts>()?;
        evm_opts.fork_url = Some(self.rpc.clone());
        let verbosity = evm_opts.verbosity;

        let config = Config::from_provider(figment).sanitized();
        let runtime = RuntimeOrHandle::new();
        let provider = Provider::try_from(evm_opts.fork_url.clone().unwrap().as_str())
            .expect("could not instantiated provider");

        if let Some(tx) = runtime.block_on(
            provider.get_transaction(H256::from_str(&self.tx).expect("invalid tx hash")),
        )? {
            let tx_block_number = tx.block_number.expect("no block number").as_u64();
            let tx_hash = tx.hash();

            // Set the environment to the moment right before our transaction
            evm_opts.fork_block_number = Some(tx_block_number - 1);
            let block_txes = runtime.block_on(provider.get_block_with_txs(tx_block_number))?;

            let env = runtime.block_on(evm_opts.evm_env());
            let db = runtime.block_on(Backend::new(
                utils::get_fork(&evm_opts, &config.rpc_storage_caching),
                &env,
            ));

            let mut builder = ExecutorBuilder::new()
                .with_config(env)
                .with_spec(crate::utils::evm_spec(&config.evm_version))
                .with_gas_limit(evm_opts.gas_limit());

            if verbosity >= 3 {
                builder = builder.with_tracing();
            }
            if self.debug {
                builder = builder.with_tracing().with_debugger();
            }

            let mut result = {
                let mut executor = builder.build(db);

                for past_tx in block_txes.unwrap().transactions.into_iter() {
                    let past_tx_hash = hex::encode(past_tx.hash());
                    if verbosity >= 3 {
                        println!("Executing: 0x{past_tx_hash}.")
                    }

                    if past_tx.hash().eq(&tx_hash) {
                        break
                    }

                    if let Some(to) = past_tx.to {
                        executor
                            .call_raw_committing(past_tx.from, to, past_tx.input.0, past_tx.value)
                            .unwrap();
                    } else {
                        executor.deploy(past_tx.from, past_tx.input.0, past_tx.value).unwrap();
                    }
                }

                if let Some(to) = tx.to {
                    let RawCallResult { reverted, gas, logs, traces, debug: run_debug, .. } =
                        executor.call_raw_committing(tx.from, to, tx.input.0, tx.value)?;

                    RunResult {
                        success: !reverted,
                        logs,
                        traces: vec![(TraceKind::Execution, traces.unwrap_or_default())],
                        debug: run_debug.unwrap_or_default(),
                        gas,
                    }
                } else {
                    let DeployResult { gas, logs, traces, debug: run_debug, .. }: DeployResult =
                        executor.deploy(tx.from, tx.input.0, tx.value).unwrap();

                    RunResult {
                        success: true,
                        logs,
                        traces: vec![(TraceKind::Execution, traces.unwrap_or_default())],
                        debug: run_debug.unwrap_or_default(),
                        gas,
                    }
                }
            };

            let etherscan_identifier = EtherscanIdentifier::new(
                evm_opts.get_remote_chain_id(),
                config.etherscan_api_key,
                Config::foundry_etherscan_cache_dir(evm_opts.get_chain_id()),
                Duration::from_secs(24 * 60 * 60),
            );

            let mut decoder = CallTraceDecoderBuilder::new().build();
            for (_, trace) in &mut result.traces {
                decoder.identify(trace, &etherscan_identifier);
            }

            if self.debug {
                // TODO Get source from etherscan
                let source_code: BTreeMap<u32, String> = BTreeMap::new();
                let calls: Vec<DebugArena> = vec![result.debug];
                let flattened =
                    calls.last().expect("we should have collected debug info").flatten(0);
                let tui = Tui::new(flattened, 0, decoder.contracts, HashMap::new(), source_code)?;
                match tui.start().expect("Failed to start tui") {
                    TUIExitReason::CharExit => return Ok(()),
                }
            } else {
                if verbosity >= 3 {
                    if result.traces.is_empty() {
                        eyre::bail!("Unexpected error: No traces despite verbosity level. Please report this as a bug: https://github.com/gakonst/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
                    }

                    if !result.success && verbosity == 3 || verbosity > 3 {
                        println!("Traces:");
                        for (kind, trace) in &mut result.traces {
                            let should_include = match kind {
                                TraceKind::Setup => {
                                    (verbosity >= 5) || (verbosity == 4 && !result.success)
                                }
                                TraceKind::Execution => verbosity > 3 || !result.success,
                                _ => false,
                            };

                            if should_include {
                                decoder.decode(trace);
                                println!("{}", trace);
                            }
                        }
                        println!();
                    }
                }

                if result.success {
                    println!("{}", Colour::Green.paint("Script ran successfully."));
                } else {
                    println!("{}", Colour::Red.paint("Script failed."));
                }

                println!("Gas used: {}", result.gas);
                println!("== Logs ==");
                let console_logs = decode_console_logs(&result.logs);
                if !console_logs.is_empty() {
                    for log in console_logs {
                        println!("  {}", log);
                    }
                }
            }
        }
        Ok(())
    }
}
struct RunResult {
    pub success: bool,
    pub logs: Vec<RawLog>,
    pub traces: Vec<(TraceKind, CallTraceArena)>,
    pub debug: DebugArena,
    pub gas: u64,
}
