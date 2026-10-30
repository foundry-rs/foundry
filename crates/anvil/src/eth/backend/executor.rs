//! Block execution on evm2.
//!
//! [`AnvilBlockExecutor`] executes a block's transactions on one evm2 EVM: every transaction is
//! committed to the EVM's accepted overlay and recorded in a [`BlockStateAccumulator`], which the
//! caller applies to Anvil's database once the block is complete.

use crate::{
    eth::{
        backend::{
            cheats::CheatsManager,
            evm::{
                AnvilEvmNetwork, ExecResult, TxContext, TxPayload, TypesFor, evm_tx, exec_result,
            },
        },
        error::{BlockchainError, InvalidTransactionError},
        pool::transactions::{PoolTransaction, TxMarker},
    },
    mem::inspector::{AnvilInspector, InspectorTxConfig},
};
use alloy_consensus::{Eip658Value, Receipt, Transaction, TransactionEnvelope};
use alloy_eips::{
    eip6110::DEPOSIT_REQUEST_TYPE,
    eip7002::WITHDRAWAL_REQUEST_TYPE,
    eip7251::CONSOLIDATION_REQUEST_TYPE,
    eip7685::Requests,
    eip7928::BlockAccessList,
    eip8282::{BUILDER_DEPOSIT_REQUEST_TYPE, BUILDER_EXIT_REQUEST_TYPE},
};
use alloy_hardforks::EthereumHardfork;
use alloy_primitives::{Address, B256, Bytes, Log, U256, map::HashSet};
use alloy_sol_types::{SolEvent, sol};
use anvil_core::eth::transaction::{MaybeImpersonatedTransaction, TransactionInfo};
use evm2::{
    AccountInfo, Evm,
    evm::{
        BEACON_ROOTS_ADDRESS, BUILDER_DEPOSIT_REQUEST_ADDRESS, BUILDER_EXIT_REQUEST_ADDRESS,
        BlockAccessIndex, BlockStateAccumulator, CONSOLIDATION_REQUEST_ADDRESS,
        HISTORY_STORAGE_ADDRESS, SystemTx, WITHDRAWAL_REQUEST_ADDRESS,
    },
};
use foundry_evm_hardforks::FoundryHardfork;
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxEnvelope, FoundryTxType};
use std::{fmt, sync::Arc};
use tempo_precompiles::{NONCE_PRECOMPILE_ADDRESS, nonce::NonceManager};

#[cfg(any(feature = "base", feature = "optimism"))]
pub(crate) mod optimism;

sol! {
    event DepositEvent(
        bytes pubkey,
        bytes withdrawal_credentials,
        bytes amount,
        bytes signature,
        bytes index
    );
}

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

/// Outputs of a complete block execution.
#[derive(Debug, Default)]
pub struct BlockExecutionResult {
    /// Receipts of the executed transactions.
    pub receipts: Vec<FoundryReceiptEnvelope>,
    /// EIP-7685 requests.
    pub requests: Requests,
    /// Gas used by the block's transactions.
    pub gas_used: u64,
    /// Blob gas used by the block's transactions.
    pub blob_gas_used: u64,
}

/// Builds the receipt of a transaction type that does not require network-specific metadata.
///
/// `eip8130_phase_statuses` are the per-phase statuses of a Base EIP-8130 transaction, and are
/// ignored for other transaction types.
pub(crate) fn build_receipt(
    tx_type: FoundryTxType,
    success: bool,
    logs: Vec<Log>,
    cumulative_gas_used: u64,
    eip8130_phase_statuses: Option<Vec<u8>>,
) -> FoundryReceiptEnvelope {
    #[cfg(not(feature = "base"))]
    let _ = eip8130_phase_statuses;
    let receipt =
        Receipt { status: Eip658Value::Eip658(success), cumulative_gas_used, logs }.with_bloom();
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
        FoundryTxType::Eip8130 => {
            FoundryReceiptEnvelope::Eip8130(alloy_consensus::ReceiptWithBloom {
                receipt: base_common_consensus::Eip8130Receipt::new(
                    receipt.receipt,
                    eip8130_phase_statuses.unwrap_or_default(),
                ),
                logs_bloom: receipt.logs_bloom,
            })
        }
        FoundryTxType::Tempo => FoundryReceiptEnvelope::Tempo(receipt),
        FoundryTxType::Celo => FoundryReceiptEnvelope::Unknown(alloy_network::AnyReceiptEnvelope {
            inner: receipt,
            r#type: foundry_primitives::CIP64_TX_TYPE,
        }),
    }
}

