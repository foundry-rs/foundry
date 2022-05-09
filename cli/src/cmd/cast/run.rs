use crate::{cmd::Cmd, utils};
use cast::trace::CallTraceDecoder;
use clap::Parser;
use ethers::{
    abi::Address,
    prelude::{Middleware, Provider},
    types::H256,
};
use forge::{
    debug::DebugArena,
    executor::{builder::Backend, opts::EvmOpts, DeployResult, ExecutorBuilder, RawCallResult},
    trace::{identifier::EtherscanIdentifier, CallTraceArena, CallTraceDecoderBuilder, TraceKind},
};
use foundry_config::Config;
use foundry_utils::RuntimeOrHandle;
use std::{
    collections::{BTreeMap, HashMap},
    str::FromStr,
    time::Duration,
};
use ui::{TUIExitReason, Tui, Ui};
use yansi::Paint;

#[derive(Debug, Clone, Parser)]
pub struct RunArgs {
    #[clap(help = "The transaction hash.")]
    tx: String,
    #[clap(short, long, env = "ETH_RPC_URL")]
    rpc_url: String,
    #[clap(long, short = 'd', help = "Debugs the transaction.")]
    debug: bool,
    #[clap(
        long,
        short = 'q',
        help = "Executes the transaction only with the state from the previous block. May result in different results than the live execution!"
    )]
    quick: bool,
    #[clap(
        long,
        help = "Labels address in the trace. 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045:vitalik.eth"
    )]
    label: Vec<String>,
}

impl Cmd for RunArgs {
    type Output = ();
    fn run(self) -> eyre::Result<Self::Output> {
        RuntimeOrHandle::new().block_on(self.run_tx())
    }
}

impl RunArgs {
    async fn run_tx(self) -> eyre::Result<()> {
        let figment = Config::figment();
        let mut evm_opts = figment.extract::<EvmOpts>()?;
        let config = Config::from_provider(figment).sanitized();

        let provider =
            Provider::try_from(self.rpc_url.as_str()).expect("could not instantiate provider");

        if let Some(tx) =
            provider.get_transaction(H256::from_str(&self.tx).expect("invalid tx hash")).await?
        {
            let tx_block_number = tx.block_number.expect("no block number").as_u64();
            let tx_hash = tx.hash();
            evm_opts.fork_url = Some(self.rpc_url);
            evm_opts.fork_block_number = Some(tx_block_number - 1);

            // Set up the execution environment
            let env = evm_opts.evm_env().await;
            let db =
                Backend::new(utils::get_fork(&evm_opts, &config.rpc_storage_caching), &env).await;

            let builder = ExecutorBuilder::new()
                .with_config(env)
                .with_spec(crate::utils::evm_spec(&config.evm_version));

            let mut executor = builder.build(db);

            // Set the state to the moment right before the transaction
            if !self.quick {
                println!("Executing previous transactions from the block.");

                let block_txes = provider.get_block_with_txs(tx_block_number).await?;

                for past_tx in block_txes.unwrap().transactions.into_iter() {
                    if past_tx.hash().eq(&tx_hash) {
                        break
                    }

                    executor.set_gas_limit(past_tx.gas);

                    if let Some(to) = past_tx.to {
                        executor
                            .call_raw_committing(past_tx.from, to, past_tx.input.0, past_tx.value)
                            .unwrap();
                    } else {
                        executor
                            .deploy(past_tx.from, past_tx.input.0, past_tx.value, None)
                            .unwrap();
                    }
                }
            }

            // Execute our transaction
            let mut result = {
                executor.set_tracing(true).set_gas_limit(tx.gas);

                if self.debug {
                    executor.set_debugger(true);
                }

                if let Some(to) = tx.to {
                    let RawCallResult { reverted, gas, traces, debug: run_debug, .. } =
                        executor.call_raw_committing(tx.from, to, tx.input.0, tx.value)?;

                    RunResult {
                        success: !reverted,
                        traces: vec![(TraceKind::Execution, traces.unwrap_or_default())],
                        debug: run_debug.unwrap_or_default(),
                        gas,
                    }
                } else {
                    let DeployResult { gas, traces, debug: run_debug, .. }: DeployResult =
                        executor.deploy(tx.from, tx.input.0, tx.value, None).unwrap();

                    RunResult {
                        success: true,
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

            let labeled_addresses: BTreeMap<Address, String> = self
                .label
                .iter()
                .filter_map(|label_str| {
                    let mut iter = label_str.split(':');

                    if let Some(addr) = iter.next() {
                        if let (Ok(address), Some(label)) = (Address::from_str(addr), iter.next()) {
                            return Some((address, label.to_string()))
                        }
                    }
                    None
                })
                .collect();

            let mut decoder = CallTraceDecoderBuilder::new().with_labels(labeled_addresses).build();

            for (_, trace) in &mut result.traces {
                decoder.identify(trace, &etherscan_identifier);
            }

            if self.debug {
                run_debugger(result, decoder)?;
            } else {
                print_traces(&mut result, decoder)?;
            }
        }
        Ok(())
    }
}

fn run_debugger(result: RunResult, decoder: CallTraceDecoder) -> eyre::Result<()> {
    // TODO Get source from etherscan
    let source_code: BTreeMap<u32, String> = BTreeMap::new();
    let calls: Vec<DebugArena> = vec![result.debug];
    let flattened = calls.last().expect("we should have collected debug info").flatten(0);
    let tui = Tui::new(flattened, 0, decoder.contracts, HashMap::new(), source_code)?;
    match tui.start().expect("Failed to start tui") {
        TUIExitReason::CharExit => Ok(()),
    }
}

fn print_traces(result: &mut RunResult, decoder: CallTraceDecoder) -> eyre::Result<()> {
    if result.traces.is_empty() {
        eyre::bail!("Unexpected error: No traces. Please report this as a bug: https://github.com/foundry-rs/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
    }

    println!("Traces:");
    for (_, trace) in &mut result.traces {
        decoder.decode(trace);
        println!("{trace}");
    }
    println!();

    if result.success {
        println!("{}", Paint::green("Script ran successfully."));
    } else {
        println!("{}", Paint::red("Script failed."));
    }

    println!("Gas used: {}", result.gas);
    Ok(())
}

struct RunResult {
    pub success: bool,
    pub traces: Vec<(TraceKind, CallTraceArena)>,
    pub debug: DebugArena,
    pub gas: u64,
}
