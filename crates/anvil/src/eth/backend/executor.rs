use crate::{
    eth::{
        backend::cheats::CheatsManager, error::InvalidTransactionError,
        pool::transactions::PoolTransaction,
    },
    mem::inspector::{AnvilInspector, InspectorTxConfig},
};
use alloy_consensus::{
    Eip658Value, Receipt, ReceiptWithBloom, Transaction, TransactionEnvelope, TxReceipt,
    transaction::{Either, Recovered},
};
use alloy_eips::{
    Encodable2718, eip2935, eip4788,
    eip6110::DEPOSIT_REQUEST_TYPE,
    eip7685::Requests,
    eip7702::{RecoveredAuthority, RecoveredAuthorization},
};
use alloy_evm::{
    Evm, FromRecoveredTx, FromTxWithEncoded, RecoveredTx,
    block::{
        BlockExecutionError, BlockExecutionResult, BlockExecutor, BlockValidationError,
        ExecutableTx, GasOutput, StateDB, SystemCaller, TxResult,
    },
    eth::{
        EthTxResult,
        eip6110::parse_deposits_from_receipts,
        receipt_builder::{ReceiptBuilder, ReceiptBuilderCtx},
        spec::EthExecutorSpec,
    },
};
use alloy_hardforks::{EthereumHardfork, EthereumHardforks, ForkCondition};
use alloy_primitives::{Address, B256, Bytes, Log, U256};
use anvil_core::eth::transaction::{
    MaybeImpersonatedTransaction, PendingTransaction, TransactionInfo,
};
#[cfg(feature = "base")]
use base_common_consensus::Eip8130Receipt;
#[cfg(feature = "base")]
use base_common_evm::Eip8130PhaseStatuses;
use foundry_evm::core::{env::FoundryTransaction, evm::IntoInstructionResult};
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxEnvelope, FoundryTxType};
use revm::{
    Database, DatabaseCommit,
    context::Block as RevmBlock,
    context_interface::result::{ExecutionResult, Output, ResultAndState},
    interpreter::InstructionResult,
    primitives::hardfork::SpecId,
    state::{AccountInfo, EvmState},
};
use std::{fmt, fmt::Debug, mem::take, sync::Arc};

#[cfg(any(feature = "base", feature = "optimism"))]
pub(crate) mod optimism;

/// Determines whether an executor produces a complete block or a historical transaction prefix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BlockExecutionKind {
    /// Apply all pre- and post-block transitions.
    #[default]
    Complete,
    /// Apply block-start transitions, but do not drain post-block request queues.
    TransactionPrefix,
}

/// Ethereum-only consensus transition configuration for an Anvil block executor.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EthereumBlockTransitions {
    pub(crate) hardfork: EthereumHardfork,
    pub(crate) deposit_contract_address: Address,
    pub(crate) parent_beacon_block_root: Option<B256>,
    pub(crate) execution_kind: BlockExecutionKind,
}

/// A hardfork specification whose configured Ethereum fork is active from genesis.
#[derive(Clone, Copy, Debug)]
struct ActiveEthereumSpec {
    hardfork: EthereumHardfork,
    deposit_contract_address: Address,
}

impl EthereumHardforks for ActiveEthereumSpec {
    fn ethereum_fork_activation(&self, fork: EthereumHardfork) -> ForkCondition {
        if fork <= self.hardfork { ForkCondition::ZERO_TIMESTAMP } else { ForkCondition::Never }
    }
}

impl EthExecutorSpec for ActiveEthereumSpec {
    fn deposit_contract_address(&self) -> Option<Address> {
        Some(self.deposit_contract_address)
    }
}

/// Applies canonical Ethereum block-start transitions in consensus order.
pub(crate) fn apply_ethereum_pre_execution_changes<E>(
    evm: &mut E,
    parent_hash: B256,
    transitions: EthereumBlockTransitions,
) -> Result<(), BlockExecutionError>
where
    E: Evm<DB: DatabaseCommit>,
{
    let mut caller = SystemCaller::new(ActiveEthereumSpec {
        hardfork: transitions.hardfork,
        deposit_contract_address: transitions.deposit_contract_address,
    });
    caller.apply_blockhashes_contract_call(parent_hash, evm)?;
    caller.apply_beacon_root_contract_call(transitions.parent_beacon_block_root, evm)
}

