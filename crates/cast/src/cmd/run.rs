use super::evm_context::BlockContext;
use crate::{
    debug::handle_traces,
    rpc_trace::{
        call_frame_to_arena_with_root_address, is_method_not_found_error, is_missing_state_error,
    },
    traces::TraceKind,
    utils::{apply_chain_and_block_specific_env_changes, block_env_from_header},
};
use alloy_consensus::{BlockHeader, Transaction, transaction::SignerRecoverable};

use alloy_evm::FromRecoveredTx;
use alloy_network::{BlockResponse, Network, ReceiptResponse, TransactionResponse};
use alloy_primitives::{
    Address, B256, Bytes, U256,
    map::{AddressHashMap, AddressSet},
};
use alloy_provider::{Provider, ext::DebugApi};
use alloy_rpc_types::{
    BlockId, BlockTransactions,
    trace::geth::{CallConfig, GethDebugTracingOptions, GethTrace, PreStateConfig},
};
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::{
    opts::{EtherscanOpts, RpcOpts, TracingArgs},
    utils::{TraceResult, init_progress, load_config_from_provider},
};
use foundry_common::{
    SYSTEM_TRANSACTION_TYPE, is_known_system_sender, provider::ProviderBuilder, shell,
};
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{
    Config, TracingConfig,
    figment::{
        self, Metadata, Profile,
        value::{Dict, Map},
    },
};
#[cfg(feature = "monad")]
use foundry_evm::core::evm::MonadEvmNetwork;
#[cfg(feature = "optimism")]
use foundry_evm::core::evm::OpEvmNetwork;
use foundry_evm::{
    core::{
        FoundryBlock as _,
        evm::{
            EthEvmNetwork, FoundryEvmFactory, FoundryEvmNetwork, SpecFor, TempoEvmNetwork, TxEnvFor,
        },
    },
    executors::{EvmError, Executor, TracingExecutor},
    hardforks::{ExecutionSpec, FoundryHardfork},
    opts::EvmOpts,
    traces::{InternalTraceMode, SparsedTraceArena, TraceRequirements, Traces},
};
use futures::TryFutureExt;
use revm::{DatabaseRef, context::Block, primitives::hardfork::SpecId};

/// CLI arguments for `cast run`.
#[derive(Clone, Debug, Parser)]
pub struct RunArgs {
    /// The transaction hash.
    tx_hash: String,

    /// Opens the transaction in the debugger.
    #[arg(long, short)]
    debug: bool,

    /// Print out opcode traces.
    #[arg(long, short)]
    trace_printer: bool,

    /// Executes the transaction only with the state from the previous block.
    ///
    /// May result in different results than the live execution!
    #[arg(long)]
    quick: bool,

    /// Whether to replay system transactions.
    #[arg(long, alias = "sys")]
    replay_system_txes: bool,

    /// Use debug_traceTransaction to fetch the prestate instead of replaying the block.
    ///
    /// This is significantly faster than replaying all previous transactions in the block, but
    /// requires the node to expose the `debug_` namespace (most public RPCs don't). If the call
    /// or response can't be used, cast silently falls back to replaying the block.
    #[arg(long, default_value_t = false)]
    prestate_tracer: bool,

    /// Fetch the transaction's trace from the node via `debug_traceTransaction` (callTracer) and
    /// render it, instead of re-executing the transaction locally.
    ///
    /// This skips the block replay entirely, so it is fast and reflects exactly what happened
    /// on-chain, including chain-specific EVM behavior a local replay may not reproduce, but it
    /// requires the node to expose the `debug_` namespace. The result is a call-tree view:
    /// nested calls, value, gas, emitted logs and revert data. It does not provide the
    /// opcode-level detail of a local run, so the local-execution-only flags (`--debug`,
    /// `--decode-internal`, `--trace-printer`, `--quick`, `--prestate-tracer`, `--evm-version`)
    /// do not apply.
    #[arg(
        long,
        default_value_t = false,
        conflicts_with_all = ["debug", "decode_internal", "trace_printer", "quick", "prestate_tracer", "evm_version"]
    )]
    debug_trace_transaction: bool,

    #[command(flatten)]
    tracing: TracingArgs,

    /// Deprecated short alias for `--labels`.
    #[arg(short = 'l', value_name = "ADDRESS:LABEL", hide = true)]
    legacy_labels: Vec<String>,

    #[command(flatten)]
    etherscan: EtherscanOpts,

    #[command(flatten)]
    rpc: RpcOpts,

    /// The EVM version to use.
    ///
    /// Overrides the version specified in the config.
    #[arg(long)]
    evm_version: Option<EvmVersion>,

    /// Use current project artifacts for trace decoding.
    #[arg(long, visible_alias = "la")]
    pub with_local_artifacts: bool,

    /// Disable block gas limit check.
    #[arg(long)]
    pub disable_block_gas_limit: bool,

    /// Enable the tx gas limit checks as imposed by Osaka (EIP-7825).
    #[arg(long)]
    pub enable_tx_gas_limit: bool,
}

