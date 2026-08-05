use crate::{
    MAX_CONCURRENT_RPC_REQUESTS,
    debug::handle_traces,
    rpc_trace::{
        call_frame_to_arena_with_root_address, is_method_not_found_error, is_missing_state_error,
    },
    traces::TraceKind,
    utils::{
        apply_chain_and_block_specific_env_changes, apply_chain_specific_tx_replay_env_changes,
        block_env_from_header,
    },
};
use alloy_consensus::{BlockHeader, Transaction, transaction::SignerRecoverable};
use alloy_evm::FromRecoveredTx;

use alloy_network::{
    AnyNetwork, AnyTransactionReceipt, BlockResponse, Network, ReceiptResponse, TransactionResponse,
};
use alloy_primitives::{
    Address, B256, Bytes, U256,
    map::{AddressHashMap, AddressSet, B256HashSet},
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
#[cfg(feature = "optimism")]
use foundry_evm::core::evm::OpEvmNetwork;
use foundry_evm::{
    core::{
        FoundryBlock as _, FoundryTransaction, FromAnyRpcTransaction,
        evm::{EthEvmNetwork, FoundryEvmNetwork, SpecFor, TempoEvmNetwork, TxEnvFor},
    },
    executors::{EvmError, Executor, TracingExecutor},
    hardforks::{ExecutionSpec, FoundryHardfork},
    opts::EvmOpts,
    traces::{InternalTraceMode, SparsedTraceArena, TraceRequirements, Traces},
};
use futures::{StreamExt, TryFutureExt};
use revm::{
    DatabaseRef,
    context::{Block, Transaction as RevmTransaction},
    primitives::{TxKind, hardfork::SpecId},
};

/// A source transaction together with the chain-specific data required for historical replay.
#[derive(Clone, Debug)]
struct PreparedReplayTransaction<TX> {
    transaction: TX,
    l1_gas_used: u64,
}

impl<TX: TransactionResponse> PreparedReplayTransaction<TX> {
    fn new(transaction: TX, nitro_receipt: Option<&AnyTransactionReceipt>) -> Result<Self> {
        let l1_gas_used = if let Some(receipt) = nitro_receipt {
            let tx_hash = *transaction.tx_hash();
            if receipt.transaction_hash() != tx_hash {
                eyre::bail!(
                    "Nitro receipt transaction hash {:?} does not match transaction {tx_hash:?}",
                    receipt.transaction_hash()
                );
            }
            let value = receipt
                .other_fields()
                .get_deserialized::<U256>("gasUsedForL1")
                .ok_or_else(|| {
                    eyre::eyre!("missing `gasUsedForL1` for Nitro transaction {tx_hash:?}")
                })?
                .wrap_err_with(|| {
                    format!("malformed `gasUsedForL1` for Nitro transaction {tx_hash:?}")
                })?;
            u64::try_from(value).wrap_err_with(|| {
                format!("`gasUsedForL1` value {value} exceeds u64::MAX for transaction {tx_hash:?}")
            })?
        } else {
            0
        };
        if let Some(receipt) = nitro_receipt {
            let receipt_gas_used = receipt.gas_used();
            if l1_gas_used > receipt_gas_used {
                eyre::bail!(
                    "Nitro poster gas {l1_gas_used} exceeds receipt gas used {receipt_gas_used} for transaction {:?}",
                    transaction.tx_hash()
                );
            }
            if receipt_gas_used > transaction.gas_limit() {
                eyre::bail!(
                    "receipt gas used {receipt_gas_used} exceeds gas limit {} for transaction {:?}",
                    transaction.gas_limit(),
                    transaction.tx_hash()
                );
            }
        }
        Ok(Self { transaction, l1_gas_used })
    }

    fn tx_env<FEN: FoundryEvmNetwork>(
        &self,
        convert: impl FnOnce(&TX) -> Result<TxEnvFor<FEN>>,
    ) -> Result<TxEnvFor<FEN>> {
        let mut tx_env = convert(&self.transaction)?;
        let gas_limit = self.transaction.gas_limit();
        let execution_gas_limit = gas_limit.checked_sub(self.l1_gas_used).ok_or_else(|| {
            eyre::eyre!(
                "Nitro poster gas {} exceeds gas limit {gas_limit} for transaction {:?}",
                self.l1_gas_used,
                self.transaction.tx_hash()
            )
        })?;
        tx_env.set_gas_limit(execution_gas_limit);
        Ok(tx_env)
    }

    fn total_gas_used(&self, execution_gas_used: u64) -> Result<u64> {
        execution_gas_used.checked_add(self.l1_gas_used).ok_or_else(|| {
            eyre::eyre!(
                "gas used overflow for transaction {:?}: execution gas {execution_gas_used}, Nitro poster gas {}",
                self.transaction.tx_hash(),
                self.l1_gas_used
            )
        })
    }
}

const ARBITRUM_ONE_CHAIN_ID: u64 = 42_161;
const ARBITRUM_ONE_NITRO_ACTIVATION_BLOCK: u64 = 22_207_818;
const ARBITRUM_NOVA_CHAIN_ID: u64 = 42_170;
const ARBITRUM_RINKEBY_CHAIN_ID: u64 = 421_611;
const ARBITRUM_RINKEBY_NITRO_ACTIVATION_BLOCK: u64 = 13_919_178;
const ARBITRUM_GOERLI_CHAIN_ID: u64 = 421_613;
const ARBITRUM_SEPOLIA_CHAIN_ID: u64 = 421_614;

const fn is_nitro_block(chain_id: u64, block_number: u64) -> bool {
    match chain_id {
        ARBITRUM_ONE_CHAIN_ID => block_number >= ARBITRUM_ONE_NITRO_ACTIVATION_BLOCK,
        ARBITRUM_RINKEBY_CHAIN_ID => block_number >= ARBITRUM_RINKEBY_NITRO_ACTIVATION_BLOCK,
        ARBITRUM_NOVA_CHAIN_ID | ARBITRUM_GOERLI_CHAIN_ID | ARBITRUM_SEPOLIA_CHAIN_ID => true,
        _ => false,
    }
}

fn validate_nitro_block<TX: TransactionResponse>(
    txs: &[TX],
    block_hash: B256,
    block_number: u64,
    receipts: &[AnyTransactionReceipt],
    target: &TX,
) -> Result<()> {
    if txs.len() != receipts.len() {
        eyre::bail!(
            "Nitro block {block_hash:?} returned {} transactions but {} receipts",
            txs.len(),
            receipts.len()
        );
    }
    let mut tx_hashes = B256HashSet::default();
    let mut receipt_hashes = B256HashSet::default();
    let mut target_count = 0;
    for (index, (tx, receipt)) in txs.iter().zip(receipts).enumerate() {
        let index = index as u64;
        let tx_hash = tx.tx_hash();
        if !tx_hashes.insert(tx_hash) {
            eyre::bail!("duplicate transaction hash {tx_hash:?} in Nitro block {block_hash:?}");
        }
        let receipt_hash = receipt.transaction_hash();
        if !receipt_hashes.insert(receipt_hash) {
            eyre::bail!("duplicate receipt hash {receipt_hash:?} in Nitro block {block_hash:?}");
        }
        if *tx_hash != receipt_hash {
            eyre::bail!(
                "Nitro transaction/receipt hash mismatch at index {index}: {tx_hash:?} != {receipt_hash:?}"
            );
        }
        if tx.block_hash() != Some(block_hash)
            || tx.block_number() != Some(block_number)
            || tx.transaction_index() != Some(index)
        {
            eyre::bail!(
                "invalid Nitro transaction block metadata for {tx_hash:?} at index {index}"
            );
        }
        if receipt.block_hash() != Some(block_hash)
            || receipt.block_number() != Some(block_number)
            || receipt.transaction_index() != Some(index)
        {
            eyre::bail!(
                "invalid Nitro receipt block metadata for {receipt_hash:?} at index {index}"
            );
        }
        if tx_hash == target.tx_hash() {
            target_count += 1;
        }
    }
    let target_index = target
        .transaction_index()
        .ok_or_else(|| eyre::eyre!("target Nitro transaction is missing its transaction index"))?;
    if target.block_hash() != Some(block_hash) || target.block_number() != Some(block_number) {
        eyre::bail!(
            "target Nitro transaction block metadata does not match fetched block {block_hash:?}"
        );
    }
    if target_count != 1
        || txs.get(target_index as usize).is_none_or(|tx| tx.tx_hash() != target.tx_hash())
    {
        eyre::bail!(
            "target Nitro transaction {:?} was not found exactly once at declared index {target_index}",
            target.tx_hash()
        );
    }
    Ok(())
}

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
            return self.run_with_evm::<TempoEvmNetwork>().await;
        }

        #[cfg(feature = "optimism")]
        if evm_opts.networks.is_optimism() {
            return self.run_with_evm::<OpEvmNetwork>().await;
        }

        self.run_with_evm::<EthEvmNetwork>().await
    }

    async fn run_with_evm<FEN: FoundryEvmNetwork>(mut self) -> Result<()> {
        let figment = self.rpc.clone().into_figment(self.with_local_artifacts).merge(&self);
        let evm_opts = figment.extract::<EvmOpts>()?;
        let mut config = load_config_from_provider(figment)?;
        self.tracing.labels.append(&mut self.legacy_labels);
        config.tracing = self.resolve_tracing(&config.tracing, shell::verbosity());
        let tracing = config.tracing.clone();

        let with_local_artifacts = self.with_local_artifacts;
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
        let tx_block_hash =
            tx.block_hash().ok_or_else(|| eyre::eyre!("tx may still be pending: {:?}", tx_hash))?;
        let source_chain_id = provider.get_chain_id().await?;
        let is_nitro = is_nitro_block(source_chain_id, tx_block_number);

        if is_nitro {
            let provider = ProviderBuilder::<AnyNetwork>::from_config(&config)?
                .compute_units_per_second_opt(compute_units_per_second)
                .build()?;
            let tx = provider
                .get_transaction_by_hash(tx_hash)
                .await
                .wrap_err_with(|| format!("tx not found: {tx_hash:?}"))?
                .ok_or_else(|| eyre::eyre!("tx not found: {tx_hash:?}"))?;
            return self
                .replay_local::<FEN, AnyNetwork, _, _, _>(
                    config,
                    evm_opts,
                    tracing,
                    provider,
                    tx,
                    tx_hash,
                    tx_block_number,
                    tx_block_hash,
                    true,
                    TxEnvFor::<FEN>::from_any_rpc_transaction,
                    |tx| {
                        let from = tx.from();
                        tx.as_envelope().is_some_and(|tx| {
                            tx.recover_signer().is_ok_and(|signer| signer != from)
                        })
                    },
                )
                .await;
        }

        self.replay_local::<FEN, FEN::Network, _, _, _>(
            config,
            evm_opts,
            tracing,
            provider,
            tx,
            tx_hash,
            tx_block_number,
            tx_block_hash,
            false,
            |tx| Ok(TxEnvFor::<FEN>::from_recovered_tx(tx.as_ref(), tx.from())),
            |tx| tx.as_ref().recover_signer().is_ok_and(|signer| signer != tx.from()),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn replay_local<FEN, SN, P, C, B>(
        self,
        mut config: Config,
        evm_opts: EvmOpts,
        tracing: TracingConfig,
        provider: P,
        tx: SN::TransactionResponse,
        tx_hash: B256,
        tx_block_number: u64,
        tx_block_hash: B256,
        is_nitro: bool,
        convert: C,
        has_signer_mismatch: B,
    ) -> Result<()>
    where
        FEN: FoundryEvmNetwork,
        SN: Network,
        P: Provider<SN>,
        C: Copy + Fn(&SN::TransactionResponse) -> Result<TxEnvFor<FEN>>,
        B: Copy + Fn(&SN::TransactionResponse) -> bool,
    {
        let with_local_artifacts = self.with_local_artifacts;
        let debug = self.debug;

        // we need to fork off the parent block
        config.fork_block_number = Some(tx_block_number - 1);

        let create2_deployer = evm_opts.create2_deployer;
        let verbosity = tracing.verbosity;
        let (block, receipts, (mut evm_env, tx_env, fork, chain, networks)) = tokio::try_join!(
            // fetch the block the transaction was mined in
            provider.get_block_by_hash(tx_block_hash).full().into_future().map_err(Into::into),
            async {
                if is_nitro {
                    let receipts = provider
                        .raw_request::<_, Option<Vec<AnyTransactionReceipt>>>(
                            "eth_getBlockReceipts".into(),
                            (BlockId::Hash(tx_block_hash.into()),),
                        )
                        .await?
                        .ok_or_else(|| {
                            eyre::eyre!("receipts not found for Nitro block {tx_block_number}")
                        })?;
                    Ok(Some(receipts))
                } else {
                    Ok(None)
                }
            },
            TracingExecutor::<FEN>::get_fork_material(&mut config, evm_opts)
        )?;
        if is_nitro {
            let block = block
                .as_ref()
                .ok_or_else(|| eyre::eyre!("Nitro block {tx_block_hash:?} not found"))?;
            if block.header().number() != tx_block_number {
                eyre::bail!(
                    "fetched Nitro block metadata does not match requested block {tx_block_hash:?} at number {tx_block_number}"
                );
            }
            let BlockTransactions::Full(txs) = block.transactions() else {
                eyre::bail!("Nitro block {tx_block_hash:?} did not contain full transactions");
            };
            validate_nitro_block(
                txs,
                tx_block_hash,
                tx_block_number,
                receipts.as_deref().unwrap_or_default(),
                &tx,
            )?;
        }

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
            apply_chain_and_block_specific_env_changes::<SN, _, _>(
                &mut evm_env,
                block,
                config.networks,
            );
        }
        apply_chain_specific_tx_replay_env_changes(&mut evm_env);

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
            parent_beacon_block_root_for_spec(spec_id, parent_beacon_block_root, is_nitro)?
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

            if let Some(block) = block {
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

                    let receipt = receipts.as_ref().and_then(|receipts| receipts.get(index));
                    let prepared = PreparedReplayTransaction::new(tx.clone(), receipt)?;
                    let tx_env = prepared.tx_env::<FEN>(convert)?;

                    evm_env.cfg_env.disable_balance_check = true;

                    match tx_env.kind() {
                        TxKind::Call(to) => {
                            trace!(tx=?tx.tx_hash(),?to, "executing previous call transaction");
                            executor
                                .transact_with_env(evm_env.clone(), tx_env.clone())
                                .wrap_err_with(|| {
                                    format!(
                                        "Failed to execute transaction: {:?} in block {}",
                                        tx.tx_hash(),
                                        evm_env.block_env.number()
                                    )
                                })?;
                        }
                        TxKind::Create => {
                            trace!(tx=?tx.tx_hash(), "executing previous create transaction");
                            if let Err(error) =
                                executor.deploy_with_env(evm_env.clone(), tx_env.clone(), None)
                            {
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
                    }

                    pb.set_position((index + 1) as u64);
                }
            }
        }

        // Execute our transaction
        let result = {
            executor.set_trace_printer(self.trace_printer);

            let receipt = receipts.as_ref().and_then(|receipts| {
                tx.transaction_index().and_then(|index| receipts.get(index as usize))
            });
            let prepared = PreparedReplayTransaction::new(tx.clone(), receipt)?;
            let tx_env = prepared.tx_env::<FEN>(convert)?;

            if has_signer_mismatch(&tx) {
                evm_env.cfg_env.disable_balance_check = true;
            }

            let mut result = match tx_env.kind() {
                TxKind::Call(to) => {
                    trace!(tx=?tx.tx_hash(), to=?to, "executing call transaction");
                    TraceResult::from(executor.transact_with_env(evm_env, tx_env)?)
                }
                TxKind::Create => {
                    trace!(tx=?tx.tx_hash(), "executing create transaction");
                    TraceResult::try_from(executor.deploy_with_env(evm_env, tx_env, None))?
                }
            };
            result.gas_used = prepared.total_gas_used(result.gas_used)?;
            result
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
    allow_missing: bool,
) -> Result<Option<B256>> {
    if !spec_id.is_enabled_in(SpecId::CANCUN) {
        return Ok(None);
    }
    if allow_missing && parent_beacon_block_root.is_none() {
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
    use alloy_consensus::{Signed, TxEip1559, transaction::Recovered};
    use alloy_network::AnyRpcTransaction;
    use alloy_primitives::{Signature, address};
    use alloy_rpc_types::{Transaction as RpcTransaction, TransactionInfo};

    fn replay_transaction(gas_limit: u64) -> AnyRpcTransaction {
        replay_transaction_with_metadata(gas_limit, B256::ZERO, B256::ZERO, 1, 0)
    }

    fn replay_transaction_with_metadata(
        gas_limit: u64,
        tx_hash: B256,
        block_hash: B256,
        block_number: u64,
        transaction_index: u64,
    ) -> AnyRpcTransaction {
        let signed = Signed::new_unchecked(
            TxEip1559 { chain_id: 42161, gas_limit, ..Default::default() },
            Signature::new(U256::ZERO, U256::ZERO, false),
            tx_hash,
        );
        RpcTransaction::from_transaction(
            Recovered::new_unchecked(signed.into(), Address::ZERO),
            TransactionInfo {
                hash: Some(tx_hash),
                index: Some(transaction_index),
                block_hash: Some(block_hash),
                block_number: Some(block_number),
                ..Default::default()
            },
        )
        .into()
    }

    fn nitro_receipt(gas_used_for_l1: &str) -> AnyTransactionReceipt {
        nitro_receipt_with_metadata(gas_used_for_l1, 2_728_986, B256::ZERO, B256::ZERO, 1, 0)
    }

    fn nitro_receipt_with_metadata(
        gas_used_for_l1: &str,
        gas_used: u64,
        tx_hash: B256,
        block_hash: B256,
        block_number: u64,
        transaction_index: u64,
    ) -> AnyTransactionReceipt {
        serde_json::from_value(serde_json::json!({
            "status": "0x0",
            "cumulativeGasUsed": format!("0x{gas_used:x}"),
            "logs": [],
            "logsBloom": format!("0x{}", "00".repeat(256)),
            "type": "0x2",
            "transactionHash": tx_hash,
            "transactionIndex": format!("0x{transaction_index:x}"),
            "blockHash": block_hash,
            "blockNumber": format!("0x{block_number:x}"),
            "gasUsed": format!("0x{gas_used:x}"),
            "effectiveGasPrice": "0x1",
            "from": Address::ZERO,
            "to": Address::ZERO,
            "contractAddress": null,
            "gasUsedForL1": gas_used_for_l1,
        }))
        .unwrap()
    }

    #[test]
    fn replays_issue_7514_with_nitro_gas_accounting() {
        let transaction = replay_transaction(2_733_748);
        let receipt = nitro_receipt("0x26ed52");
        let prepared = PreparedReplayTransaction::new(transaction, Some(&receipt)).unwrap();

        let tx_env = prepared
            .tx_env::<EthEvmNetwork>(TxEnvFor::<EthEvmNetwork>::from_any_rpc_transaction)
            .unwrap();
        assert_eq!(tx_env.gas_limit, 182_626);
        assert_eq!(prepared.total_gas_used(177_864).unwrap(), 2_728_986);
    }

    #[test]
    fn rejects_invalid_nitro_replay_metadata() {
        let receipt = nitro_receipt_with_metadata("0x33", 50, B256::ZERO, B256::ZERO, 1, 0);
        let err =
            PreparedReplayTransaction::new(replay_transaction(100), Some(&receipt)).unwrap_err();
        assert!(err.to_string().contains("poster gas 51 exceeds receipt gas used 50"));

        let receipt = nitro_receipt_with_metadata("0x1", 50, B256::ZERO, B256::ZERO, 1, 0);
        let err =
            PreparedReplayTransaction::new(replay_transaction(49), Some(&receipt)).unwrap_err();
        assert!(err.to_string().contains("receipt gas used 50 exceeds gas limit 49"));

        let receipt = nitro_receipt_with_metadata("0x0", 0, B256::repeat_byte(1), B256::ZERO, 1, 0);
        let err =
            PreparedReplayTransaction::new(replay_transaction(1), Some(&receipt)).unwrap_err();
        assert!(err.to_string().contains("does not match transaction"));

        let receipt = nitro_receipt("0x1");
        let prepared =
            PreparedReplayTransaction::new(replay_transaction(u64::MAX), Some(&receipt)).unwrap();
        let err = prepared.total_gas_used(u64::MAX).unwrap_err();
        assert!(err.to_string().contains("gas used overflow"));
    }

    #[test]
    fn rejects_inconsistent_nitro_block_receipts() {
        let block_hash = B256::repeat_byte(0xaa);
        let first_hash = B256::repeat_byte(1);
        let target_hash = B256::repeat_byte(2);
        let block_number = 42;
        let first = replay_transaction_with_metadata(100, first_hash, block_hash, block_number, 0);
        let target =
            replay_transaction_with_metadata(100, target_hash, block_hash, block_number, 1);
        let first_receipt =
            nitro_receipt_with_metadata("0x1", 50, first_hash, block_hash, block_number, 0);
        let target_receipt =
            nitro_receipt_with_metadata("0x1", 50, target_hash, block_hash, block_number, 1);
        let txs = vec![first.clone(), target.clone()];
        let receipts = vec![first_receipt.clone(), target_receipt.clone()];

        validate_nitro_block(&txs, block_hash, block_number, &receipts, &target).unwrap();

        let err = validate_nitro_block(&txs, block_hash, block_number, &receipts[..1], &target)
            .unwrap_err();
        assert!(err.to_string().contains("2 transactions but 1 receipts"));

        let err = validate_nitro_block(
            &txs,
            block_hash,
            block_number,
            &[target_receipt.clone(), first_receipt.clone()],
            &target,
        )
        .unwrap_err();
        assert!(err.to_string().contains("hash mismatch at index 0"));

        let duplicate_receipt =
            nitro_receipt_with_metadata("0x1", 50, first_hash, block_hash, block_number, 1);
        let err = validate_nitro_block(
            &txs,
            block_hash,
            block_number,
            &[first_receipt.clone(), duplicate_receipt],
            &target,
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate receipt hash"));

        let wrong_block_receipt = nitro_receipt_with_metadata(
            "0x1",
            50,
            target_hash,
            B256::repeat_byte(0xbb),
            block_number,
            1,
        );
        let err = validate_nitro_block(
            &txs,
            block_hash,
            block_number,
            &[first_receipt, wrong_block_receipt],
            &target,
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid Nitro receipt block metadata"));

        let wrong_target_index =
            replay_transaction_with_metadata(100, target_hash, block_hash, block_number, 0);
        let err =
            validate_nitro_block(&txs, block_hash, block_number, &receipts, &wrong_target_index)
                .unwrap_err();
        assert!(err.to_string().contains("not found exactly once at declared index 0"));
    }

    #[test]
    fn prepares_preceding_and_target_nitro_transactions() {
        let block_hash = B256::repeat_byte(0xaa);
        let block_number = 42;
        let preceding_hash = B256::repeat_byte(1);
        let target_hash = B256::repeat_byte(2);
        let preceding =
            replay_transaction_with_metadata(1_000, preceding_hash, block_hash, block_number, 0);
        let target =
            replay_transaction_with_metadata(500, target_hash, block_hash, block_number, 1);
        let preceding_receipt =
            nitro_receipt_with_metadata("0x64", 900, preceding_hash, block_hash, block_number, 0);
        let target_receipt =
            nitro_receipt_with_metadata("0x12c", 480, target_hash, block_hash, block_number, 1);
        let txs = [preceding.clone(), target.clone()];
        let receipts = [preceding_receipt, target_receipt];
        validate_nitro_block(&txs, block_hash, block_number, &receipts, &target).unwrap();

        let preceding = PreparedReplayTransaction::new(preceding, Some(&receipts[0])).unwrap();
        let preceding_env = preceding
            .tx_env::<EthEvmNetwork>(TxEnvFor::<EthEvmNetwork>::from_any_rpc_transaction)
            .unwrap();
        assert_eq!(preceding_env.gas_limit, 900);

        let target = PreparedReplayTransaction::new(target, Some(&receipts[1])).unwrap();
        let target_env = target
            .tx_env::<EthEvmNetwork>(TxEnvFor::<EthEvmNetwork>::from_any_rpc_transaction)
            .unwrap();
        assert_eq!(target_env.gas_limit, 200);
        assert_eq!(target.total_gas_used(180).unwrap(), 480);
    }

    #[test]
    fn gates_nitro_by_chain_and_block() {
        assert!(!is_nitro_block(ARBITRUM_ONE_CHAIN_ID, ARBITRUM_ONE_NITRO_ACTIVATION_BLOCK - 1));
        assert!(is_nitro_block(ARBITRUM_ONE_CHAIN_ID, ARBITRUM_ONE_NITRO_ACTIVATION_BLOCK));
        assert!(!is_nitro_block(
            ARBITRUM_RINKEBY_CHAIN_ID,
            ARBITRUM_RINKEBY_NITRO_ACTIVATION_BLOCK - 1
        ));
        assert!(is_nitro_block(ARBITRUM_RINKEBY_CHAIN_ID, ARBITRUM_RINKEBY_NITRO_ACTIVATION_BLOCK));
        assert!(is_nitro_block(ARBITRUM_NOVA_CHAIN_ID, 0));
        assert!(is_nitro_block(ARBITRUM_GOERLI_CHAIN_ID, 0));
        assert!(is_nitro_block(ARBITRUM_SEPOLIA_CHAIN_ID, 0));
        assert!(!is_nitro_block(1, ARBITRUM_ONE_NITRO_ACTIVATION_BLOCK));
        assert!(!is_nitro_block(412_346, u64::MAX));
    }

    #[test]
    fn rejects_invalid_nitro_poster_gas() {
        let transaction = replay_transaction(u64::MAX);
        for (value, expected) in [
            (None, "missing `gasUsedForL1`"),
            (Some("bogus"), "malformed `gasUsedForL1`"),
            (Some("0x10000000000000000"), "exceeds u64::MAX"),
        ] {
            let mut receipt = nitro_receipt("0x0");
            match value {
                Some(value) => {
                    receipt
                        .other_fields_mut()
                        .insert("gasUsedForL1".into(), serde_json::json!(value));
                }
                None => {
                    receipt.other_fields_mut().remove("gasUsedForL1");
                }
            }
            let err =
                PreparedReplayTransaction::new(transaction.clone(), Some(&receipt)).unwrap_err();
            assert!(err.to_string().contains(expected), "{err}");
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
    fn parent_beacon_block_root_is_required_for_cancun() {
        let err = parent_beacon_block_root_for_spec(SpecId::CANCUN, None, false).unwrap_err();
        assert!(err.to_string().contains("MissingParentBeaconBlockRoot"));

        let root = B256::repeat_byte(0x42);
        assert_eq!(
            parent_beacon_block_root_for_spec(SpecId::CANCUN, Some(root), false).unwrap(),
            Some(root),
        );
        assert_eq!(
            parent_beacon_block_root_for_spec(SpecId::SHANGHAI, Some(root), false).unwrap(),
            None
        );
        assert_eq!(parent_beacon_block_root_for_spec(SpecId::SHANGHAI, None, false).unwrap(), None);
        assert_eq!(parent_beacon_block_root_for_spec(SpecId::CANCUN, None, true).unwrap(), None);
    }

    #[test]
    fn debug_trace_transaction_ignores_configured_internal_decoding() {
        let args = RunArgs::parse_from(["cast run", "0x00", "--debug-trace-transaction"]);
        let config = TracingConfig { decode_internal: true, ..Default::default() };

        assert!(!args.resolve_tracing(&config, 0).decode_internal);
    }
}