/// Collects deposits before draining the withdrawal and consolidation request queues.
pub(crate) fn apply_ethereum_post_execution_changes<E>(
    evm: &mut E,
    transitions: EthereumBlockTransitions,
    receipts: &[FoundryReceiptEnvelope],
) -> Result<Requests, BlockExecutionError>
where
    E: Evm<DB: DatabaseCommit>,
{
    if transitions.hardfork < EthereumHardfork::Prague {
        return Ok(Requests::default());
    }

    let spec = ActiveEthereumSpec {
        hardfork: transitions.hardfork,
        deposit_contract_address: transitions.deposit_contract_address,
    };
    let mut requests = Requests::default();
    append_deposit_requests(spec, receipts, &mut requests)?;
    SystemCaller::new(spec).append_post_execution_changes(evm, &mut requests)?;
    Ok(requests)
}

fn append_deposit_requests(
    spec: ActiveEthereumSpec,
    receipts: &[FoundryReceiptEnvelope],
    requests: &mut Requests,
) -> Result<(), BlockExecutionError> {
    let deposits = parse_deposits_from_receipts(spec, receipts)?;
    if !deposits.is_empty() {
        requests.push_request_with_type(DEPOSIT_REQUEST_TYPE, deposits);
    }
    Ok(())
}

/// Receipt builder for transaction types that do not require network-specific metadata.
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct FoundryReceiptBuilder;

impl FoundryReceiptBuilder {
    #[cfg_attr(not(feature = "base"), allow(clippy::missing_const_for_fn))]
    fn wrap_receipt(
        tx_type: FoundryTxType,
        receipt: ReceiptWithBloom<Receipt>,
    ) -> FoundryReceiptEnvelope {
        match tx_type {
            FoundryTxType::Legacy => FoundryReceiptEnvelope::Legacy(receipt),
            FoundryTxType::Eip2930 => FoundryReceiptEnvelope::Eip2930(receipt),
            FoundryTxType::Eip1559 => FoundryReceiptEnvelope::Eip1559(receipt),
            FoundryTxType::Eip4844 => FoundryReceiptEnvelope::Eip4844(receipt),
            FoundryTxType::Eip7702 => FoundryReceiptEnvelope::Eip7702(receipt),
            #[cfg(any(feature = "base", feature = "optimism"))]
            FoundryTxType::Deposit => {
                panic!("deposit receipts require fork-specific metadata")
            }
            #[cfg(feature = "optimism")]
            FoundryTxType::PostExec => FoundryReceiptEnvelope::PostExec(receipt),
            #[cfg(feature = "base")]
            FoundryTxType::Eip8130 => FoundryReceiptEnvelope::Eip8130(ReceiptWithBloom {
                receipt: Eip8130Receipt::new(receipt.receipt, Eip8130PhaseStatuses::take()),
                logs_bloom: receipt.logs_bloom,
            }),
            FoundryTxType::Tempo => FoundryReceiptEnvelope::Tempo(receipt),
        }
    }

    /// Builds a typed receipt for an RPC-simulated transaction.
    pub(crate) fn build_simulated_receipt(
        tx_type: FoundryTxType,
        result: &ExecutionResult,
        logs: Vec<Log>,
        cumulative_gas_used: u64,
    ) -> FoundryReceiptEnvelope {
        let receipt =
            Receipt { status: Eip658Value::Eip658(result.is_success()), cumulative_gas_used, logs }
                .with_bloom();
        Self::wrap_receipt(tx_type, receipt)
    }
}

impl ReceiptBuilder for FoundryReceiptBuilder {
    type Transaction = FoundryTxEnvelope;
    type Receipt = FoundryReceiptEnvelope;

    fn build_receipt<E: Evm>(
        &self,
        ctx: ReceiptBuilderCtx<'_, FoundryTxType, E>,
    ) -> FoundryReceiptEnvelope {
        let receipt = Receipt {
            status: Eip658Value::Eip658(ctx.result.is_success()),
            cumulative_gas_used: ctx.cumulative_gas_used,
            logs: ctx.result.into_logs(),
        }
        .with_bloom();
        Self::wrap_receipt(ctx.tx_type, receipt)
    }
}

/// Result of executing a transaction in [`AnvilBlockExecutor`].
///
/// Wraps [`EthTxResult`] with the sender address, needed for deposit nonce resolution.
#[derive(Debug)]
pub struct AnvilTxResult<H> {
    pub inner: EthTxResult<H, FoundryTxType>,
    pub sender: Address,
}