impl RunArgs {
    fn resolve_tracing(&self, config: &TracingConfig, verbosity: u8) -> TracingConfig {
        if self.debug_trace_transaction {
            self.tracing.resolve_call_tracer(config, verbosity)
        } else {
            self.tracing.resolve(config, verbosity)
        }
    }

    /// Executes the transaction by replaying it
    ///
    /// This replays the entire block the transaction was mined in unless `quick` is set to true
    ///
    /// Note: This executes the transaction(s) as is: Cheatcodes are disabled
    pub async fn run(self) -> Result<()> {
        let figment = self.rpc.clone().into_figment(self.with_local_artifacts).merge(&self);
        let mut evm_opts = figment.extract::<EvmOpts>()?;

        // Auto-detect network from fork chain ID when not explicitly configured.
        evm_opts.infer_network_from_fork().await;

        if evm_opts.networks.is_tempo() {
            return self.run_with_evm::<TempoEvmNetwork>(evm_opts).await;
        }

        #[cfg(feature = "monad")]
        if evm_opts.networks.is_monad() {
            return self.run_with_evm::<MonadEvmNetwork>(evm_opts).await;
        }

        #[cfg(feature = "optimism")]
        if evm_opts.networks.is_optimism() {
            return self.run_with_evm::<OpEvmNetwork>(evm_opts).await;
        }

        self.run_with_evm::<EthEvmNetwork>(evm_opts).await
    }

