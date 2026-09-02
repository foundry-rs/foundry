use crate::{
    MAX_CONCURRENT_RPC_REQUESTS,
    debug::{ensure_remote_trace_context_unchanged, handle_traces, select_remote_trace_hardfork},
    rpc_trace::{
        call_frame_to_arena_with_root_address, is_method_not_found_error, is_missing_state_error,
    },
    traces::TraceKind,
    utils::{
        apply_chain_and_block_specific_env_changes_for_chain,
        apply_chain_specific_tx_replay_env_changes_for_chain, block_env_from_header,
    },
};
use alloy_chains::Chain;
use alloy_consensus::{BlockHeader, Transaction, transaction::SignerRecoverable};
use alloy_eips::BlockNumHash;
use alloy_network::{
    AnyNetwork, AnyRpcBlock, AnyRpcTransaction, AnyTxEnvelope, BlockResponse, Network,
    ReceiptResponse, TransactionResponse, primitives::HeaderResponse,
};
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
    utils::{TraceResult, init_progress},
};
use foundry_common::{
    SYSTEM_TRANSACTION_TYPE, is_known_system_sender,
    provider::{ProviderBuilder, RetryProvider},
    shell,
};
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{
    Config, TracingConfig,
    figment::{
        self, Metadata, Profile,
        value::{Dict, Map},
    },
};
#[cfg(feature = "optimism")]
use foundry_evm::core::evm::OpEvmNetwork;
#[cfg(feature = "monad")]
use foundry_evm::core::evm::{BlockContext, ChainFor, MonadEvmNetwork};
use foundry_evm::{
    core::{
        FoundryBlock as _,
        env::FromAnyRpcTransaction as _,
        evm::{EthEvmNetwork, EvmEnvFor, FoundryEvmNetwork, TempoEvmNetwork, TxEnvFor},
    },
    executors::{Executor, ExecutorBuilder, TracingExecutor},
    hardforks::FoundryHardfork,
    opts::EvmOpts,
    traces::{InternalTraceMode, SparsedTraceArena, TraceRequirements, Traces},
};
use foundry_evm_networks::NetworkConfigs;
use futures::{StreamExt, TryFutureExt};
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
    ///
    /// Always implied: a mined transaction already passed its chain's own check.
    #[arg(long)]
    pub disable_block_gas_limit: bool,

    /// Enable the tx gas limit checks as imposed by Osaka (EIP-7825).
    #[arg(long)]
    pub enable_tx_gas_limit: bool,
}

/// Target transaction resolved up front by [`RunArgs::fetch_target`], before any per-network
/// preparation.
struct TargetFetch {
    tx: AnyRpcTransaction,
    provider: RetryProvider,
    compute_units_per_second: Option<u64>,
    target_is_system: bool,
}

/// Fields only needed by the Monad-specific `execute_monad`/`execute_monad_target` path.
#[cfg(feature = "monad")]
struct MonadPrepared {
    tx_block_number: u64,
    compute_units_per_second: Option<u64>,
}

/// State assembled by [`RunArgs::prepare`] and consumed by the network-specific `execute_*`
/// methods below.
struct PreparedRun<FEN: FoundryEvmNetwork> {
    args: RunArgs,
    config: Box<Config>,
    tracing: TracingConfig,
    tx: AnyRpcTransaction,
    block: Option<AnyRpcBlock>,
    evm_env: EvmEnvFor<FEN>,
    executor: TracingExecutor<FEN>,
    chain: Chain,
    networks: NetworkConfigs,
    resolved_hardfork: Option<FoundryHardfork>,
    verbosity: u8,
    prestate_applied: bool,
    #[cfg(feature = "monad")]
    monad: MonadPrepared,
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
        let (mut config, mut evm_opts) = super::load_cast_config_and_evm_opts(figment)?;
        if config.eth_rpc_url.is_none()
            && let Some(chain) = self.etherscan.chain
        {
            let alias = chain.to_string();
            if config.rpc_endpoints.contains_key(&alias) {
                config.eth_rpc_url = Some(alias);
            }
        }
        evm_opts.fork_url = Some(config.get_rpc_url_or_localhost_http()?.into_owned());