impl<H: Send + 'static> TxResult for AnvilTxResult<H> {
    type HaltReason = H;

    fn result(&self) -> &ResultAndState<Self::HaltReason> {
        self.inner.result()
    }

    fn into_result(self) -> ResultAndState<Self::HaltReason> {
        self.inner.into_result()
    }
}

/// Block executor for Anvil that implements [`BlockExecutor`].
///
/// Wraps an EVM instance and produces [`FoundryReceiptEnvelope`] receipts.
/// Validation (gas limits, blob gas, transaction validity) is handled by the
/// caller before transactions are fed to this executor.
pub struct AnvilBlockExecutor<E> {
    /// The EVM instance used for execution.
    evm: E,
    /// Parent block hash — needed for EIP-2935 system call.
    parent_hash: B256,
    /// The active spec id, used to gate hardfork-specific behavior.
    spec_id: SpecId,
    /// Canonical Ethereum consensus transitions, disabled for other networks.
    ethereum_transitions: Option<EthereumBlockTransitions>,
    /// Receipt builder.
    receipt_builder: FoundryReceiptBuilder,
    /// Receipts of executed transactions.
    receipts: Vec<FoundryReceiptEnvelope>,
    /// Total gas used by transactions in this block.
    gas_used: u64,
    /// Blob gas used by the block.
    blob_gas_used: u64,
    /// Maximum blob gas available to transactions in this block.
    max_blob_gas_per_block: u64,
    /// Whether OP Jovian repurposes `blobGasUsed` for the DA footprint.
    optimism_jovian: bool,
    /// State changes captured for deferred publication.
    state_changes: Option<Vec<EvmState>>,
}

impl<E: fmt::Debug> fmt::Debug for AnvilBlockExecutor<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("AnvilBlockExecutor");
        debug
            .field("evm", &self.evm)
            .field("parent_hash", &self.parent_hash)
            .field("spec_id", &self.spec_id)
            .field("ethereum_transitions", &self.ethereum_transitions)
            .field("gas_used", &self.gas_used)
            .field("blob_gas_used", &self.blob_gas_used)
            .field("max_blob_gas_per_block", &self.max_blob_gas_per_block)
            .field("optimism_jovian", &self.optimism_jovian);
        debug.field("receipts", &self.receipts.len()).finish_non_exhaustive()
    }
}

impl<E> AnvilBlockExecutor<E> {
    /// Creates a new [`AnvilBlockExecutor`].
    pub(crate) const fn new(
        evm: E,
        parent_hash: B256,
        spec_id: SpecId,
        ethereum_transitions: Option<EthereumBlockTransitions>,
    ) -> Self {
        Self {
            evm,
            parent_hash,
            spec_id,
            ethereum_transitions,
            receipt_builder: FoundryReceiptBuilder,
            receipts: Vec::new(),
            gas_used: 0,
            blob_gas_used: 0,
            max_blob_gas_per_block: u64::MAX,
            optimism_jovian: false,
            state_changes: None,
        }
    }

    /// Captures every committed changeset so callers can publish them after block execution.
    pub(crate) fn with_state_changes(mut self) -> Self {
        self.state_changes = Some(Vec::new());
        self
    }

    /// Applies the active block blob gas limit.
    pub(crate) const fn with_max_blob_gas_per_block(mut self, limit: u64) -> Self {
        self.max_blob_gas_per_block = limit;
        self
    }

    /// Takes the captured changesets.
    pub(crate) fn take_state_changes(&mut self) -> Vec<EvmState> {
        self.state_changes.take().unwrap_or_default()
    }
}