    async fn run_with_evm<FEN: FoundryEvmNetwork>(mut self, evm_opts: EvmOpts) -> Result<()> {
        let figment = self.rpc.clone().into_figment(self.with_local_artifacts).merge(&self);
        let mut config = load_config_from_provider(figment)?;
        config.networks = evm_opts.networks;
        self.tracing.labels.append(&mut self.legacy_labels);
        let tracing = self.resolve_tracing(&config.tracing, shell::verbosity());

        let with_local_artifacts = self.with_local_artifacts;
        let debug = self.debug;
        let compute_units_per_second = if self.rpc.common.no_rpc_rate_limit {
            Some(u64::MAX)
        } else {
            self.rpc.common.compute_units_per_second
        };

        let provider = ProviderBuilder::<FEN::Network>::from_config(&config)?
            .compute_units_per_second_opt(compute_units_per_second)
            .build()?;

        let tx_hash = self.tx_hash.parse().wrap_err("invalid tx hash")?;
        let tx = provider
            .get_transaction_by_hash(tx_hash)
            .await
            .wrap_err_with(|| format!("tx not found: {tx_hash:?}"))?
            .ok_or_else(|| eyre::eyre!("tx not found: {:?}", tx_hash))?;

        // Fetch the trace from the node via `debug_traceTransaction` (callTracer) instead of
        // re-executing the transaction locally. The node already holds the transaction's exact
        // pre-state and EVM rules, so this needs no block replay and no local executor; it also
        // handles system transactions, so this path comes before the system transaction guard.
        if self.debug_trace_transaction {
            let tx_block_number = tx
                .block_number()
                .ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?;

            let geth_trace = provider
                .debug_trace_transaction(
                    tx_hash,
                    GethDebugTracingOptions::call_tracer(CallConfig::default().with_log()),
                )
                .await
                .map_err(|err| -> eyre::Report {
                    // Two RPC rejections deserve an actionable hint instead of the raw transport
                    // error, and they need different fixes: a disabled `debug` namespace, and
                    // missing historical state, hit whenever the transaction's block has been
                    // pruned by a full node.
                    if is_method_not_found_error(&err) {
                        eyre::eyre!(
                            "the RPC endpoint does not support `debug_traceTransaction` (method not found); use a node with the `debug` namespace enabled (e.g. a local anvil/reth or an archive endpoint), or drop `--debug-trace-transaction` to re-execute the transaction locally"
                        )
                    } else if is_missing_state_error(&err) {
                        eyre::eyre!(
                            "the RPC endpoint does not have the historical state for the transaction's block; use an archive endpoint"
                        )
                    } else {
                        err.into()
                    }
                })?;
            let GethTrace::CallTracer(frame) = geth_trace else {
                eyre::bail!(
                    "`debug_traceTransaction` did not return a callTracer frame; the RPC endpoint \
                     may not support the `callTracer`"
                );
            };

            let receipt = provider
                .get_transaction_receipt(tx_hash)
                .await?
                .ok_or_else(|| eyre::eyre!("tx receipt not found: {:?}", tx_hash))?;

            let success = receipt.status();
            let gas_used = receipt.gas_used();
            let root_create_address = Transaction::to(&tx).is_none().then(|| {
                receipt.contract_address().unwrap_or_else(|| tx.from().create(tx.nonce()))
            });
            let arena = SparsedTraceArena {
                arena: call_frame_to_arena_with_root_address(&frame, root_create_address),
                ignored: Default::default(),
            };
            let result = TraceResult {
                success,
                traces: Some(vec![(TraceKind::Execution, arena)]),
                gas_used,
            };

            // Local-artifact labeling matches deployed runtime bytecode against the project
            // artifacts. There is no local executor on this path, so fetch the code over RPC
            // for the addresses in the trace, at the transaction's block. Skip the extra
            // round-trips unless local artifacts were requested.
            let contracts_bytecode = if with_local_artifacts {
                fetch_transaction_contracts_bytecode_via_rpc(
                    &provider,
                    &result,
                    tx_hash,
                    tx_block_number.into(),
                )
                .await?
            } else {
                Default::default()
            };

            let chain = alloy_chains::Chain::from_id(provider.get_chain_id().await?);
            handle_traces(
                result,
                &config,
                chain,
                &contracts_bytecode,
                &tracing,
                with_local_artifacts,
                false,
                config.hardfork.and_then(|hardfork| match hardfork {
                    FoundryHardfork::Tempo(hardfork) => Some(hardfork),
                    _ => None,
                }),
            )
            .await?;

            return Ok(());
        }

        // check if the tx is a system transaction
        if !self.replay_system_txes
            && (is_known_system_sender(tx.from())
                || tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE))
        {
            return Err(eyre::eyre!(
                "{:?} is a system transaction.\nReplaying system transactions is currently not supported.",
                tx.tx_hash()
            ));
        }

        let tx_block_number = tx
            .block_number()
            .ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?;

        // we need to fork off the parent block
        config.fork_block_number = Some(tx_block_number - 1);

        let create2_deployer = evm_opts.create2_deployer;
        let verbosity = tracing.verbosity;
        let (block, (mut evm_env, tx_env, fork, chain, networks)) = tokio::try_join!(
            // fetch the block the transaction was mined in
            provider.get_block(tx_block_number.into()).full().into_future().map_err(Into::into),
            TracingExecutor::<FEN>::get_fork_material(&mut config, evm_opts)
        )?;

        let mut evm_version = self.evm_version;
        let mut resolved_tempo_hardfork = config
            .hardfork
            .and_then(|hardfork| match hardfork {
                FoundryHardfork::Tempo(hardfork) => Some(hardfork),
                _ => None,
            })
            .or_else(|| (networks.is_tempo() || chain.is_tempo()).then(|| config.evm_spec_id()));

        evm_env.cfg_env.disable_block_gas_limit = self.disable_block_gas_limit;