        // Auto-detect network from fork chain ID when not explicitly configured.
        evm_opts.infer_network_from_fork().await?;

        if evm_opts.networks.is_tempo() {
            return self
                .run_with_evm(config, evm_opts, ExecutorBuilder::<TempoEvmNetwork>::new())
                .await;
        }

        #[cfg(feature = "base")]
        if evm_opts.networks.is_base() {
            return self
                .run_with_evm(
                    config,
                    evm_opts,
                    ExecutorBuilder::<foundry_evm::core::evm::BaseEvmNetwork>::new(),
                )
                .await;
        }

        #[cfg(feature = "monad")]
        if evm_opts.networks.is_monad() {
            return self.run_with_monad(config, evm_opts).await;
        }

        #[cfg(feature = "optimism")]
        if evm_opts.networks.is_optimism() {
            return self
                .run_with_evm(config, evm_opts, ExecutorBuilder::<OpEvmNetwork>::new())
                .await;
        }

        self.run_with_evm(config, evm_opts, ExecutorBuilder::<EthEvmNetwork>::new()).await
    }

    async fn run_with_evm<FEN: FoundryEvmNetwork>(
        self,
        config: Box<Config>,
        evm_opts: EvmOpts,
        executor_builder: ExecutorBuilder<FEN>,
    ) -> Result<()> {
        let target = self.fetch_target(&config).await?;
        if target.target_is_system && !self.replay_system_txes && !self.debug_trace_transaction {
            eyre::bail!(
                "{:?} is a system transaction.\nReplaying system transactions is currently not supported.",
                target.tx.tx_hash()
            );
        }
        let Some(mut run) = self.prepare::<FEN>(config, evm_opts, target, executor_builder).await?
        else {
            return Ok(());
        };
        let result = run.execute_ordinary()?;
        run.finish(result).await
    }

    #[cfg(feature = "monad")]
    async fn run_with_monad(self, config: Box<Config>, evm_opts: EvmOpts) -> Result<()> {
        let target = self.fetch_target(&config).await?;
        let Some(mut run) = self
            .prepare::<MonadEvmNetwork>(
                config,
                evm_opts,
                target,
                ExecutorBuilder::<MonadEvmNetwork>::new(),
            )
            .await?
        else {
            return Ok(());
        };
        let result = run.execute_monad().await?;
        run.finish(result).await
    }

    /// `AnyNetwork` rather than `FEN::Network`: chains such as Arbitrum, Celo and the OP-stack
    /// forks Foundry does not route to a dedicated network put transaction types the strict
    /// Ethereum envelope cannot decode into every block, which would fail the full block fetch in
    /// `prepare` for the whole chain. Execution still uses `FEN`.
    async fn fetch_target(&self, config: &Config) -> Result<TargetFetch> {
        let compute_units_per_second = if self.rpc.common.no_rpc_rate_limit {
            Some(u64::MAX)
        } else {
            self.rpc.common.compute_units_per_second
        };
        let provider = ProviderBuilder::<AnyNetwork>::from_config(config)?
            .compute_units_per_second_opt(compute_units_per_second)
            .build()?;
        let tx_hash = self.tx_hash.parse().wrap_err("invalid tx hash")?;
        let tx = provider
            .get_transaction_by_hash(tx_hash)
            .await
            .wrap_err_with(|| format!("tx not found: {tx_hash:?}"))?
            .ok_or_else(|| eyre::eyre!("tx not found: {tx_hash:?}"))?;
        let target_is_system = is_known_system_sender(tx.from())
            || tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE);
        Ok(TargetFetch { tx, provider, compute_units_per_second, target_is_system })
    }

    async fn prepare<FEN: FoundryEvmNetwork>(
        mut self,
        mut config: Box<Config>,
        evm_opts: EvmOpts,
        target: TargetFetch,
        executor_builder: ExecutorBuilder<FEN>,
    ) -> Result<Option<PreparedRun<FEN>>> {
        #[cfg_attr(not(feature = "monad"), allow(unused_variables))]
        let TargetFetch { tx, provider, compute_units_per_second, .. } = target;
        let tx_hash = tx.tx_hash();
        config.networks = evm_opts.networks;
        self.tracing.labels.append(&mut self.legacy_labels);
        config.tracing = self.resolve_tracing(&config.tracing, shell::verbosity());
        let tracing = config.tracing.clone();

        let with_local_artifacts = self.with_local_artifacts;

        let endpoint_identity = if self.debug_trace_transaction {
            Some(evm_opts.discover_fork_endpoint().await?)
        } else {
            None
        };

        // Fetch the trace from the node via `debug_traceTransaction` (callTracer) instead of
        // re-executing the transaction locally. The node already holds the transaction's exact
        // pre-state and EVM rules, so this needs no block replay and no local executor; it also
        // handles system transactions, so this path comes before the system transaction guard.
        if self.debug_trace_transaction {
            let endpoint_identity = endpoint_identity
                .as_ref()
                .ok_or_else(|| eyre::eyre!("remote trace endpoint identity was not captured"))?;
            let tx_inclusion = tx
                .block_hash_num()
                .ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?;
            let tx_block_number = tx_inclusion.number;
            let tx_block_hash = tx_inclusion.hash;

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
            ensure_remote_transaction_inclusion(
                tx_hash,
                tx_inclusion,
                receipt.block_hash_num(),
                "transaction receipt",
            )?;

            let Some(transaction_block) = provider.get_block_by_hash(tx_block_hash).await? else {
                // `actual: None` always errors; this call exists to reuse its error message.
                ensure_remote_transaction_inclusion(
                    tx_hash,
                    tx_inclusion,
                    None,
                    "block fetched by hash",
                )?;
                unreachable!("ensure_remote_transaction_inclusion errors when actual is None");
            };
            ensure_remote_transaction_inclusion(
                tx_hash,
                tx_inclusion,
                Some(BlockNumHash::new(
                    transaction_block.header().number(),
                    transaction_block.header().hash(),
                )),
                "block fetched by hash",
            )?;

            let success = receipt.status();
            let gas_used = receipt.gas_used();
            let root_create_address = Transaction::to(&tx).is_none().then(|| {
                receipt.contract_address().unwrap_or_else(|| tx.from().create(tx.nonce()))
            });
            let arena = SparsedTraceArena {
                arena: call_frame_to_arena_with_root_address(&frame, root_create_address),
                ignored: Default::default(),
                diagnostics: Default::default(),
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
                    BlockId::hash(tx_block_hash),
                )
                .await?
            } else {
                Default::default()
            };

            // The remote node executed this trace, so its reported family is authoritative for
            // decoding even when the caller selected a compatible local EVM implementation.
            let execution_network = endpoint_identity.network;
            let chain = alloy_chains::Chain::from_id(endpoint_identity.source_chain_id);
            // A configured hardfork is an explicit trace-decoding override. Otherwise honor an
            // Anvil endpoint's exact execution hardfork before consulting the source schedule.
            let resolved_hardfork = if let Some(hardfork) = select_remote_trace_hardfork(
                config.hardfork,
                endpoint_identity.hardfork,
                execution_network,
            ) {
                Some(hardfork)
            } else {
                FoundryHardfork::from_chain_and_timestamp(
                    chain.id(),
                    transaction_block.header().timestamp(),
                )
            };
            let final_endpoint_identity = evm_opts.discover_fork_endpoint().await?;
            ensure_remote_trace_context_unchanged(endpoint_identity, &final_endpoint_identity)?;

            let current_tx = provider.get_transaction_by_hash(tx_hash).await?;
            ensure_remote_transaction_inclusion(
                tx_hash,
                tx_inclusion,
                current_tx.and_then(|tx| tx.block_hash_num()),
                "transaction lookup",
            )?;
            let canonical_block = provider.get_block_by_number(tx_block_number.into()).await?;
            ensure_remote_transaction_inclusion(
                tx_hash,
                tx_inclusion,
                canonical_block
                    .map(|block| BlockNumHash::new(block.header().number(), block.header().hash())),
                "canonical block lookup",
            )?;
            handle_traces(
                result,
                &config,
                chain,
                &contracts_bytecode,
                &tracing,
                with_local_artifacts,
                false,
                resolved_hardfork,
                endpoint_identity.network_profile,
            )
            .await?;

            return Ok(None);
        }

        let tx_block_number = tx
            .block_number()
            .ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?;

        // we need to fork off the parent block
        config.fork_block_number = Some(tx_block_number - 1);

        let create2_deployer = evm_opts.create2_deployer;
        let verbosity = tracing.verbosity;
        let (block, (mut evm_env, tx_env, fork, chain, networks, endpoint_hardfork)) = tokio::try_join!(
            // fetch the block the transaction was mined in
            provider.get_block(tx_block_number.into()).full().into_future().map_err(Into::into),
            TracingExecutor::<FEN>::get_fork_material(&mut config, evm_opts)
        )?;

        let mut evm_version = self.evm_version;
        // Mined transactions already passed the block gas limit check their chain applies, and
        // some chains admit transactions whose gas limit exceeds it: BSC validator transactions
        // carry a gas limit of `i64::MAX`. Re-applying the check can only reject a transaction
        // the chain accepted.
        evm_env.cfg_env.disable_block_gas_limit = true;

        // By default do not enforce transaction gas limits imposed by Osaka (EIP-7825).
        // Users can opt-in to enable these limits by setting `enable_tx_gas_limit` to true.
        if !self.enable_tx_gas_limit {
            evm_env.cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        }

        evm_env.cfg_env.limit_contract_code_size = None;
        evm_env.block_env.set_number(U256::from(tx_block_number));

        let mut parent_beacon_block_root = None;
        if let Some(block) = &block {
            evm_env.block_env = block_env_from_header(block.header());
            parent_beacon_block_root = block.header().parent_beacon_block_root();

            // Unless explicitly configured, resolve the correct spec for the block using the same
            // approach as reth: walk known chain activation conditions to find the latest active
            // fork. Falls back to a blob-gas heuristic for unknown chains.
            if evm_version.is_none()
                && config.hardfork.is_none()
                && FoundryHardfork::from_chain_and_timestamp(chain.id(), block.header().timestamp())
                    .is_none()
                && block.header().excess_blob_gas().is_some()
            {
                // TODO: add glamsterdam header field checks in the future
                evm_version = Some(EvmVersion::Cancun);
            }
            apply_chain_and_block_specific_env_changes_for_chain::<AnyNetwork, _, _>(
                &mut evm_env,
                block,
                chain.id(),
                config.networks,
            );
        }
        let resolved_hardfork = TracingExecutor::<FEN>::resolve_spec_for_chain(
            &config,
            networks,
            chain.id(),
            endpoint_hardfork,
            &mut evm_env,
            evm_version,
        );
        TracingExecutor::<FEN>::extend_precompile_labels(&mut config, networks, resolved_hardfork);

        apply_chain_specific_tx_replay_env_changes_for_chain(&mut evm_env, chain.id());

        let mut executor = TracingExecutor::<FEN>::new(
            executor_builder,
            (evm_env.clone(), tx_env),
            fork,
            evm_version,
            TraceRequirements::none(),
            networks,
            create2_deployer,
            None,
        )?;

        evm_env.cfg_env.set_spec_and_mainnet_gas_params(executor.spec_id());

        let spec_id = (*evm_env.cfg_env.spec()).into();

        if let Some(parent_beacon_block_root) =
            parent_beacon_block_root_for_network(networks, spec_id, parent_beacon_block_root)
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

        Ok(Some(PreparedRun {
            args: self,
            config,
            tracing,
            tx,
            block,
            evm_env,
            executor,
            chain,
            networks,
            resolved_hardfork,
            verbosity,
            prestate_applied,
            #[cfg(feature = "monad")]
            monad: MonadPrepared { tx_block_number, compute_units_per_second },
        }))
    }
}