impl<E> AnvilBlockExecutor<E>
where
    E: Evm<
            DB: StateDB,
            Tx: FromRecoveredTx<FoundryTxEnvelope> + FromTxWithEncoded<FoundryTxEnvelope>,
        >,
{
    /// Executes a transaction without committing it, using the supplied network-specific
    /// transaction entry point.
    pub(crate) fn execute_transaction_without_commit_with<T, F>(
        &mut self,
        tx: T,
        transact: F,
    ) -> Result<AnvilTxResult<E::HaltReason>, BlockExecutionError>
    where
        T: ExecutableTx<Self>,
        F: FnOnce(
            &mut E,
            E::Tx,
            B256,
        ) -> Result<ResultAndState<E::HaltReason>, BlockExecutionError>,
    {
        let (tx_env, tx) = tx.into_parts();

        let block_available_gas = self.evm.block().gas_limit() - self.gas_used;
        if tx.tx().gas_limit() > block_available_gas {
            return Err(BlockValidationError::TransactionGasLimitMoreThanAvailableBlockGas {
                transaction_gas_limit: tx.tx().gas_limit(),
                block_available_gas,
            }
            .into());
        }

        let sender = *tx.signer();
        let transaction_hash = tx.tx().trie_hash();
        #[cfg(feature = "optimism")]
        let blob_gas_used =
            optimism::blob_gas_used(self.evm.db_mut(), tx.tx(), self.optimism_jovian)?;
        #[cfg(not(feature = "optimism"))]
        let blob_gas_used = tx.tx().blob_gas_used().unwrap_or_default();
        let blob_gas_limit = block_blob_gas_limit(
            self.optimism_jovian,
            self.evm.block().gas_limit(),
            self.max_blob_gas_per_block,
        );
        if self.blob_gas_used.saturating_add(blob_gas_used) > blob_gas_limit {
            return Err(BlockExecutionError::msg("block blob gas limit exceeded"));
        }
        let result = transact(&mut self.evm, tx_env, transaction_hash)?;

        Ok(AnvilTxResult {
            inner: EthTxResult { result, blob_gas_used, tx_type: tx.tx().tx_type() },
            sender,
        })
    }
}

impl<E> BlockExecutor for AnvilBlockExecutor<E>
where
    E: Evm<
            DB: StateDB,
            Tx: FromRecoveredTx<FoundryTxEnvelope> + FromTxWithEncoded<FoundryTxEnvelope>,
        >,
{
    type Transaction = FoundryTxEnvelope;
    type Receipt = FoundryReceiptEnvelope;
    type Evm = E;
    type Result = AnvilTxResult<E::HaltReason>;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        if let Some(transitions) = self.ethereum_transitions {
            // Historical fork-prefix publication needs the individual changesets after executing
            // against its disposable overlay. Preserve canonical ordering while capturing them.
            if let Some(state_changes) = &mut self.state_changes {
                if transitions.hardfork >= EthereumHardfork::Prague {
                    let result = self
                        .evm
                        .transact_system_call(
                            eip4788::SYSTEM_ADDRESS,
                            eip2935::HISTORY_STORAGE_ADDRESS,
                            Bytes::copy_from_slice(self.parent_hash.as_slice()),
                        )
                        .map_err(BlockExecutionError::other)?;
                    state_changes.push(result.state.clone());
                    self.evm.db_mut().commit(result.state);
                }
                if transitions.hardfork >= EthereumHardfork::Cancun {
                    let parent_beacon_block_root = transitions
                        .parent_beacon_block_root
                        .ok_or(BlockValidationError::MissingParentBeaconBlockRoot)?;
                    let result = self
                        .evm
                        .transact_system_call(
                            eip4788::SYSTEM_ADDRESS,
                            eip4788::BEACON_ROOTS_ADDRESS,
                            Bytes::copy_from_slice(parent_beacon_block_root.as_slice()),
                        )
                        .map_err(BlockExecutionError::other)?;
                    state_changes.push(result.state.clone());
                    self.evm.db_mut().commit(result.state);
                }
                return Ok(());
            }
            apply_ethereum_pre_execution_changes(&mut self.evm, self.parent_hash, transitions)?;
        }
        Ok(())
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<Self::Result, BlockExecutionError> {
        self.execute_transaction_without_commit_with(tx, |evm, tx_env, transaction_hash| {
            evm.transact(tx_env).map_err(|err| BlockExecutionError::evm(err, transaction_hash))
        })
    }

    fn commit_transaction(&mut self, output: Self::Result) -> GasOutput {
        let AnvilTxResult {
            inner: EthTxResult { result: ResultAndState { result, state }, blob_gas_used, tx_type },
            #[cfg_attr(not(any(feature = "base", feature = "optimism")), allow(unused_variables))]
            sender,
        } = output;

        let gas_used = result.tx_gas_used();
        self.gas_used += gas_used;

        if self.spec_id >= SpecId::CANCUN {
            self.blob_gas_used = self.blob_gas_used.saturating_add(blob_gas_used);
        }

        #[cfg(any(feature = "base", feature = "optimism"))]
        let receipt = if tx_type.is_deposit() {
            optimism::build_mined_deposit_receipt(result, &state, sender, self.gas_used)
        } else {
            self.receipt_builder.build_receipt(ReceiptBuilderCtx {
                tx_type,
                evm: &self.evm,
                result,
                state: &state,
                cumulative_gas_used: self.gas_used,
            })
        };
        #[cfg(not(any(feature = "base", feature = "optimism")))]
        let receipt = self.receipt_builder.build_receipt(ReceiptBuilderCtx {
            tx_type,
            evm: &self.evm,
            result,
            state: &state,
            cumulative_gas_used: self.gas_used,
        });

        if let Some(state_changes) = &mut self.state_changes {
            state_changes.push(state.clone());
        }
        self.receipts.push(receipt);
        self.evm.db_mut().commit(state);

        GasOutput::new(gas_used)
    }

    fn finish(
        mut self,
    ) -> Result<(Self::Evm, BlockExecutionResult<FoundryReceiptEnvelope>), BlockExecutionError>
    {
        let requests = match self.ethereum_transitions {
            Some(transitions) if transitions.execution_kind == BlockExecutionKind::Complete => {
                apply_ethereum_post_execution_changes(&mut self.evm, transitions, &self.receipts)?
            }
            _ => Requests::default(),
        };
        Ok((
            self.evm,
            BlockExecutionResult {
                receipts: self.receipts,
                requests,
                gas_used: self.gas_used,
                blob_gas_used: self.blob_gas_used,
            },
        ))
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        &mut self.evm
    }

    fn evm(&self) -> &Self::Evm {
        &self.evm
    }

    fn receipts(&self) -> &[FoundryReceiptEnvelope] {
        &self.receipts
    }
}