        // By default do not enforce transaction gas limits imposed by Osaka (EIP-7825).
        // Users can opt-in to enable these limits by setting `enable_tx_gas_limit` to true.
        if !self.enable_tx_gas_limit {
            evm_env.cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        }

        evm_env.cfg_env.limit_contract_code_size = None;
        evm_env.block_env.set_number(U256::from(tx_block_number));
        let configured_spec =
            config.hardfork.and_then(<SpecFor<FEN> as ExecutionSpec>::from_foundry_hardfork);
        if let Some(spec) = configured_spec {
            evm_env.cfg_env.set_spec_and_mainnet_gas_params(spec);
        }

        let mut parent_beacon_block_root = None;
        if let Some(block) = &block {
            evm_env.block_env = block_env_from_header(block.header());
            parent_beacon_block_root = block.header().parent_beacon_block_root();

            // Unless explicitly configured, resolve the correct spec for the block using the same
            // approach as reth: walk known chain activation conditions to find the latest active
            // fork. Falls back to a blob-gas heuristic for unknown chains.
            if evm_version.is_none() && configured_spec.is_none() {
                if let Some(hardfork) = FoundryHardfork::from_chain_and_timestamp(
                    evm_env.cfg_env.chain_id,
                    block.header().timestamp(),
                ) {
                    if let FoundryHardfork::Tempo(hardfork) = hardfork {
                        resolved_tempo_hardfork = Some(hardfork);
                    }
                    evm_env.cfg_env.set_spec_and_mainnet_gas_params(hardfork.into());
                } else if block.header().excess_blob_gas().is_some() {
                    // TODO: add glamsterdam header field checks in the future
                    evm_version = Some(EvmVersion::Cancun);
                }
            }
            apply_chain_and_block_specific_env_changes::<FEN::Network, _, _>(
                &mut evm_env,
                block,
                config.networks,
            );
        }

        let block_context = if FEN::EvmFactory::NEEDS_BLOCK_CONTEXT {
            let block = block.as_ref().ok_or_else(|| {
                eyre::eyre!(
                    "block {tx_block_number} is required to reconstruct transaction context"
                )
            })?;
            Some(BlockContext::<FEN>::fetch(&provider, block).await?)
        } else {
            None
        };

        let trace_requirements = TraceRequirements::none()
            .with_calls(true)
            .with_debug(self.debug)
            .with_decode_internal(if tracing.decode_internal {
                InternalTraceMode::Full
            } else {
                InternalTraceMode::None
            })
            .with_state_changes(verbosity > 4);
        let mut executor = TracingExecutor::<FEN>::new(
            (evm_env.clone(), tx_env),
            fork,
            evm_version,
            trace_requirements,
            networks,
            create2_deployer,
            None,
        )?;

        evm_env.cfg_env.set_spec_and_mainnet_gas_params(executor.spec_id());

        let spec_id = (*evm_env.cfg_env.spec()).into();

        if let Some(parent_beacon_block_root) =
            parent_beacon_block_root_for_spec(spec_id, parent_beacon_block_root)?
        {
            executor.apply_beacon_root(parent_beacon_block_root)?;
        }

        // Set the state to the moment right before the transaction.
        //
        // When `--prestate-tracer` is set, opportunistically try to fetch the prestate directly
        // via `debug_traceTransaction` (much faster than replaying the block). This requires the
        // `debug_` namespace, which most nodes don't expose, so it is opt-in and silently falls
        // back to replaying previous transactions in the block if the call or parsing fails.
        let mut prestate_applied = false;
        if !self.quick && self.prestate_tracer {
            trace!(?tx_hash, "attempting to fetch prestate via debug_traceTransaction");
            match provider
                .debug_trace_transaction(
                    tx_hash,
                    GethDebugTracingOptions::prestate_tracer(PreStateConfig::default()),
                )
                .await
            {
                Ok(trace) => match trace.try_into_pre_state_frame() {
                    Ok(pre_state_frame) => {
                        executor.apply_prestate_trace(pre_state_frame.into_pre_state())?;
                        prestate_applied = true;
                        trace!("prestate trace applied successfully, skipping block replay");
                    }
                    Err(err) => {
                        trace!(%err, "failed to parse prestate trace response");
                    }
                },
                Err(err) => {
                    trace!(?err, "debug_traceTransaction failed, falling back to block replay");
                }
            }
        }