/// Result of executing pool transactions against a block executor.
pub struct ExecutedPoolTransactions<T> {
    /// Successfully included transactions.
    pub included: Vec<Arc<PoolTransaction<T>>>,
    /// Transactions whose nonce was already consumed by the current state.
    pub stale: Vec<Arc<PoolTransaction<T>>>,
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
pub struct PoolTxGasConfig {
    pub disable_block_gas_limit: bool,
    /// Resolved transaction gas cap, or `None` when the caller disables this check.
    pub enforced_tx_gas_limit_cap: Option<u64>,
    pub max_blob_gas_per_block: u64,
    pub is_cancun: bool,
}

/// A transaction executed and committed by [`AnvilBlockExecutor`].
#[derive(Debug)]
pub(crate) struct CommittedTx {
    /// Network-neutral result.
    pub(crate) result: ExecResult,
    /// Traces recorded while executing it.
    pub(crate) traces: Vec<evm2_inspectors::tracing::types::CallTraceNode>,
}

/// Executes the transactions of one block on an evm2 EVM.
pub struct AnvilBlockExecutor<'a, N: AnvilEvmNetwork> {
    /// The EVM executing the block.
    pub(crate) evm: Evm<'a, TypesFor<N>>,
    /// State changes of the block.
    pub(crate) state: BlockStateAccumulator,
    /// Inspector lent to the EVM for each transaction.
    pub(crate) inspector: AnvilInspector,
    /// Parent block hash — needed for the EIP-2935 system call.
    parent_hash: B256,
    /// Canonical Ethereum consensus transitions, disabled for other networks.
    ethereum_transitions: Option<EthereumBlockTransitions>,
    /// Receipts of executed transactions.
    pub(crate) receipts: Vec<FoundryReceiptEnvelope>,
    /// Total gas used by transactions in this block.
    pub(crate) gas_used: u64,
    /// Blob gas used by the block.
    pub(crate) blob_gas_used: u64,
    /// Whether blob gas is accounted (Cancun and later).
    is_cancun: bool,
    /// Maximum blob gas available to transactions in this block.
    max_blob_gas_per_block: u64,
    /// Whether OP Jovian repurposes `blobGasUsed` for the DA footprint.
    #[cfg(feature = "optimism")]
    optimism_jovian: bool,
    /// The OP-stack hardfork that gates deposit receipt metadata.
    #[cfg(any(feature = "base", feature = "optimism"))]
    pub(crate) deposit_hardfork: Option<FoundryHardfork>,
    /// Addresses of the EVM's precompiles, for trace annotation.
    precompiles: HashSet<Address>,
    /// Whether an EIP-7928 block access list is built.
    bal: bool,
    /// Cheat state applied to EIP-7702 authorizations.
    cheats: Option<CheatsManager>,
}

impl<N: AnvilEvmNetwork> fmt::Debug for AnvilBlockExecutor<'_, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilBlockExecutor")
            .field("parent_hash", &self.parent_hash)
            .field("ethereum_transitions", &self.ethereum_transitions)
            .field("gas_used", &self.gas_used)
            .field("blob_gas_used", &self.blob_gas_used)
            .field("receipts", &self.receipts.len())
            .finish_non_exhaustive()
    }
}

impl<'a, N: AnvilEvmNetwork> AnvilBlockExecutor<'a, N> {
    /// Creates an executor over `evm`, inspecting with `inspector`.
    pub(crate) fn new(
        evm: Evm<'a, TypesFor<N>>,
        inspector: AnvilInspector,
        parent_hash: B256,
        is_cancun: bool,
        ethereum_transitions: Option<EthereumBlockTransitions>,
    ) -> Self {
        let precompiles = evm.precompiles().addresses().into_iter().collect();
        Self {
            evm,
            state: BlockStateAccumulator::new(),
            inspector,
            parent_hash,
            ethereum_transitions,
            receipts: Vec::new(),
            gas_used: 0,
            blob_gas_used: 0,
            is_cancun,
            max_blob_gas_per_block: u64::MAX,
            #[cfg(feature = "optimism")]
            optimism_jovian: false,
            #[cfg(any(feature = "base", feature = "optimism"))]
            deposit_hardfork: None,
            precompiles,
            bal: false,
            cheats: None,
        }
    }