/// Result of executing pool transactions against a block executor.
pub struct ExecutedPoolTransactions<T> {
    /// Successfully included transactions.
    pub included: Vec<Arc<PoolTransaction<T>>>,
    /// Transactions that failed validation.
    pub invalid: Vec<Arc<PoolTransaction<T>>>,
    /// Transactions skipped because they're not yet valid (e.g., valid_after in the future).
    /// These remain in the pool and should be retried later.
    pub not_yet_valid: Vec<Arc<PoolTransaction<T>>>,
    /// Per-transaction execution info.
    pub tx_info: Vec<TransactionInfo>,
    /// The raw pending transactions that were included (in order).
    pub txs: Vec<MaybeImpersonatedTransaction<T>>,
}

/// Gas-related configuration for pool transaction execution.
///
/// Bundles parameters that cannot be derived from the generic `Evm` trait
/// (which doesn't expose `cfg()`), so callers construct this from `EvmEnv`
/// before calling [`execute_pool_transactions`].
pub struct PoolTxGasConfig {
    pub disable_block_gas_limit: bool,
    pub tx_gas_limit_cap: Option<u64>,
    pub tx_gas_limit_cap_resolved: u64,
    pub max_blob_gas_per_block: u64,
    pub is_cancun: bool,
}

/// Hooks invoked around each candidate transaction's execution.
pub struct PoolTransactionHooks<BeforeTransaction, ExecuteTransaction, OnExecutionError> {
    /// Runs after validation and immediately before execution.
    pub before_transaction: BeforeTransaction,
    /// Executes the candidate through the network-specific transaction entry point.
    pub execute_transaction: ExecuteTransaction,
    /// Runs when execution fails before the candidate can be included.
    pub on_execution_error: OnExecutionError,
}

/// Executes a pool candidate through the block executor's ordinary transaction entry point.
pub(crate) fn execute_pool_transaction<B>(
    executor: &mut B,
    tx_env: <B::Evm as Evm>::Tx,
    recovered: Recovered<B::Transaction>,
    _is_replay: bool,
) -> Result<B::Result, BlockExecutionError>
where
    B: BlockExecutor,
{
    executor.execute_transaction_without_commit((tx_env, recovered))
}