        // Fall back to replaying previous transactions if prestate trace wasn't applied.
        if !self.quick && !prestate_applied {
            sh_status!("Executing previous transactions from the block.")?;

            if let Some(block) = &block {
                let pb = init_progress(block.transactions().len() as u64, "tx");
                pb.set_position(0);

                let BlockTransactions::Full(ref txs) = *block.transactions() else {
                    return Err(eyre::eyre!("Could not get block txs"));
                };

                for (index, tx) in txs.iter().enumerate() {
                    // Replay system transactions only if running with `sys` option.
                    // System transactions such as on L2s don't contain any pricing info so it
                    // could cause reverts.
                    if !self.replay_system_txes
                        && (is_known_system_sender(tx.from())
                            || tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE))
                    {
                        pb.set_position((index + 1) as u64);
                        continue;
                    }
                    if tx.tx_hash() == tx_hash {
                        break;
                    }

                    let tx_env = TxEnvFor::<FEN>::from_recovered_tx(tx.as_ref(), tx.from());
                    let context_aux = block_context.as_ref().map_or_else(
                        || FEN::EvmFactory::default().context_for_transaction(&tx_env),
                        |context| context.transaction(index),
                    );

                    evm_env.cfg_env.disable_balance_check = true;

                    if let Some(to) = Transaction::to(tx) {
                        trace!(tx=?tx.tx_hash(),?to, "executing previous call transaction");
                        executor
                            .transact_with_env_and_context(
                                evm_env.clone(),
                                tx_env.clone(),
                                context_aux,
                            )
                            .wrap_err_with(|| {
                                format!(
                                    "Failed to execute transaction: {:?} in block {}",
                                    tx.tx_hash(),
                                    evm_env.block_env.number()
                                )
                            })?;
                    } else {
                        trace!(tx=?tx.tx_hash(), "executing previous create transaction");
                        if let Err(error) = executor.deploy_with_env_and_context(
                            evm_env.clone(),
                            tx_env.clone(),
                            context_aux,
                            None,
                        ) {
                            match error {
                                // Reverted transactions should be skipped
                                EvmError::Execution(_) => (),
                                error => {
                                    return Err(error).wrap_err_with(|| {
                                        format!(
                                            "Failed to deploy transaction: {:?} in block {}",
                                            tx.tx_hash(),
                                            evm_env.block_env.number()
                                        )
                                    });
                                }
                            }
                        }
                    }

                    pb.set_position((index + 1) as u64);
                }
            }
        }

        // Execute our transaction
        let result = {
            executor.set_trace_printer(self.trace_printer);

            let tx_env = TxEnvFor::<FEN>::from_recovered_tx(tx.as_ref(), tx.from());
            let target_index = if let Some(block) = &block {
                let BlockTransactions::Full(transactions) = block.transactions() else {
                    return Err(eyre::eyre!("Could not get block txs"));
                };
                transactions
                    .iter()
                    .position(|candidate| candidate.tx_hash() == tx_hash)
                    .ok_or_else(|| {
                        eyre::eyre!("transaction {tx_hash:?} is missing from its block")
                    })?
            } else {
                0
            };
            let context_aux = block_context.as_ref().map_or_else(
                || FEN::EvmFactory::default().context_for_transaction(&tx_env),
                |context| context.transaction(target_index),
            );

            if tx.as_ref().recover_signer().is_ok_and(|signer| signer != tx.from()) {
                evm_env.cfg_env.disable_balance_check = true;
            }

            if let Some(to) = Transaction::to(&tx) {
                trace!(tx=?tx.tx_hash(), to=?to, "executing call transaction");
                TraceResult::from(executor.transact_with_env_and_context(
                    evm_env,
                    tx_env,
                    context_aux,
                )?)
            } else {
                trace!(tx=?tx.tx_hash(), "executing create transaction");
                TraceResult::try_from(executor.deploy_with_env_and_context(
                    evm_env,
                    tx_env,
                    context_aux,
                    None,
                ))?
            }
        };

        let contracts_bytecode = fetch_contracts_bytecode_from_trace(&executor, &result)?;
        handle_traces(
            result,
            &config,
            chain,
            &contracts_bytecode,
            &tracing,
            with_local_artifacts,
            debug,
            resolved_tempo_hardfork,
        )
        .await?;

        Ok(())
    }
}

