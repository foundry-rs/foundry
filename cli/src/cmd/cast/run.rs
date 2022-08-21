use crate::{cmd::Cmd, init_progress, update_progress, utils::consume_config_rpc_url};
use cast::{
    revm::TransactTo,
    trace::{identifier::SignaturesIdentifier, CallTraceDecoder},
};
use clap::Parser;
use ethers::{
    abi::Address,
    prelude::Middleware,
    solc::utils::RuntimeOrHandle,
    types::{Transaction, H256},
};
use eyre::WrapErr;
use forge::{
    debug::DebugArena,
    executor::{opts::EvmOpts, Backend, DeployResult, ExecutorBuilder, RawCallResult},
    trace::{identifier::EtherscanIdentifier, CallTraceArena, CallTraceDecoderBuilder, TraceKind},
    utils::h256_to_u256_be,
};
use foundry_common::get_http_provider;
use foundry_config::{find_project_root_path, Config};
use indicatif::{ProgressBar, ProgressStyle};
use std::{
    collections::{BTreeMap, HashMap},
    str::FromStr,
};
use ui::{TUIExitReason, Tui, Ui};
use yansi::Paint;

#[derive(Debug, Clone, Parser)]
pub struct RunArgs {
    #[clap(help = "The transaction hash.", value_name = "TXHASH")]
    tx: String,
    #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
    rpc_url: Option<String>,
    #[clap(long, short = 'd', help = "Debugs the transaction.")]
    debug: bool,
    #[clap(
        long,
        short = 'q',
        help = "Executes the transaction only with the state from the previous block. May result in different results than the live execution!"
    )]
    quick: bool,
    #[clap(long, short = 'v', help = "Prints full address")]
    verbose: bool,
    #[clap(
        long,
        help = "Labels address in the trace. 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045:vitalik.eth",
        value_name = "LABEL"
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
        let figment = Config::figment_with_root(find_project_root_path().unwrap());
        let mut evm_opts = figment.extract::<EvmOpts>()?;
        let config = Config::from_provider(figment).sanitized();

        let rpc_url = consume_config_rpc_url(self.rpc_url);
        let provider = get_http_provider(rpc_url.as_str());

        if let Some(tx) = provider
            .get_transaction(
                H256::from_str(&self.tx)
                    .wrap_err_with(|| format!("invalid tx hash: {:?}", self.tx))?,
            )
            .await?
        {
            let tx_block_number = tx.block_number.expect("no block number").as_u64();
            let tx_hash = tx.hash();
            evm_opts.fork_url = Some(rpc_url);
            // we need to set the fork block to the previous block, because that's the state at
            // which we access the data in order to execute the transaction(s)
            evm_opts.fork_block_number = Some(tx_block_number - 1);

            // Set up the execution environment
            let env = evm_opts.evm_env().await;
            let db = Backend::spawn(evm_opts.get_fork(&config, env.clone()));

            let builder = ExecutorBuilder::default()
                .with_config(env)
                .with_spec(crate::utils::evm_spec(&config.evm_version));

            let mut executor = builder.build(db);

            let mut env = executor.env().clone();
            env.block.number = tx_block_number.into();

            let block = provider.get_block_with_txs(tx_block_number).await?;
            if let Some(ref block) = block {
                env.block.timestamp = block.timestamp;
                env.block.coinbase = block.author.unwrap_or_default();
                env.block.difficulty = block.difficulty;
                env.block.basefee = block.base_fee_per_gas.unwrap_or_default();
                env.block.gas_limit = block.gas_limit;
            }

            // Set the state to the moment right before the transaction
            if !self.quick {
                println!("Executing previous transactions from the block.");

                if let Some(block) = block {
                    let pb = init_progress!(block.transactions, "tx");
                    update_progress!(pb, -1);

                    for (index, tx) in block.transactions.into_iter().enumerate() {
                        if tx.hash().eq(&tx_hash) {
                            break
                        }
                        // executor.set_gas_limit(past_tx.gas);
                        configure_tx_env(&mut env, &tx);

                        if let Some(to) = tx.to {
                            env.tx.transact_to = TransactTo::Call(to);
                            executor.commit_tx_with_env(env.clone()).unwrap();
                        } else {
                            executor.deploy_with_env(env.clone(), None).unwrap();
                        }

                        update_progress!(pb, index);
                    }
                }
            }

            // Execute our transaction
            let mut result = {
                executor.set_tracing(true).set_debugger(self.debug);

                configure_tx_env(&mut env, &tx);

                if let Some(to) = tx.to {
                    env.tx.transact_to = TransactTo::Call(to);
                    let RawCallResult {
                        reverted, gas, traces, debug: run_debug, status: _, ..
                    } = executor.commit_tx_with_env(env).unwrap();

                    RunResult {
                        success: !reverted,
                        traces: vec![(TraceKind::Execution, traces.unwrap_or_default())],
                        debug: run_debug.unwrap_or_default(),
                        gas,
                    }
                } else {
                    let DeployResult { gas, traces, debug: run_debug, .. }: DeployResult =
                        executor.deploy_with_env(env, None).unwrap();

                    RunResult {
                        success: true,
                        traces: vec![(TraceKind::Execution, traces.unwrap_or_default())],
                        debug: run_debug.unwrap_or_default(),
                        gas,
                    }
                }
            };

            let etherscan_identifier =
                EtherscanIdentifier::new(&config, evm_opts.get_remote_chain_id())?;

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

            decoder
                .add_signature_identifier(SignaturesIdentifier::new(Config::foundry_cache_dir())?);

            for (_, trace) in &mut result.traces {
                decoder.identify(trace, &etherscan_identifier);
            }

            if self.debug {
                run_debugger(result, decoder)?;
            } else {
                print_traces(&mut result, decoder, self.verbose).await?;
            }
        }
        Ok(())
    }
}

/// Configures the env for the transaction
fn configure_tx_env(env: &mut forge::revm::Env, tx: &Transaction) {
    env.tx.caller = tx.from;
    env.tx.gas_limit = tx.gas.as_u64();
    env.tx.gas_price = tx.gas_price.unwrap_or_default();
    env.tx.gas_priority_fee = tx.max_priority_fee_per_gas;
    env.tx.nonce = Some(tx.nonce.as_u64());
    env.tx.access_list = tx
        .access_list
        .clone()
        .unwrap_or_default()
        .0
        .into_iter()
        .map(|item| (item.address, item.storage_keys.into_iter().map(h256_to_u256_be).collect()))
        .collect();
    env.tx.value = tx.value;
    env.tx.data = tx.input.0.clone();
}

fn run_debugger(result: RunResult, decoder: CallTraceDecoder) -> eyre::Result<()> {
    // TODO Get source from etherscan
    let calls: Vec<DebugArena> = vec![result.debug];
    let flattened = calls.last().expect("we should have collected debug info").flatten(0);
    let tui = Tui::new(flattened, 0, decoder.contracts, HashMap::new(), BTreeMap::new())?;
    match tui.start().expect("Failed to start tui") {
        TUIExitReason::CharExit => Ok(()),
    }
}

async fn print_traces(
    result: &mut RunResult,
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
            println!("{:#}", trace);
        }
    }
    println!();

    if result.success {
        println!("{}", Paint::green("Transaction successfully executed."));
    } else {
        println!("{}", Paint::red("Transaction failed."));
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