/// Executes pool transactions against a block executor, handling validation,
/// execution, commit, inspector drain, and result collection.
///
/// This is the shared core of `do_mine_block` and `with_pending_block`.
#[allow(clippy::type_complexity)]
pub fn execute_pool_transactions<B, BeforeTransaction, ExecuteTransaction, OnExecutionError>(
    executor: &mut B,
    pool_transactions: &[Arc<PoolTransaction<B::Transaction>>],
    gas_config: &PoolTxGasConfig,
    inspector_config: &InspectorTxConfig,
    cheats: &CheatsManager,
    validator: &dyn Fn(
        &PoolTransaction<B::Transaction>,
        &AccountInfo,
    ) -> Result<(), InvalidTransactionError>,
    hooks: &mut PoolTransactionHooks<BeforeTransaction, ExecuteTransaction, OnExecutionError>,
) -> ExecutedPoolTransactions<B::Transaction>
where
    B: BlockExecutor<
            Transaction = FoundryTxEnvelope,
            Evm: Evm<DB: Database + Debug, Inspector = AnvilInspector>,
        >,
    B::Receipt: TxReceipt,
    <B::Result as TxResult>::HaltReason: Clone + IntoInstructionResult,
    <B::Evm as Evm>::Tx: FromTxWithEncoded<B::Transaction> + FoundryTransaction,
    BeforeTransaction: FnMut(&mut B::Evm, &<B::Evm as Evm>::Tx),
    ExecuteTransaction: FnMut(
        &mut B,
        <B::Evm as Evm>::Tx,
        Recovered<B::Transaction>,
        bool,
    ) -> Result<B::Result, BlockExecutionError>,
    OnExecutionError: FnMut(&mut B::Evm),
{
    let gas_limit = executor.evm().block().gas_limit();

    let mut included = Vec::new();
    let mut invalid = Vec::new();
    let mut not_yet_valid = Vec::new();
    let mut tx_info: Vec<TransactionInfo> = Vec::new();
    let mut transactions = Vec::new();
    let mut blob_gas_used = 0u64;

    for pool_tx in pool_transactions {
        let pending = &pool_tx.pending_transaction;
        let sender = *pending.sender();
        let block_timestamp = executor.evm().block().timestamp();

        if let FoundryTxEnvelope::Tempo(aa_tx) = pending.transaction.as_ref()
            && let Some(valid_after) = aa_tx.tx().valid_after
            && U256::from(valid_after.get()) > block_timestamp
        {
            trace!(target: "backend", "[{:?}] transaction not valid yet, will retry later", pool_tx.hash());
            not_yet_valid.push(pool_tx.clone());
            continue;
        }

        let account = match executor.evm_mut().db_mut().basic(sender).map(|a| a.unwrap_or_default())
        {
            Ok(acc) => acc,
            Err(err) => {
                trace!(target: "backend", ?err, "db error for tx {:?}, skipping", pool_tx.hash());
                continue;
            }
        };

        let tx_env =
            build_tx_env_for_pending::<B::Transaction, <B::Evm as Evm>::Tx>(pending, cheats);

        // Gas limit checks
        let cumulative_gas =
            executor.receipts().last().map(|r| r.cumulative_gas_used()).unwrap_or(0);
        let max_block_gas = cumulative_gas.saturating_add(pending.transaction.gas_limit());
        if !gas_config.disable_block_gas_limit && max_block_gas > gas_limit {
            trace!(target: "backend", tx_gas_limit = %pending.transaction.gas_limit(), ?pool_tx, "block gas limit exhausting, skipping transaction");
            continue;
        }

        // Osaka EIP-7825 tx gas limit cap check
        if gas_config.tx_gas_limit_cap.is_none()
            && pending.transaction.gas_limit() > gas_config.tx_gas_limit_cap_resolved
        {
            trace!(target: "backend", tx_gas_limit = %pending.transaction.gas_limit(), ?pool_tx, "transaction gas limit exhausting, skipping transaction");
            continue;
        }

        // Reject declared blob gas before execution. Network-specific accounting is checked again
        // against the execution result below.
        let declared_blob_gas = pending.transaction.blob_gas_used().unwrap_or(0);
        if blob_gas_used.saturating_add(declared_blob_gas) > gas_config.max_blob_gas_per_block {
            trace!(target: "backend", blob_gas = %declared_blob_gas, ?pool_tx, "block blob gas limit exhausting, skipping transaction");
            continue;
        }

        // Validate
        if let Err(err) = validator(pool_tx, &account) {
            warn!(target: "backend", "Skipping invalid tx execution [{:?}] {}", pool_tx.hash(), err);
            invalid.push(pool_tx.clone());
            continue;
        }

        let nonce = account.nonce;

        (hooks.before_transaction)(executor.evm_mut(), &tx_env);
        let recovered = Recovered::new_unchecked(pending.transaction.as_ref().clone(), sender);
        trace!(target: "backend", "[{:?}] executing", pool_tx.hash());
        match (hooks.execute_transaction)(executor, tx_env, recovered, pool_tx.is_replay) {
            Ok(result) => {
                let exec_result = result.result().result.clone();
                let gas_used = result.result().result.tx_gas_used();

                executor.commit_transaction(result);

                let traces =
                    executor.evm_mut().inspector_mut().finish_transaction(inspector_config);

                if gas_config.is_cancun {
                    blob_gas_used = blob_gas_used.saturating_add(declared_blob_gas);
                }

                let (exit_reason, out, _logs) = match exec_result {
                    ExecutionResult::Success { reason, logs, output, .. } => {
                        (reason.into(), Some(output), logs)
                    }
                    ExecutionResult::Revert { output, .. } => {
                        (InstructionResult::Revert, Some(Output::Call(output)), Vec::new())
                    }
                    ExecutionResult::Halt { reason, .. } => {
                        (reason.into_instruction_result(), None, Vec::new())
                    }
                };

                if exit_reason == InstructionResult::OutOfGas {
                    warn!(target: "backend", "[{:?}] executed with out of gas", pool_tx.hash());
                }

                trace!(target: "backend", ?exit_reason, ?gas_used, "[{:?}] executed with out={:?}", pool_tx.hash(), out);
                trace!(target: "backend::executor", "transacted [{:?}], result: {:?} gas {}", pool_tx.hash(), exit_reason, gas_used);

                let contract_address = pending.transaction.to().is_none().then(|| {
                    let addr = sender.create(nonce);
                    trace!(target: "backend", "Contract creation tx: computed address {:?}", addr);
                    addr
                });

                // TODO: replace `TransactionInfo` with alloy receipt/transaction types
                let transaction_index = tx_info.len() as u64;
                let info = TransactionInfo {
                    transaction_hash: pool_tx.hash(),
                    transaction_index,
                    from: sender,
                    to: pending.transaction.to(),
                    contract_address,
                    traces,
                    exit: exit_reason,
                    out: out.map(Output::into_data),
                    nonce,
                    gas_used,
                };

                included.push(pool_tx.clone());
                tx_info.push(info);
                transactions.push(pending.transaction.clone());
            }
            Err(err) => {
                (hooks.on_execution_error)(executor.evm_mut());
                executor.evm_mut().inspector_mut().discard_transaction(inspector_config);
                if err.as_validation().is_some() {
                    warn!(target: "backend", "Skipping invalid tx [{:?}]: {}", pool_tx.hash(), err);
                    invalid.push(pool_tx.clone());
                } else {
                    trace!(target: "backend", ?err, "tx execution error, skipping {:?}", pool_tx.hash());
                }
            }
        }
    }

    ExecutedPoolTransactions { included, invalid, not_yet_valid, tx_info, txs: transactions }
}