fn parent_beacon_block_root_for_spec(
    spec_id: SpecId,
    parent_beacon_block_root: Option<B256>,
) -> Result<Option<B256>> {
    if !spec_id.is_enabled_in(SpecId::CANCUN) {
        return Ok(None);
    }

    parent_beacon_block_root.map(Some).ok_or_else(|| {
        eyre::eyre!(
            "MissingParentBeaconBlockRoot: missing parent beacon block root for Cancun block"
        )
    })
}

pub fn fetch_contracts_bytecode_from_trace<FEN: FoundryEvmNetwork>(
    executor: &Executor<FEN>,
    result: &TraceResult,
) -> Result<AddressHashMap<Bytes>> {
    let mut contracts_bytecode = AddressHashMap::default();
    if let Some(ref traces) = result.traces {
        contracts_bytecode.extend(gather_trace_addresses(traces).filter_map(|addr| {
            // All relevant bytecodes should already be cached in the executor.
            let code = executor
                .backend()
                .basic_ref(addr)
                .inspect_err(|e| _ = sh_warn!("Failed to fetch code for {addr}: {e}"))
                .ok()??
                .code?
                .bytes();
            if code.is_empty() {
                return None;
            }
            Some((addr, code))
        }));
    }
    Ok(contracts_bytecode)
}

/// Fetches the runtime bytecode of the addresses seen in `result` over RPC.
///
/// The RPC trace path (`cast call --debug-trace-call`) has no local executor to read code
/// from, so the bytecode needed to match local artifacts is fetched from the node with
/// `eth_getCode`. Addresses whose code cannot be fetched are skipped with a warning.
pub async fn fetch_contracts_bytecode_via_rpc<N: Network, P: Provider<N>>(
    provider: &P,
    result: &TraceResult,
    block: BlockId,
) -> Result<AddressHashMap<Bytes>> {
    let mut contracts_bytecode = AddressHashMap::default();
    if let Some(ref traces) = result.traces {
        for addr in gather_trace_addresses(traces) {
            match provider.get_code_at(addr).block_id(block).await {
                Ok(code) if !code.is_empty() => {
                    contracts_bytecode.insert(addr, code);
                }
                Ok(_) => {}
                Err(err) => {
                    let _ = sh_warn!("Failed to fetch code for {addr}: {err}");
                }
            }
        }
    }
    Ok(contracts_bytecode)
}

/// Fetches bytecode for a mined transaction at its exact transaction index.
///
/// The prestate tracer provides the code that existed immediately before the transaction, which
/// avoids reading end-of-block state for contracts changed or removed by later transactions. Any
/// address absent from the prestate (for example, a contract created by this transaction) falls
/// back to `eth_getCode` at the transaction's block.
async fn fetch_transaction_contracts_bytecode_via_rpc<N: Network, P: Provider<N>>(
    provider: &P,
    result: &TraceResult,
    tx_hash: B256,
    block: BlockId,
) -> Result<AddressHashMap<Bytes>> {
    let mut contracts_bytecode = AddressHashMap::default();
    let prestate_config = PreStateConfig { disable_storage: Some(true), ..Default::default() };
    match provider
        .debug_trace_transaction(tx_hash, GethDebugTracingOptions::prestate_tracer(prestate_config))
        .await
    {
        Ok(trace) => match trace.try_into_pre_state_frame() {
            Ok(prestate) => {
                for (&address, account) in prestate.pre_state() {
                    if let Some(code) = account.code.clone().filter(|code| !code.is_empty()) {
                        contracts_bytecode.insert(address, code);
                    }
                }
            }
            Err(err) => {
                let _ = sh_warn!("Failed to parse transaction prestate for local artifacts: {err}");
            }
        },
        Err(err) => {
            let _ = sh_warn!("Failed to fetch transaction prestate for local artifacts: {err}");
        }
    }

    if let Some(ref traces) = result.traces {
        for address in gather_trace_addresses(traces) {
            if contracts_bytecode.contains_key(&address) {
                continue;
            }
            match provider.get_code_at(address).block_id(block).await {
                Ok(code) if !code.is_empty() => {
                    contracts_bytecode.insert(address, code);
                }
                Ok(_) => {}
                Err(err) => {
                    let _ = sh_warn!("Failed to fetch code for {address}: {err}");
                }
            }
        }
    }
    Ok(contracts_bytecode)
}

