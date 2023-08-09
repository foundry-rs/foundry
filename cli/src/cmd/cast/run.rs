use crate::{init_progress, opts::RpcOpts, update_progress, utils};
use cast::{
    executor::{EvmError, ExecutionErr},
    trace::{identifier::SignaturesIdentifier, CallTraceDecoder, Traces},
};
use clap::Parser;
use ethers::{
    abi::Address,
    prelude::{artifacts::ContractBytecodeSome, ArtifactId, Middleware},
    solc::EvmVersion,
    types::H160,
};
use eyre::WrapErr;
use forge::{
    debug::DebugArena,
    executor::{
        inspector::cheatcodes::util::configure_tx_env, opts::EvmOpts, Backend, DeployResult,
        ExecutorBuilder, RawCallResult,
    },
    revm::primitives::U256 as rU256,
    trace::{identifier::EtherscanIdentifier, CallTraceDecoderBuilder, TraceKind},
    utils::h256_to_b256,
};
use foundry_config::{find_project_root_path, Config};
use foundry_evm::utils::evm_spec;
use std::{collections::BTreeMap, str::FromStr};
use tracing::trace;
use ui::{TUIExitReason, Tui, Ui};
use yansi::Paint;

const ARBITRUM_SENDER: H160 = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x0a, 0x4b, 0x05,
]);

/// CLI arguments for `cast run`.
#[derive(Debug, Clone, Parser)]
pub struct RunArgs {
    /// The transaction hash.
    tx_hash: String,

    /// Opens the transaction in the debugger.
    #[clap(long, short)]
    debug: bool,

    /// Print out opcode traces.
    #[clap(long, short)]
    trace_printer: bool,

    /// Executes the transaction only with the state from the previous block.
    ///
    /// May result in different results than the live execution!
    #[clap(long, short)]
    quick: bool,

    /// Prints the full address of the contract.
    #[clap(long, short)]
    verbose: bool,

    /// Label addresses in the trace.
    ///
    /// Example: 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045:vitalik.eth
    #[clap(long, short)]
    label: Vec<String>,

    #[clap(flatten)]
    rpc: RpcOpts,

    /// The evm version to use.
    ///
    /// Overrides the version specified in the config.
    #[clap(long, short)]
    evm_version: Option<EvmVersion>,
    /// Sets the number of assumed available compute units per second for this provider
    ///
    /// default value: 330
    ///
    /// See also, https://github.com/alchemyplatform/alchemy-docs/blob/master/documentation/compute-units.md#rate-limits-cups
    #[clap(long, alias = "cups", value_name = "CUPS")]
    pub compute_units_per_second: Option<u64>,

    /// Disables rate limiting for this node's provider.
    ///
    /// default value: false
    ///
    /// See also, https://github.com/alchemyplatform/alchemy-docs/blob/master/documentation/compute-units.md#rate-limits-cups
    #[clap(long, value_name = "NO_RATE_LIMITS", visible_alias = "no-rpc-rate-limit")]
    pub no_rate_limit: bool,
}

impl RunArgs {
    /// Executes the transaction by replaying it
    ///
    /// This replays the entire block the transaction was mined in unless `quick` is set to true
    ///
    /// Note: This executes the transaction(s) as is: Cheatcodes are disabled
    pub async fn run(self) -> eyre::Result<()> {
        let figment =
            Config::figment_with_root(find_project_root_path(None).unwrap()).merge(self.rpc);
        let mut evm_opts = figment.extract::<EvmOpts>()?;
        let config = Config::from_provider(figment).sanitized();

        let compute_units_per_second =
            if self.no_rate_limit { Some(u64::MAX) } else { self.compute_units_per_second };

        let provider = utils::get_provider_builder(&config)?
            .compute_units_per_second_opt(compute_units_per_second)
            .build()?;

        let tx_hash = self.tx_hash.parse().wrap_err("invalid tx hash")?;
        let tx = provider
            .get_transaction(tx_hash)
            .await?
            .ok_or_else(|| eyre::eyre!("tx not found: {:?}", tx_hash))?;

        let tx_block_number = tx
            .block_number
            .ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?
            .as_u64();
        evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());
        // we need to set the fork block to the previous block, because that's the state at
        // which we access the data in order to execute the transaction(s)
        evm_opts.fork_block_number = Some(tx_block_number - 1);

        // Set up the execution environment
        let mut env = evm_opts.evm_env().await?;
        // can safely disable base fee checks on replaying txs because can
        // assume those checks already passed on confirmed txs
        env.cfg.disable_base_fee = true;
        let db = Backend::spawn(evm_opts.get_fork(&config, env.clone())).await;