    /// Applies the active block blob gas limit.
    pub(crate) const fn with_max_blob_gas_per_block(mut self, limit: u64) -> Self {
        self.max_blob_gas_per_block = limit;
        self
    }

    /// Applies ecrecover cheat overrides to EIP-7702 authorizations.
    pub(crate) fn with_cheats(mut self, cheats: CheatsManager) -> Self {
        self.cheats = Some(cheats);
        self
    }

    /// Builds an EIP-7928 block access list while executing.
    pub(crate) fn with_block_access_list(mut self) -> Self {
        self.bal = true;
        self.evm.state_mut().enable_bal_builder();
        self.evm.state_mut().set_bal_index(BlockAccessIndex::new(0));
        self
    }

    /// Configures OP-specific block accounting.
    #[cfg(any(feature = "base", feature = "optimism"))]
    pub(crate) fn with_deposit_hardfork(mut self, hardfork: FoundryHardfork) -> Self {
        #[cfg(feature = "optimism")]
        {
            self.optimism_jovian = matches!(hardfork, FoundryHardfork::Optimism(_))
                && foundry_evm_hardforks::OpHardfork::from(hardfork)
                    >= foundry_evm_hardforks::OpHardfork::Jovian;
        }
        self.deposit_hardfork = Some(hardfork);
        self
    }

    /// Returns the account at `address` as seen by the next transaction.
    pub(crate) fn account(&mut self, address: Address) -> Result<AccountInfo, BlockchainError> {
        Ok(self.evm.read_account_info(&address)?.unwrap_or_default())
    }

    /// Returns the storage slot as seen by the next transaction.
    pub(crate) fn storage(
        &mut self,
        address: Address,
        slot: U256,
    ) -> Result<U256, BlockchainError> {
        Ok(self.evm.state_mut().storage_slot_untracked(&address, &slot)?)
    }

    fn system_call(&mut self, address: Address, data: Bytes) -> Result<Bytes, BlockchainError> {
        let result = self.evm.system_call(SystemTx::new(address, data))?.commit_to(&mut self.state);
        if result.stop.is_revert() {
            return Err(InvalidTransactionError::Revert(Some(result.output)).into());
        }
        if !result.status {
            return Err(BlockchainError::Message(format!(
                "system call to {address} failed: {:?}",
                result.stop
            )));
        }
        Ok(result.output)
    }

    /// Applies canonical Ethereum block-start transitions in consensus order.
    pub(crate) fn apply_pre_execution_changes(&mut self) -> Result<(), BlockchainError> {
        let Some(transitions) = self.ethereum_transitions else { return Ok(()) };
        if transitions.hardfork >= EthereumHardfork::Prague {
            self.system_call(HISTORY_STORAGE_ADDRESS, Bytes::from(self.parent_hash))?;
        }
        if transitions.hardfork >= EthereumHardfork::Cancun {
            let root = transitions.parent_beacon_block_root.ok_or_else(|| {
                BlockchainError::Message("missing parent beacon block root".to_string())
            })?;
            self.system_call(BEACON_ROOTS_ADDRESS, Bytes::from(root))?;
        }
        Ok(())
    }