fn gather_trace_addresses(traces: &Traces) -> impl Iterator<Item = Address> {
    let mut addresses = AddressSet::default();
    for (_, trace) in traces {
        for node in trace.arena.nodes() {
            if !node.trace.address.is_zero() {
                addresses.insert(node.trace.address);
            }
            if !node.trace.caller.is_zero() {
                addresses.insert(node.trace.caller);
            }
        }
    }
    addresses.into_iter()
}

impl figment::Provider for RunArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("RunArgs")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut map = Map::new();

        if let Some(api_key) = &self.etherscan.key {
            map.insert("etherscan_api_key".into(), api_key.as_str().into());
        }

        if let Some(evm_version) = self.evm_version {
            map.insert("evm_version".into(), figment::value::Value::serialize(evm_version)?);
        }

        Ok(Map::from([(Config::selected_profile(), map)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn parses_legacy_short_label_alias() {
        let address = address!("0x0000000000000000000000000000000000000001");
        let label = format!("{address}:alice");
        let args = RunArgs::parse_from(["cast run", "0x00", "-l", &label]);

        assert_eq!(args.legacy_labels, vec![label]);
    }

    #[test]
    fn debug_trace_transaction_rejects_local_execution_flags() {
        for flag in
            ["--debug", "--decode-internal", "--trace-printer", "--quick", "--prestate-tracer"]
        {
            let result = RunArgs::try_parse_from([
                "foundry-cli",
                "--debug-trace-transaction",
                "0x0000000000000000000000000000000000000000000000000000000000000000",
                flag,
            ]);
            assert!(result.is_err(), "--debug-trace-transaction must reject {flag}");
        }
        // --evm-version takes a value, so it is checked separately from the boolean flags above.
        let result = RunArgs::try_parse_from([
            "foundry-cli",
            "--debug-trace-transaction",
            "0x0000000000000000000000000000000000000000000000000000000000000000",
            "--evm-version",
            "shanghai",
        ]);
        assert!(result.is_err(), "--debug-trace-transaction must reject --evm-version");
    }

    #[test]
    fn debug_trace_transaction_accepts_label_and_render_flags() {
        let args = RunArgs::try_parse_from([
            "foundry-cli",
            "--debug-trace-transaction",
            "0x0000000000000000000000000000000000000000000000000000000000000000",
            "--label",
            "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045:vitalik.eth",
            "--disable-labels",
            "--trace-depth",
            "2",
            "--with-local-artifacts",
        ]);
        assert!(args.is_ok(), "--debug-trace-transaction must accept label/rendering flags");
    }

    #[test]
    fn parent_beacon_block_root_is_required_for_cancun() {
        let err = parent_beacon_block_root_for_spec(SpecId::CANCUN, None).unwrap_err();
        assert!(err.to_string().contains("MissingParentBeaconBlockRoot"));

        let root = B256::repeat_byte(0x42);
        assert_eq!(
            parent_beacon_block_root_for_spec(SpecId::CANCUN, Some(root)).unwrap(),
            Some(root),
        );
        assert_eq!(parent_beacon_block_root_for_spec(SpecId::SHANGHAI, Some(root)).unwrap(), None);
        assert_eq!(parent_beacon_block_root_for_spec(SpecId::SHANGHAI, None).unwrap(), None);
    }

    #[test]
    fn debug_trace_transaction_ignores_configured_internal_decoding() {
        let args = RunArgs::parse_from(["cast run", "0x00", "--debug-trace-transaction"]);
        let config = TracingConfig { decode_internal: true, ..Default::default() };

        assert!(!args.resolve_tracing(&config, 0).decode_internal);
    }
}