        // configures a bare version of the evm executor: no cheatcode inspector is enabled,
        // tracing will be enabled only for the targeted transaction
        let builder = ExecutorBuilder::default()
            .with_config(env)
            .with_spec(evm_spec(&self.evm_version.unwrap_or(config.evm_version)));

        let mut executor = builder.build(db);

        let mut env = executor.env().clone();
        env.block.number = rU256::from(tx_block_number);

        let block = provider.get_block_with_txs(tx_block_number).await?;
        if let Some(ref block) = block {
            env.block.timestamp = block.timestamp.into();
            env.block.coinbase = block.author.unwrap_or_default().into();
            env.block.difficulty = block.difficulty.into();
            env.block.prevrandao = block.mix_hash.map(h256_to_b256);
            env.block.basefee = block.base_fee_per_gas.unwrap_or_default().into();
            env.block.gas_limit = block.gas_limit.into();
        }

        // Set the state to the moment right before the transaction
        if !self.quick {
            println!("Executing previous transactions from the block.");

            if let Some(block) = block {
                let pb = init_progress!(block.transactions, "tx");
                pb.set_position(0);

                for (index, tx) in block.transactions.into_iter().enumerate() {
                    // arbitrum L1 transaction at the start of every block that has gas price 0
                    // and gas limit 0 which causes reverts, so we skip it
                    if tx.from == ARBITRUM_SENDER {
                        update_progress!(pb, index);
                        continue
                    }
                    if tx.hash().eq(&tx_hash) {
                        break
                    }

                    configure_tx_env(&mut env, &tx);

                    if let Some(to) = tx.to {
                        trace!(tx=?tx.hash,?to, "executing previous call transaction");
                        executor.commit_tx_with_env(env.clone()).wrap_err_with(|| {
                            format!("Failed to execute transaction: {:?}", tx.hash())
                        })?;
                    } else {
                        trace!(tx=?tx.hash, "executing previous create transaction");
                        if let Err(error) = executor.deploy_with_env(env.clone(), None) {
                            match error {
                                // Reverted transactions should be skipped
                                EvmError::Execution(_) => (),
                                error => {
                                    return Err(error).wrap_err_with(|| {
                                        format!("Failed to deploy transaction: {:?}", tx.hash())
                                    })
                                }
                            }
                        }
                    }

                    update_progress!(pb, index);
                }
            }
        }

        // Execute our transaction
        let mut result = {
            executor
                .set_tracing(true)
                .set_debugger(self.debug)
                .set_trace_printer(self.trace_printer);

            configure_tx_env(&mut env, &tx);

            if let Some(to) = tx.to {
                trace!(tx=?tx.hash,to=?to, "executing call transaction");
                let RawCallResult {
                    reverted,
                    gas_used: gas,
                    traces,
                    debug: run_debug,
                    exit_reason: _,
                    ..
                } = executor.commit_tx_with_env(env)?;

                RunResult {
                    success: !reverted,
                    traces: vec![(TraceKind::Execution, traces.unwrap_or_default())],
                    debug: run_debug.unwrap_or_default(),
                    gas_used: gas,
                }
            } else {
                trace!(tx=?tx.hash, "executing create transaction");
                match executor.deploy_with_env(env, None) {
                    Ok(DeployResult { gas_used, traces, debug: run_debug, .. }) => RunResult {
                        success: true,
                        traces: vec![(TraceKind::Execution, traces.unwrap_or_default())],
                        debug: run_debug.unwrap_or_default(),
                        gas_used,
                    },
                    Err(EvmError::Execution(inner)) => {
                        let ExecutionErr { reverted, gas_used, traces, debug: run_debug, .. } =
                            *inner;
                        RunResult {
                            success: !reverted,
                            traces: vec![(TraceKind::Execution, traces.unwrap_or_default())],
                            debug: run_debug.unwrap_or_default(),
                            gas_used,
                        }
                    }
                    Err(err) => {
                        eyre::bail!("unexpected error when running create transaction: {:?}", err)
                    }
                }
            }
        };

        let mut etherscan_identifier =
            EtherscanIdentifier::new(&config, evm_opts.get_remote_chain_id())?;

        let labeled_addresses = self.label.iter().filter_map(|label_str| {
            let mut iter = label_str.split(':');

            if let Some(addr) = iter.next() {
                if let (Ok(address), Some(label)) = (Address::from_str(addr), iter.next()) {
                    return Some((address, label.to_string()))
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

        if self.debug {
            let (sources, bytecode) = etherscan_identifier.get_compiled_contracts().await?;
            run_debugger(result, decoder, bytecode, sources)?;
        } else {
            print_traces(&mut result, decoder, self.verbose).await?;
        }
        Ok(())
    }
}

fn run_debugger(
    result: RunResult,
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

struct RunResult {
    pub success: bool,
    pub traces: Traces,
    pub debug: DebugArena,
    pub gas_used: u64,
}