    /// Executes and commits one transaction, returning its result and traces.
    pub(crate) fn execute_transaction(
        &mut self,
        tx: &FoundryTxEnvelope,
        sender: Address,
        context: &TxContext,
        inspector_config: &InspectorTxConfig,
    ) -> Result<CommittedTx, BlockchainError> {
        let block_available_gas =
            self.evm.block().gas_limit.saturating_to::<u64>().saturating_sub(self.gas_used);
        if tx.gas_limit() > block_available_gas {
            return Err(InvalidTransactionError::GasTooHigh(crate::eth::error::ErrDetail {
                detail: format!(
                    "transaction gas limit {} exceeds available block gas {block_available_gas}",
                    tx.gas_limit()
                ),
            })
            .into());
        }

        // The deposit nonce reported in the receipt is the sender nonce before the deposit ran.
        #[cfg(any(feature = "base", feature = "optimism"))]
        let depositor_nonce =
            if tx.is_deposit() { Some(self.account(sender)?.nonce) } else { None };

        #[cfg(feature = "optimism")]
        let blob_gas_used = optimism::blob_gas_used(&mut self.evm, tx, self.optimism_jovian)?;
        #[cfg(not(feature = "optimism"))]
        let blob_gas_used = tx.blob_gas_used().unwrap_or_default();
        #[cfg(feature = "optimism")]
        let blob_gas_limit = block_blob_gas_limit(
            self.optimism_jovian,
            self.evm.block().gas_limit.saturating_to(),
            self.max_blob_gas_per_block,
        );
        #[cfg(not(feature = "optimism"))]
        let blob_gas_limit = self.max_blob_gas_per_block;
        if self.blob_gas_used.saturating_add(blob_gas_used) > blob_gas_limit {
            return Err(BlockchainError::Message("block blob gas limit exceeded".to_string()));
        }

        let evm_tx = evm_tx::<N>(tx, sender, TxPayload::Signed, self.cheats.as_ref())?;
        N::set_tx_context(&mut self.evm, &evm_tx, context);
        if self.bal {
            // EIP-7928 block access index 0 holds the pre-block system writes, transaction `i` is
            // index `i + 1` and the post-block system writes follow the last transaction.
            self.evm
                .state_mut()
                .set_bal_index(BlockAccessIndex::new(self.receipts.len() as u64 + 1));
        }
        let result = match self.evm.transact_with_inspector(&evm_tx, &mut self.inspector) {
            Ok(executed) => exec_result::<N>(executed.commit_to(&mut self.state)),
            Err(err) => {
                self.inspector.discard_transaction(inspector_config);
                return Err(err.into());
            }
        };

        let gas_used = result.tx_gas_used();
        self.gas_used += gas_used;
        if self.is_cancun {
            self.blob_gas_used = self.blob_gas_used.saturating_add(blob_gas_used);
        }

        #[cfg(any(feature = "base", feature = "optimism"))]
        let receipt = if let Some(depositor_nonce) = depositor_nonce {
            optimism::build_deposit_receipt(
                result.status,
                result.logs.clone(),
                self.deposit_hardfork,
                depositor_nonce,
                self.gas_used,
            )
        } else {
            build_receipt(
                tx.tx_type(),
                result.status,
                result.logs.clone(),
                self.gas_used,
                result.ext.eip8130_phase_statuses.clone(),
            )
        };
        #[cfg(not(any(feature = "base", feature = "optimism")))]
        let receipt = build_receipt(
            tx.tx_type(),
            result.status,
            result.logs.clone(),
            self.gas_used,
            result.ext.eip8130_phase_statuses.clone(),
        );
        self.receipts.push(receipt);

        let precompiles = &self.precompiles;
        let traces = self
            .inspector
            .finish_transaction(inspector_config, |address| precompiles.contains(address));
        Ok(CommittedTx { result, traces })
    }