impl<FEN: FoundryEvmNetwork> PreparedRun<FEN> {
    fn enable_target_tracing(&mut self) {
        let requirements = TraceRequirements::none()
            .with_calls(true)
            .with_debug(self.args.debug)
            .with_decode_internal(if self.tracing.decode_internal {
                InternalTraceMode::Full
            } else {
                InternalTraceMode::None
            })
            .with_state_changes(self.verbosity > 4);
        self.executor.set_trace_requirements(requirements);
        self.executor.set_trace_printer(self.args.trace_printer);
    }

    fn disable_balance_check_for_forged_sender(&mut self) {
        let sender_is_forged = match &*self.tx.inner.inner {
            AnyTxEnvelope::Ethereum(inner) => {
                inner.recover_signer().is_ok_and(|signer| signer != self.tx.from())
            }
            AnyTxEnvelope::Unknown(_) => true,
        };
        if sender_is_forged {
            self.evm_env.cfg_env.disable_balance_check = true;
        }
    }

    fn execute_ordinary(&mut self) -> Result<TraceResult> {
        // Decode the target transaction before replaying the block: an envelope this build
        // can't decode should fail fast.
        let target_tx_env = TxEnvFor::<FEN>::from_any_rpc_transaction(&self.tx)?;

        let target_index = if let Some(block) = &self.block {
            let BlockTransactions::Full(txs) = block.transactions() else {
                eyre::bail!("Could not get block txs");
            };
            txs.iter().position(|candidate| candidate.tx_hash() == self.tx.tx_hash()).ok_or_else(
                || eyre::eyre!("transaction {:?} is missing from its block", self.tx.tx_hash()),
            )?
        } else {
            0
        };

        self.enable_target_tracing();
        self.disable_balance_check_for_forged_sender();
        let replay_prefix = !self.args.quick && !self.prestate_applied;
        let block_number = self.evm_env.block_env.number();
        let tx_hash = self.tx.tx_hash();
        let block = &self.block;
        let replay_system_txes = self.args.replay_system_txes;
        let mut replay = Vec::new();
        if replay_prefix {
            sh_status!("Executing previous transactions from the block.")?;
            if let Some(block) = block {
                let BlockTransactions::Full(txs) = block.transactions() else {
                    eyre::bail!("Could not get block txs");
                };
                let pb = init_progress(txs.len() as u64, "tx");
                for (index, tx) in txs.iter().take(target_index).enumerate() {
                    let is_system = is_known_system_sender(tx.from())
                        || tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE);
                    if !is_system || replay_system_txes {
                        let tx_env =
                            TxEnvFor::<FEN>::from_any_rpc_transaction(tx).wrap_err_with(|| {
                                format!(
                                    "Failed to prepare transaction: {:?} in block {}",
                                    tx.tx_hash(),
                                    block_number
                                )
                            })?;
                        if let Some(to) = Transaction::to(tx) {
                            trace!(tx=?tx.tx_hash(), ?to, "preparing previous call transaction");
                        } else {
                            trace!(tx=?tx.tx_hash(), "preparing previous create transaction");
                        }
                        replay.push((tx.tx_hash(), tx_env));
                    }
                    pb.set_position((index + 1) as u64);
                }
            }
        }
        let result = self.executor.transact_with_ordinary_block_replay(
            self.evm_env.clone(),
            target_tx_env,
            replay,
        )?;
        let trace_kind = if let Some(to) = Transaction::to(&self.tx) {
            trace!(tx=?self.tx.tx_hash(), ?to, "executing call transaction");
            TraceKind::Execution
        } else {
            trace!(tx=?self.tx.tx_hash(), "executing create transaction");
            TraceKind::Deployment
        };
        trace!(?tx_hash, "completed block replay");
        Ok(TraceResult::from_raw(result, trace_kind))
    }

    async fn finish(self, result: TraceResult) -> Result<()> {
        let contracts_bytecode = fetch_contracts_bytecode_from_trace(&self.executor, &result)?;
        handle_traces(
            result,
            &self.config,
            self.chain,
            &contracts_bytecode,
            &self.tracing,
            self.args.with_local_artifacts,
            self.args.debug,
            self.resolved_hardfork,
            self.networks,
        )
        .await
    }
}