/// Builds the EVM transaction env from a pending pool transaction.
pub fn build_tx_env_for_pending<Tx, T>(tx: &PendingTransaction<Tx>, cheats: &CheatsManager) -> T
where
    Tx: Transaction + Encodable2718,
    T: FromTxWithEncoded<Tx> + FoundryTransaction,
{
    let encoded = tx.transaction.encoded_2718().into();
    let mut tx_env: T =
        FromTxWithEncoded::from_encoded_tx(tx.transaction.as_ref(), *tx.sender(), encoded);

    if let Some(signed_auths) = tx.transaction.authorization_list()
        && cheats.has_recover_overrides()
    {
        let auth_list = tx_env.authorization_list_mut();
        let cheated_auths = signed_auths
            .iter()
            .zip(take(auth_list))
            .map(|(signed_auth, either_auth)| {
                either_auth.right_and_then(|recovered_auth| {
                    if recovered_auth.authority().is_none()
                        && let Ok(signature) = signed_auth.signature()
                        && let Some(override_addr) =
                            cheats.get_recover_override(&signature.as_bytes().into())
                    {
                        Either::Right(RecoveredAuthorization::new_unchecked(
                            recovered_auth.into_parts().0,
                            RecoveredAuthority::Valid(override_addr),
                        ))
                    } else {
                        Either::Right(recovered_auth)
                    }
                })
            })
            .collect();
        *tx_env.authorization_list_mut() = cheated_auths;
    }

    tx_env
}