    /// Applies post-block transitions and returns the block's outputs, state changes and access
    /// list.
    pub(crate) fn finish(
        mut self,
    ) -> Result<
        (BlockExecutionResult, BlockStateAccumulator, Option<BlockAccessList>),
        BlockchainError,
    > {
        if self.bal {
            self.evm
                .state_mut()
                .set_bal_index(BlockAccessIndex::new(self.receipts.len() as u64 + 1));
        }
        let requests = match self.ethereum_transitions {
            Some(transitions)
                if transitions.execution_kind == BlockExecutionKind::Complete
                    && transitions.hardfork >= EthereumHardfork::Prague =>
            {
                let mut requests = Requests::default();
                append_deposit_requests(
                    transitions.deposit_contract_address,
                    &self.receipts,
                    &mut requests,
                )?;
                let withdrawals = self.system_call(WITHDRAWAL_REQUEST_ADDRESS, Bytes::new())?;
                if !withdrawals.is_empty() {
                    requests.push_request_with_type(WITHDRAWAL_REQUEST_TYPE, withdrawals);
                }
                let consolidations =
                    self.system_call(CONSOLIDATION_REQUEST_ADDRESS, Bytes::new())?;
                if !consolidations.is_empty() {
                    requests.push_request_with_type(CONSOLIDATION_REQUEST_TYPE, consolidations);
                }
                if transitions.hardfork >= EthereumHardfork::Amsterdam {
                    let deposits =
                        self.system_call(BUILDER_DEPOSIT_REQUEST_ADDRESS, Bytes::new())?;
                    if !deposits.is_empty() {
                        requests.push_request_with_type(BUILDER_DEPOSIT_REQUEST_TYPE, deposits);
                    }
                    let exits = self.system_call(BUILDER_EXIT_REQUEST_ADDRESS, Bytes::new())?;
                    if !exits.is_empty() {
                        requests.push_request_with_type(BUILDER_EXIT_REQUEST_TYPE, exits);
                    }
                }
                requests
            }
            _ => Requests::default(),
        };
        let bal =
            if self.bal { self.evm.overlay_db_mut().bal_context.take_alloy_bal() } else { None };
        Ok((
            BlockExecutionResult {
                receipts: self.receipts,
                requests,
                gas_used: self.gas_used,
                blob_gas_used: self.blob_gas_used,
            },
            self.state,
            bal,
        ))
    }