#[cfg(feature = "monad")]
impl PreparedRun<MonadEvmNetwork> {
    async fn execute_monad(&mut self) -> Result<TraceResult> {
        // `BlockContext` is typed to `MonadEvmNetwork::Network` (`Ethereum`). Monad blocks only
        // carry standard envelopes, so a typed provider can serve this path while the rest of the
        // command stays on `AnyNetwork`.
        let provider = ProviderBuilder::<alloy_network::Ethereum>::from_config(&self.config)?
            .compute_units_per_second_opt(self.monad.compute_units_per_second)
            .build()?;
        let block =
            provider.get_block(self.monad.tx_block_number.into()).full().await?.ok_or_else(
                || {
                    eyre::eyre!(
                        "block {} is required to reconstruct transaction context",
                        self.monad.tx_block_number
                    )
                },
            )?;
        let block_context = BlockContext::<MonadEvmNetwork>::fetch(&provider, &block).await?;
        // Decode the target transaction before replaying the block: an envelope this build
        // can't decode should fail fast, not after paying for the entire prior-transaction
        // replay.
        let target_tx_env = TxEnvFor::<MonadEvmNetwork>::from_any_rpc_transaction(&self.tx)?;

        let target_index = if let Some(block) = &self.block {
            let BlockTransactions::Full(txs) = block.transactions() else {
                eyre::bail!("Could not get block txs");
            };
            txs.iter().position(|candidate| candidate.tx_hash() == self.tx.tx_hash()).ok_or_else(
                || eyre::eyre!("transaction {:?} is missing from its block", self.tx.tx_hash()),
            )?
        } else {
            0
        };
        self.enable_target_tracing();
        self.disable_balance_check_for_forged_sender();
        let replay_prefix = !self.args.quick && !self.prestate_applied;
        let replay_system_txes = self.args.replay_system_txes;
        let block = &self.block;
        let mut replay = Vec::new();
        if replay_prefix {
            sh_status!("Executing previous transactions from the block.")?;
            if let Some(block) = block {
                let BlockTransactions::Full(txs) = block.transactions() else {
                    eyre::bail!("Could not get block txs");
                };
                let pb = init_progress(txs.len() as u64, "tx");
                for (index, tx) in txs.iter().take(target_index).enumerate() {
                    let tx_env = TxEnvFor::<MonadEvmNetwork>::from_any_rpc_transaction(tx)?;
                    if let Some(to) = Transaction::to(tx) {
                        trace!(tx=?tx.tx_hash(), ?to, "preparing previous call transaction");
                    } else {
                        trace!(tx=?tx.tx_hash(), "preparing previous create transaction");
                    }
                    let chain_context: ChainFor<MonadEvmNetwork> = block_context.transaction(index);
                    replay.push((tx.tx_hash(), tx_env, chain_context));
                    pb.set_position((index + 1) as u64);
                }
            }
        }
        let result = self.executor.transact_with_monad_block_replay(
            self.evm_env.clone(),
            target_tx_env,
            block_context.transaction(target_index),
            replay,
            replay_system_txes,
        )?;
        let Some((result, used_system_replay)) = result else {
            eyre::bail!(
                "{:?} is a system transaction.\nReplaying system transactions is currently not supported.",
                self.tx.tx_hash()
            );
        };
        if used_system_replay {
            trace!(tx=?self.tx.tx_hash(), "executed canonical system transaction");
        }
        let trace_kind = if let Some(to) = Transaction::to(&self.tx) {
            trace!(tx=?self.tx.tx_hash(), ?to, "executing call transaction");
            TraceKind::Execution
        } else {
            trace!(tx=?self.tx.tx_hash(), "executing create transaction");
            TraceKind::Deployment
        };
        Ok(TraceResult::from_raw(result, trace_kind))
    }
}