/// Returns the block's blob gas budget.
///
/// OP Jovian repurposes the block gas limit as the DA-footprint budget. Every other execution
/// profile uses the active EIP-4844 limit.
pub(crate) const fn block_blob_gas_limit(
    optimism_jovian: bool,
    block_gas_limit: u64,
    max_blob_gas_per_block: u64,
) -> u64 {
    if optimism_jovian { block_gas_limit } else { max_blob_gas_per_block }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_eips::{
        eip6110::MAINNET_DEPOSIT_CONTRACT_ADDRESS, eip7002::WITHDRAWAL_REQUEST_TYPE,
        eip7251::CONSOLIDATION_REQUEST_TYPE,
    };
    use alloy_sol_types::{SolEvent, sol};

    sol! {
        event DepositEvent(
            bytes pubkey,
            bytes withdrawal_credentials,
            bytes amount,
            bytes signature,
            bytes index
        );
    }

    #[test]
    fn prague_requests_use_consensus_order() {
        let event = DepositEvent {
            pubkey: Bytes::from(vec![0x11; 48]),
            withdrawal_credentials: Bytes::from(vec![0x22; 32]),
            amount: Bytes::from(vec![0x33; 8]),
            signature: Bytes::from(vec![0x44; 96]),
            index: Bytes::from(vec![0x55; 8]),
        };
        let log = DepositEvent::encode_log(&Log {
            address: MAINNET_DEPOSIT_CONTRACT_ADDRESS,
            data: event,
        });
        let receipt =
            Receipt { status: Eip658Value::Eip658(true), cumulative_gas_used: 0, logs: vec![log] }
                .with_bloom();
        let receipts = [FoundryReceiptEnvelope::Legacy(receipt)];
        let mut requests = Requests::default();

        append_deposit_requests(
            ActiveEthereumSpec {
                hardfork: EthereumHardfork::Prague,
                deposit_contract_address: MAINNET_DEPOSIT_CONTRACT_ADDRESS,
            },
            &receipts,
            &mut requests,
        )
        .unwrap();
        requests.push_request_with_type(WITHDRAWAL_REQUEST_TYPE, [0xaa]);
        requests.push_request_with_type(CONSOLIDATION_REQUEST_TYPE, [0xbb]);

        assert_eq!(
            requests.iter().map(|request| request[0]).collect::<Vec<_>>(),
            [DEPOSIT_REQUEST_TYPE, WITHDRAWAL_REQUEST_TYPE, CONSOLIDATION_REQUEST_TYPE]
        );
    }

    #[test]
    fn deposit_requests_use_configured_contract_address() {
        let configured_address = Address::repeat_byte(0x42);
        let event = DepositEvent {
            pubkey: Bytes::from(vec![0x11; 48]),
            withdrawal_credentials: Bytes::from(vec![0x22; 32]),
            amount: Bytes::from(vec![0x33; 8]),
            signature: Bytes::from(vec![0x44; 96]),
            index: Bytes::from(vec![0x55; 8]),
        };
        let configured_log =
            DepositEvent::encode_log(&Log { address: configured_address, data: event.clone() });
        let mainnet_log = DepositEvent::encode_log(&Log {
            address: MAINNET_DEPOSIT_CONTRACT_ADDRESS,
            data: event,
        });
        let receipt = Receipt {
            status: Eip658Value::Eip658(true),
            cumulative_gas_used: 0,
            logs: vec![mainnet_log, configured_log],
        }
        .with_bloom();
        let receipts = [FoundryReceiptEnvelope::Legacy(receipt)];
        let mut requests = Requests::default();

        append_deposit_requests(
            ActiveEthereumSpec {
                hardfork: EthereumHardfork::Prague,
                deposit_contract_address: configured_address,
            },
            &receipts,
            &mut requests,
        )
        .unwrap();

        let request = requests.first().expect("configured deposit should be collected");
        assert_eq!(request[0], DEPOSIT_REQUEST_TYPE);
        assert_eq!(request.len(), 1 + 48 + 32 + 8 + 96 + 8);
    }
}