    /// Executes pool transactions, handling validation, execution, commit and result collection.
    ///
    /// This is the shared core of mining and pending-block construction. `context` returns the
    /// network context of a candidate, and `rollback_context` undoes it when the candidate is not
    /// included.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub(crate) fn execute_pool_transactions(
        &mut self,
        pool_transactions: &[Arc<PoolTransaction<FoundryTxEnvelope>>],
        gas_config: &PoolTxGasConfig,
        inspector_config: &InspectorTxConfig,
        validator: &dyn Fn(
            &PoolTransaction<FoundryTxEnvelope>,
            &AccountInfo,
        ) -> Result<(), InvalidTransactionError>,
        context: &mut dyn FnMut(&PoolTransaction<FoundryTxEnvelope>) -> TxContext,
        rollback_context: &mut dyn FnMut(),
    ) -> ExecutedPoolTransactions<FoundryTxEnvelope> {
        let gas_limit = self.evm.block().gas_limit.saturating_to::<u64>();
        let timestamp = self.evm.block().timestamp;

        let mut included = Vec::new();
        let mut stale = Vec::new();
        let mut invalid = Vec::new();
        let mut not_yet_valid = Vec::new();
        let mut tx_info = Vec::<TransactionInfo>::new();
        let mut transactions = Vec::new();
        let mut blob_gas_used = 0u64;
        let mut unavailable_markers = HashSet::<TxMarker>::default();

        for pool_tx in pool_transactions {
            let pending = &pool_tx.pending_transaction;
            let sender = *pending.sender();
            if pool_tx.requires.iter().any(|marker| unavailable_markers.contains(marker)) {
                trace!(target: "backend", "[{:?}] dependency was not included, skipping transaction", pool_tx.hash());
                unavailable_markers.extend(pool_tx.provides.iter().cloned());
                continue;
            }

            let account = match self.account(sender) {
                Ok(account) => account,
                Err(err) => {
                    trace!(target: "backend", ?err, "db error for tx {:?}, skipping", pool_tx.hash());
                    unavailable_markers.extend(pool_tx.provides.iter().cloned());
                    continue;
                }
            };

            let transaction = pending.transaction.as_ref();
            let state_nonce = match transaction {
                FoundryTxEnvelope::Tempo(tx) if tx.tx().nonce_key == U256::MAX => None,
                FoundryTxEnvelope::Tempo(tx) if !tx.tx().nonce_key.is_zero() => {
                    let slot = NonceManager::new().nonces[sender][tx.tx().nonce_key].slot();
                    match self.storage(NONCE_PRECOMPILE_ADDRESS, slot) {
                        Ok(nonce) => Some(nonce.saturating_to::<u64>()),
                        Err(err) => {
                            trace!(target: "backend", ?err, "db error for tx {:?}, skipping", pool_tx.hash());
                            unavailable_markers.extend(pool_tx.provides.iter().cloned());
                            continue;
                        }
                    }
                }
                #[cfg(feature = "base")]
                FoundryTxEnvelope::Eip8130(_) => None,
                #[cfg(any(feature = "base", feature = "optimism"))]
                FoundryTxEnvelope::Deposit(_) => None,
                #[cfg(feature = "optimism")]
                FoundryTxEnvelope::PostExec(_) => None,
                _ => Some(account.nonce),
            };
            if state_nonce.is_some_and(|nonce| transaction.nonce() < nonce) {
                warn!(target: "backend", "Skipping stale tx [{:?}]", pool_tx.hash());
                stale.push(pool_tx.clone());
                continue;
            }

            if let FoundryTxEnvelope::Tempo(aa_tx) = transaction
                && let Some(valid_after) = aa_tx.tx().valid_after
                && U256::from(valid_after.get()) > timestamp
            {
                trace!(target: "backend", "[{:?}] transaction not valid yet, will retry later", pool_tx.hash());
                not_yet_valid.push(pool_tx.clone());
                unavailable_markers.extend(pool_tx.provides.iter().cloned());
                continue;
            }

            // Gas limit checks
            let cumulative_gas = self.receipts.last().map(|r| r.cumulative_gas_used()).unwrap_or(0);
            let max_block_gas = cumulative_gas.saturating_add(pending.transaction.gas_limit());
            if !gas_config.disable_block_gas_limit && max_block_gas > gas_limit {
                trace!(target: "backend", tx_gas_limit = %pending.transaction.gas_limit(), tx_hash = ?pool_tx.hash(), "block gas limit exhausting, skipping transaction");
                unavailable_markers.extend(pool_tx.provides.iter().cloned());
                continue;
            }

            // Osaka EIP-7825 tx gas limit cap check
            if let Some(tx_gas_limit_cap) = gas_config.enforced_tx_gas_limit_cap
                && pending.transaction.gas_limit() > tx_gas_limit_cap
            {
                trace!(target: "backend", tx_gas_limit = %pending.transaction.gas_limit(), tx_hash = ?pool_tx.hash(), "transaction gas limit exhausting, skipping transaction");
                unavailable_markers.extend(pool_tx.provides.iter().cloned());
                continue;
            }

            // Reject declared blob gas before execution. Network-specific accounting is checked
            // again against the execution result below.
            let declared_blob_gas = pending.transaction.blob_gas_used().unwrap_or(0);
            if blob_gas_used.saturating_add(declared_blob_gas) > gas_config.max_blob_gas_per_block {
                trace!(target: "backend", blob_gas = %declared_blob_gas, tx_hash = ?pool_tx.hash(), "block blob gas limit exhausting, skipping transaction");
                unavailable_markers.extend(pool_tx.provides.iter().cloned());
                continue;
            }

            // Validate
            if let Err(err) = validator(pool_tx, &account) {
                warn!(target: "backend", "Skipping invalid tx execution [{:?}] {}", pool_tx.hash(), err);
                invalid.push(pool_tx.clone());
                continue;
            }

            let nonce = account.nonce;
            trace!(target: "backend", "[{:?}] executing", pool_tx.hash());
            let tx_context = context(pool_tx);
            match self.execute_transaction(transaction, sender, &tx_context, inspector_config) {
                Ok(CommittedTx { result, traces }) => {
                    if gas_config.is_cancun {
                        blob_gas_used = blob_gas_used.saturating_add(declared_blob_gas);
                    }
                    let gas_used = result.tx_gas_used();
                    if result.stop.is_out_of_gas() {
                        warn!(target: "backend", "[{:?}] executed with out of gas", pool_tx.hash());
                    }
                    trace!(target: "backend::executor", "transacted [{:?}], result: {:?} gas {}", pool_tx.hash(), result.stop, gas_used);

                    let info = transaction_info(
                        pool_tx.hash(),
                        tx_info.len() as u64,
                        sender,
                        transaction,
                        nonce,
                        &result,
                        traces,
                    );
                    included.push(pool_tx.clone());
                    tx_info.push(info);
                    transactions.push(pending.transaction.clone());
                }
                Err(err) => {
                    rollback_context();
                    if matches!(err, BlockchainError::InvalidTransaction(_)) {
                        warn!(target: "backend", "Skipping invalid tx [{:?}]: {}", pool_tx.hash(), err);
                        invalid.push(pool_tx.clone());
                    } else {
                        unavailable_markers.extend(pool_tx.provides.iter().cloned());
                        trace!(target: "backend", ?err, "tx execution error, skipping {:?}", pool_tx.hash());
                    }
                }
            }
        }

        ExecutedPoolTransactions {
            included,
            stale,
            invalid,
            not_yet_valid,
            tx_info,
            txs: transactions,
        }
    }
}