fn ensure_remote_transaction_inclusion(
    tx_hash: B256,
    expected: BlockNumHash,
    actual: Option<BlockNumHash>,
    source: &str,
) -> Result<()> {
    let Some(actual) = actual else {
        eyre::bail!(
            "transaction {tx_hash} changed inclusion while collecting its remote trace: {source} no longer reports it as mined; retry the command"
        );
    };
    if actual != expected {
        eyre::bail!(
            "transaction {tx_hash} changed inclusion while collecting its remote trace: expected block {} at {}, but {source} reported block {} at {}; retry the command",
            expected.hash,
            expected.number,
            actual.hash,
            actual.number,
        );
    }

    Ok(())
}

const fn parent_beacon_block_root_for_network(
    networks: NetworkConfigs,
    spec_id: SpecId,
    parent_beacon_block_root: Option<B256>,
) -> Option<B256> {
    if networks.is_monad() || !spec_id.is_enabled_in(SpecId::CANCUN) {
        return None;
    }

    // Chains that run a Cancun or later EVM without Ethereum's beacon chain, such as Polygon and
    // Scroll, never populate this header field and never deploy the EIP-4788 contract, so there
    // is no root to apply. Requiring one makes their blocks unreplayable.
    parent_beacon_block_root
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
        let mut requests =
            futures::stream::iter(gather_trace_addresses(traces))
                .map(|address| async move {
                    (address, provider.get_code_at(address).block_id(block).await)
                })
                .buffer_unordered(MAX_CONCURRENT_RPC_REQUESTS);
        while let Some((address, code)) = requests.next().await {
            match code {
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
        let missing_addresses = gather_trace_addresses(traces)
            .filter(|address| !contracts_bytecode.contains_key(address))
            .collect::<Vec<_>>();
        let mut requests =
            futures::stream::iter(missing_addresses)
                .map(|address| async move {
                    (address, provider.get_code_at(address).block_id(block).await)
                })
                .buffer_unordered(MAX_CONCURRENT_RPC_REQUESTS);
        while let Some((address, code)) = requests.next().await {
            match code {
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
    fn remote_transaction_inclusion_must_remain_stable() {
        let tx_hash = B256::repeat_byte(0x11);
        let expected = BlockNumHash::new(42, B256::repeat_byte(0x22));

        ensure_remote_transaction_inclusion(tx_hash, expected, Some(expected), "receipt").unwrap();

        let err =
            ensure_remote_transaction_inclusion(tx_hash, expected, None, "receipt").unwrap_err();
        assert!(err.to_string().contains("no longer reports it as mined"));

        for actual in [
            BlockNumHash::new(43, expected.hash),
            BlockNumHash::new(expected.number, B256::repeat_byte(0x33)),
        ] {
            let err =
                ensure_remote_transaction_inclusion(tx_hash, expected, Some(actual), "receipt")
                    .unwrap_err();
            assert!(err.to_string().contains("changed inclusion"));
        }
    }

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
    fn parent_beacon_block_root_is_applied_only_when_the_header_has_one() {
        let networks = NetworkConfigs::default();
        // Polygon and Scroll run a Cancun or later EVM without populating the header field.
        assert_eq!(parent_beacon_block_root_for_network(networks, SpecId::CANCUN, None), None);

        let root = B256::repeat_byte(0x42);
        assert_eq!(
            parent_beacon_block_root_for_network(networks, SpecId::CANCUN, Some(root)),
            Some(root),
        );
        assert_eq!(
            parent_beacon_block_root_for_network(networks, SpecId::SHANGHAI, Some(root)),
            None,
        );
        assert_eq!(parent_beacon_block_root_for_network(networks, SpecId::SHANGHAI, None), None,);
    }

    #[cfg(feature = "monad")]
    #[test]
    fn parent_beacon_block_root_is_not_used_by_monad() {
        let networks = NetworkConfigs::with_monad();
        for spec_id in [SpecId::PRAGUE, SpecId::OSAKA] {
            assert_eq!(parent_beacon_block_root_for_network(networks, spec_id, None), None,);
            assert_eq!(
                parent_beacon_block_root_for_network(
                    networks,
                    spec_id,
                    Some(B256::repeat_byte(0x42)),
                ),
                None,
            );
        }
    }

    #[test]
    fn debug_trace_transaction_ignores_configured_internal_decoding() {
        let args = RunArgs::parse_from(["cast run", "0x00", "--debug-trace-transaction"]);
        let config = TracingConfig { decode_internal: true, ..Default::default() };

        assert!(!args.resolve_tracing(&config, 0).decode_internal);
    }
}