/// Builds the stored execution info of an included transaction.
pub(crate) fn transaction_info(
    transaction_hash: B256,
    transaction_index: u64,
    sender: Address,
    transaction: &FoundryTxEnvelope,
    nonce: u64,
    result: &ExecResult,
    traces: Vec<evm2_inspectors::tracing::types::CallTraceNode>,
) -> TransactionInfo {
    let contract_address = transaction.to().is_none().then(|| sender.create(nonce));
    TransactionInfo {
        transaction_hash,
        transaction_index,
        from: sender,
        to: transaction.to(),
        contract_address,
        traces,
        exit: result.stop,
        out: (!result.stop.is_halt()).then(|| result.output.clone()),
        nonce,
        gas_used: result.tx_gas_used(),
    }
}

/// Collects EIP-6110 deposit requests from the deposit contract's logs.
pub(crate) fn append_deposit_requests(
    deposit_contract_address: Address,
    receipts: &[FoundryReceiptEnvelope],
    requests: &mut Requests,
) -> Result<(), BlockchainError> {
    let mut deposits = Vec::new();
    for log in receipts.iter().flat_map(|receipt| receipt.logs()) {
        if log.address != deposit_contract_address
            || log.topics().first() != Some(&DepositEvent::SIGNATURE_HASH)
        {
            continue;
        }
        let event = DepositEvent::decode_log_data(&log.data).map_err(|err| {
            BlockchainError::Message(format!("failed to decode deposit request: {err}"))
        })?;
        deposits.extend_from_slice(&event.pubkey);
        deposits.extend_from_slice(&event.withdrawal_credentials);
        deposits.extend_from_slice(&event.amount);
        deposits.extend_from_slice(&event.signature);
        deposits.extend_from_slice(&event.index);
    }
    if !deposits.is_empty() {
        requests.push_request_with_type(DEPOSIT_REQUEST_TYPE, deposits);
    }
    Ok(())
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
    use alloy_eips::eip6110::MAINNET_DEPOSIT_CONTRACT_ADDRESS;

    fn deposit_log(address: Address) -> Log {
        let event = DepositEvent {
            pubkey: Bytes::from(vec![0x11; 48]),
            withdrawal_credentials: Bytes::from(vec![0x22; 32]),
            amount: Bytes::from(vec![0x33; 8]),
            signature: Bytes::from(vec![0x44; 96]),
            index: Bytes::from(vec![0x55; 8]),
        };
        DepositEvent::encode_log(&Log { address, data: event })
    }

    #[test]
    fn prague_requests_use_consensus_order() {
        let receipt = Receipt {
            status: Eip658Value::Eip658(true),
            cumulative_gas_used: 0,
            logs: vec![deposit_log(MAINNET_DEPOSIT_CONTRACT_ADDRESS)],
        }
        .with_bloom();
        let receipts = [FoundryReceiptEnvelope::Legacy(receipt)];
        let mut requests = Requests::default();

        append_deposit_requests(MAINNET_DEPOSIT_CONTRACT_ADDRESS, &receipts, &mut requests)
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
        let receipt = Receipt {
            status: Eip658Value::Eip658(true),
            cumulative_gas_used: 0,
            logs: vec![
                deposit_log(MAINNET_DEPOSIT_CONTRACT_ADDRESS),
                deposit_log(configured_address),
            ],
        }
        .with_bloom();
        let receipts = [FoundryReceiptEnvelope::Legacy(receipt)];
        let mut requests = Requests::default();

        append_deposit_requests(configured_address, &receipts, &mut requests).unwrap();

        let request = requests.first().expect("configured deposit should be collected");
        assert_eq!(request[0], DEPOSIT_REQUEST_TYPE);
        assert_eq!(request.len(), 1 + 48 + 32 + 8 + 96 + 8);
    }
}
