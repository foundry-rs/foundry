use crate::{
    eth::{
        backend::{
            db::Db, mem::op_haltreason_to_instruction_result, validate::TransactionValidator,
        },
        error::InvalidTransactionError,
        pool::transactions::PoolTransaction,
    },
    // inject_precompiles,
    mem::inspector::AnvilInspector,
    PrecompileFactory,
};
use alloy_consensus::{constants::EMPTY_WITHDRAWALS, Receipt, ReceiptWithBloom};
use alloy_eips::{eip2718::Encodable2718, eip7685::EMPTY_REQUESTS_HASH};
use alloy_evm::{eth::EthEvmContext, EthEvm, Evm};
use alloy_op_evm::OpEvm;
use alloy_primitives::{Bloom, BloomInput, Log, B256};
use anvil_core::eth::{
    block::{Block, BlockInfo, PartialHeader},
    transaction::{
        DepositReceipt, PendingTransaction, TransactionInfo, TypedReceipt, TypedTransaction,
    },
    trie,
};
use foundry_evm::{backend::DatabaseError, traces::CallTraceNode, Env};
use foundry_evm_core::{either_evm::EitherEvm, evm::FoundryPrecompiles};
use op_revm::{
    transaction::deposit::DEPOSIT_TRANSACTION_TYPE, L1BlockInfo, OpContext, OpTransaction,
    OpTransactionError,
};
use revm::{
    context::{Block as RevmBlock, BlockEnv, CfgEnv, Evm as RevmEvm, JournalTr},
    context_interface::result::{EVMError, ExecutionResult, Output},
    database::WrapDatabaseRef,
    handler::instructions::EthInstructions,
    interpreter::InstructionResult,
    primitives::hardfork::SpecId,
    Database, DatabaseRef, Inspector, Journal,
};
use std::sync::Arc;

/// Represents an executed transaction (transacted on the DB)
#[derive(Debug)]
pub struct ExecutedTransaction {
    transaction: Arc<PoolTransaction>,
    exit_reason: InstructionResult,
    out: Option<Output>,
    gas_used: u64,
    logs: Vec<Log>,
    traces: Vec<CallTraceNode>,
    nonce: u64,
}

// == impl ExecutedTransaction ==

impl ExecutedTransaction {
    /// Creates the receipt for the transaction
    fn create_receipt(&self, cumulative_gas_used: &mut u64) -> TypedReceipt {
        let logs = self.logs.clone();
        *cumulative_gas_used = cumulative_gas_used.saturating_add(self.gas_used);

        // successful return see [Return]
        let status_code = u8::from(self.exit_reason as u8 <= InstructionResult::SelfDestruct as u8);
        let receipt_with_bloom: ReceiptWithBloom = Receipt {
            status: (status_code == 1).into(),
            cumulative_gas_used: *cumulative_gas_used,
            logs,
        }
        .into();

        match &self.transaction.pending_transaction.transaction.transaction {
            TypedTransaction::Legacy(_) => TypedReceipt::Legacy(receipt_with_bloom),
            TypedTransaction::EIP2930(_) => TypedReceipt::EIP2930(receipt_with_bloom),
            TypedTransaction::EIP1559(_) => TypedReceipt::EIP1559(receipt_with_bloom),
            TypedTransaction::EIP4844(_) => TypedReceipt::EIP4844(receipt_with_bloom),
            TypedTransaction::EIP7702(_) => TypedReceipt::EIP7702(receipt_with_bloom),
            TypedTransaction::Deposit(tx) => TypedReceipt::Deposit(DepositReceipt {
                inner: receipt_with_bloom,
                deposit_nonce: Some(tx.nonce),
                deposit_receipt_version: Some(1),
            }),
        }
    }
}

/// Represents the outcome of mining a new block
#[derive(Clone, Debug)]
pub struct ExecutedTransactions {
    /// The block created after executing the `included` transactions
    pub block: BlockInfo,
    /// All transactions included in the block
    pub included: Vec<Arc<PoolTransaction>>,
    /// All transactions that were invalid at the point of their execution and were not included in
    /// the block
    pub invalid: Vec<Arc<PoolTransaction>>,
}

/// An executor for a series of transactions
pub struct TransactionExecutor<'a, Db: ?Sized, V: TransactionValidator> {
    /// where to insert the transactions
    pub db: &'a mut Db,
    /// type used to validate before inclusion
    pub validator: &'a V,
    /// all pending transactions
    pub pending: std::vec::IntoIter<Arc<PoolTransaction>>,
    pub block_env: BlockEnv,
    /// The configuration environment and spec id
    pub cfg_env: CfgEnv,
    pub parent_hash: B256,
    /// Cumulative gas used by all executed transactions
    pub gas_used: u64,
    /// Cumulative blob gas used by all executed transactions
    pub blob_gas_used: u64,
    pub enable_steps_tracing: bool,
    pub odyssey: bool,
    pub print_logs: bool,
    pub print_traces: bool,
    /// Precompiles to inject to the EVM.
    pub precompile_factory: Option<Arc<dyn PrecompileFactory>>,
}

impl<DB: Db + ?Sized, V: TransactionValidator> TransactionExecutor<'_, DB, V> {
    /// Executes all transactions and puts them in a new block with the provided `timestamp`
    pub fn execute(mut self) -> ExecutedTransactions {
        let mut transactions = Vec::new();
        let mut transaction_infos = Vec::new();
        let mut receipts = Vec::new();
        let mut bloom = Bloom::default();
        let mut cumulative_gas_used = 0u64;
        let mut invalid = Vec::new();
        let mut included = Vec::new();
        let gas_limit = self.block_env.gas_limit;
        let parent_hash = self.parent_hash;
        let block_number = self.block_env.number;
        let difficulty = self.block_env.difficulty;
        let beneficiary = self.block_env.beneficiary;
        let timestamp = self.block_env.timestamp;
        let base_fee = if self.cfg_env.spec.is_enabled_in(SpecId::LONDON) {
            Some(self.block_env.basefee)
        } else {
            None
        };

        let is_shanghai = self.cfg_env.spec >= SpecId::SHANGHAI;
        let is_cancun = self.cfg_env.spec >= SpecId::CANCUN;
        let is_prague = self.cfg_env.spec >= SpecId::PRAGUE;
        let excess_blob_gas = if is_cancun { self.block_env.blob_excess_gas() } else { None };
        let mut cumulative_blob_gas_used = if is_cancun { Some(0u64) } else { None };

        for tx in self.into_iter() {
            let tx = match tx {
                TransactionExecutionOutcome::Executed(tx) => {
                    included.push(tx.transaction.clone());
                    tx
                }
                TransactionExecutionOutcome::Exhausted(tx) => {
                    trace!(target: "backend",  tx_gas_limit = %tx.pending_transaction.transaction.gas_limit(), ?tx,  "block gas limit exhausting, skipping transaction");
                    continue
                }
                TransactionExecutionOutcome::BlobGasExhausted(tx) => {
                    trace!(target: "backend",  blob_gas = %tx.pending_transaction.transaction.blob_gas().unwrap_or_default(), ?tx,  "block blob gas limit exhausting, skipping transaction");
                    continue
                }
                TransactionExecutionOutcome::Invalid(tx, _) => {
                    trace!(target: "backend", ?tx,  "skipping invalid transaction");
                    invalid.push(tx);
                    continue
                }
                TransactionExecutionOutcome::DatabaseError(_, err) => {
                    // Note: this is only possible in forking mode, if for example a rpc request
                    // failed
                    trace!(target: "backend", ?err,  "Failed to execute transaction due to database error");
                    continue
                }
            };
            if is_cancun {
                let tx_blob_gas = tx
                    .transaction
                    .pending_transaction
                    .transaction
                    .transaction
                    .blob_gas()
                    .unwrap_or(0);
                cumulative_blob_gas_used =
                    Some(cumulative_blob_gas_used.unwrap_or(0u64).saturating_add(tx_blob_gas));
            }
            let receipt = tx.create_receipt(&mut cumulative_gas_used);

            let ExecutedTransaction { transaction, logs, out, traces, exit_reason: exit, .. } = tx;
            build_logs_bloom(logs.clone(), &mut bloom);

            let contract_address = out.as_ref().and_then(|out| {
                if let Output::Create(_, contract_address) = out {
                    trace!(target: "backend", "New contract deployed: at {:?}", contract_address);
                    *contract_address
                } else {
                    None
                }
            });

            let transaction_index = transaction_infos.len() as u64;
            let info = TransactionInfo {
                transaction_hash: transaction.hash(),
                transaction_index,
                from: *transaction.pending_transaction.sender(),
                to: transaction.pending_transaction.transaction.to(),
                contract_address,
                traces,
                exit,
                out: out.map(Output::into_data),
                nonce: tx.nonce,
                gas_used: tx.gas_used,
            };

            transaction_infos.push(info);
            receipts.push(receipt);
            transactions.push(transaction.pending_transaction.transaction.clone());
        }

        let receipts_root =
            trie::ordered_trie_root(receipts.iter().map(Encodable2718::encoded_2718));

        let partial_header = PartialHeader {
            parent_hash,
            beneficiary,
            state_root: self.db.maybe_state_root().unwrap_or_default(),
            receipts_root,
            logs_bloom: bloom,
            difficulty,
            number: block_number,
            gas_limit,
            gas_used: cumulative_gas_used,
            timestamp,
            extra_data: Default::default(),
            mix_hash: Default::default(),
            nonce: Default::default(),
            base_fee,
            parent_beacon_block_root: is_cancun.then_some(Default::default()),
            blob_gas_used: cumulative_blob_gas_used,
            excess_blob_gas,
            withdrawals_root: is_shanghai.then_some(EMPTY_WITHDRAWALS),
            requests_hash: is_prague.then_some(EMPTY_REQUESTS_HASH),
        };

        let block = Block::new(partial_header, transactions.clone());
        let block = BlockInfo { block, transactions: transaction_infos, receipts };
        ExecutedTransactions { block, included, invalid }
    }

    fn env_for(&self, tx: &PendingTransaction) -> Env {
        let (tx_env, maybe_deposit) = tx.to_revm_tx_env();

        let mut env = Env::from(self.cfg_env.clone(), self.block_env.clone(), tx_env);
        if env.tx.tx_type == DEPOSIT_TRANSACTION_TYPE {
            env = env.with_deposit(maybe_deposit.unwrap());
        }

        env
    }
}

/// Represents the result of a single transaction execution attempt
#[derive(Debug)]
pub enum TransactionExecutionOutcome {
    /// Transaction successfully executed
    Executed(ExecutedTransaction),
    /// Invalid transaction not executed
    Invalid(Arc<PoolTransaction>, InvalidTransactionError),
    /// Execution skipped because could exceed gas limit
    Exhausted(Arc<PoolTransaction>),
    /// Execution skipped because it exceeded the blob gas limit
    BlobGasExhausted(Arc<PoolTransaction>),
    /// When an error occurred during execution
    DatabaseError(Arc<PoolTransaction>, DatabaseError),
}

impl<DB: Db + ?Sized, V: TransactionValidator> Iterator for &mut TransactionExecutor<'_, DB, V> {
    type Item = TransactionExecutionOutcome;

    fn next(&mut self) -> Option<Self::Item> {
        let transaction = self.pending.next()?;
        let sender = *transaction.pending_transaction.sender();
        let account = match self.db.basic(sender).map(|acc| acc.unwrap_or_default()) {
            Ok(account) => account,
            Err(err) => return Some(TransactionExecutionOutcome::DatabaseError(transaction, err)),
        };
        let env = self.env_for(&transaction.pending_transaction);

        // check that we comply with the block's gas limit, if not disabled
        let max_gas = self.gas_used.saturating_add(env.tx.gas_limit);
        if !env.evm_env.cfg_env.disable_block_gas_limit && max_gas > env.evm_env.block_env.gas_limit
        {
            return Some(TransactionExecutionOutcome::Exhausted(transaction))
        }

        // check that we comply with the block's blob gas limit
        let max_blob_gas = self.blob_gas_used.saturating_add(
            transaction.pending_transaction.transaction.transaction.blob_gas().unwrap_or(0),
        );
        if max_blob_gas > alloy_eips::eip4844::MAX_DATA_GAS_PER_BLOCK {
            return Some(TransactionExecutionOutcome::BlobGasExhausted(transaction))
        }

        // validate before executing
        if let Err(err) = self.validator.validate_pool_transaction_for(
            &transaction.pending_transaction,
            &account,
            &env,
        ) {
            warn!(target: "backend", "Skipping invalid tx execution [{:?}] {}", transaction.hash(), err);
            return Some(TransactionExecutionOutcome::Invalid(transaction, err))
        }

        let nonce = account.nonce;

        let mut inspector = AnvilInspector::default().with_tracing();
        if self.enable_steps_tracing {
            inspector = inspector.with_steps_tracing();
        }
        if self.print_logs {
            inspector = inspector.with_log_collector();
        }
        if self.print_traces {
            inspector = inspector.with_trace_printer();
        }

        let mut evm = evm_with_inspector(
            &mut *self.db,
            &env,
            &mut inspector,
            transaction.tx_type() == DEPOSIT_TRANSACTION_TYPE,
        );

        let tx_commit = if transaction.tx_type() == DEPOSIT_TRANSACTION_TYPE {
            // Unwrap is safe. This should always be set if the transaction is a deposit
            evm.transact_deposit_commit(env.tx, env.deposit.unwrap())
        } else {
            evm.transact_commit(env.tx)
        };
        trace!(target: "backend", "[{:?}] executing", transaction.hash());
        let exec_result = match tx_commit {
            Ok(exec_result) => exec_result,
            Err(err) => {
                warn!(target: "backend", "[{:?}] failed to execute: {:?}", transaction.hash(), err);
                match err {
                    EVMError::Database(err) => {
                        return Some(TransactionExecutionOutcome::DatabaseError(transaction, err))
                    }
                    EVMError::Transaction(err) => {
                        let err = match err {
                            OpTransactionError::Base(err) => err.into(),
                            OpTransactionError::HaltedDepositPostRegolith |
                            OpTransactionError::DepositSystemTxPostRegolith => {
                                InvalidTransactionError::DepositTxErrorPostRegolith
                            }
                        };
                        return Some(TransactionExecutionOutcome::Invalid(transaction, err))
                    }
                    e => panic!("failed to execute transaction: {e}"),
                }
            }
        };

        if self.print_traces {
            inspector.print_traces();
        }
        inspector.print_logs();

        let (exit_reason, gas_used, out, logs) = match exec_result {
            ExecutionResult::Success { reason, gas_used, logs, output, .. } => {
                (reason.into(), gas_used, Some(output), Some(logs))
            }
            ExecutionResult::Revert { gas_used, output } => {
                (InstructionResult::Revert, gas_used, Some(Output::Call(output)), None)
            }
            ExecutionResult::Halt { reason, gas_used } => {
                (op_haltreason_to_instruction_result(reason), gas_used, None, None)
            }
        };

        if exit_reason == InstructionResult::OutOfGas {
            // this currently useful for debugging estimations
            warn!(target: "backend", "[{:?}] executed with out of gas", transaction.hash())
        }

        trace!(target: "backend", ?exit_reason, ?gas_used, "[{:?}] executed with out={:?}", transaction.hash(), out);

        // Track the total gas used for total gas per block checks
        self.gas_used = self.gas_used.saturating_add(gas_used);

        // Track the total blob gas used for total blob gas per blob checks
        if let Some(blob_gas) = transaction.pending_transaction.transaction.transaction.blob_gas() {
            self.blob_gas_used = self.blob_gas_used.saturating_add(blob_gas);
        }

        trace!(target: "backend::executor", "transacted [{:?}], result: {:?} gas {}", transaction.hash(), exit_reason, gas_used);

        let tx = ExecutedTransaction {
            transaction,
            exit_reason,
            out,
            gas_used,
            logs: logs.unwrap_or_default(),
            traces: inspector.tracer.map(|t| t.into_traces().into_nodes()).unwrap_or_default(),
            nonce,
        };

        Some(TransactionExecutionOutcome::Executed(tx))
    }
}

/// Inserts all logs into the bloom
fn build_logs_bloom(logs: Vec<Log>, bloom: &mut Bloom) {
    for log in logs {
        bloom.accrue(BloomInput::Raw(&log.address[..]));
        for topic in log.topics() {
            bloom.accrue(BloomInput::Raw(&topic[..]));
        }
    }
}

/// Creates a database with given database and inspector, optionally enabling odyssey features.
pub fn evm_with_inspector<DB, I>(
    db: DB,
    env: &Env,
    inspector: I,
    is_optimism: bool,
) -> EitherEvm<DB, I, FoundryPrecompiles>
where
    DB: Database<Error = DatabaseError>,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>>,
{
    if is_optimism {
        let op_context = OpContext {
            journaled_state: {
                let mut journal = Journal::new(db);
                // Converting SpecId into OpSpecId
                journal.set_spec_id(env.evm_env.cfg_env.spec);
                journal
            },
            block: env.evm_env.block_env.clone(),
            cfg: env.evm_env.cfg_env.clone().with_spec(op_revm::OpSpecId::BEDROCK),
            tx: OpTransaction::new(env.tx.clone()),
            chain: L1BlockInfo::default(),
            error: Ok(()),
        };

        let evm = op_revm::OpEvm(RevmEvm::new_with_inspector(
            op_context,
            inspector,
            EthInstructions::default(),
            FoundryPrecompiles::default(),
        ));

        let op = OpEvm::new(evm, true);

        EitherEvm::Op(op)
    } else {
        let evm_context = EthEvmContext {
            journaled_state: {
                let mut journal = Journal::new(db);
                journal.set_spec_id(env.evm_env.cfg_env.spec);
                journal
            },
            block: env.evm_env.block_env.clone(),
            cfg: env.evm_env.cfg_env.clone(),
            tx: env.tx.clone(),
            chain: (),
            error: Ok(()),
        };

        let evm = RevmEvm::new_with_inspector(
            evm_context,
            inspector,
            EthInstructions::default(),
            FoundryPrecompiles::new(),
        );

        let eth = EthEvm::new(evm, true);

        EitherEvm::Eth(eth)
    }
}

/// Creates a new EVM with the given inspector and wraps the database in a `WrapDatabaseRef`.
pub fn evm_with_inspector_ref<'db, DB, I>(
    db: &'db DB,
    env: &Env,
    inspector: &'db mut I,
    is_optimism: bool,
) -> EitherEvm<WrapDatabaseRef<&'db DB>, &'db mut I, FoundryPrecompiles>
where
    DB: DatabaseRef<Error = DatabaseError> + 'db + ?Sized,
    I: Inspector<EthEvmContext<WrapDatabaseRef<&'db DB>>>
        + Inspector<OpContext<WrapDatabaseRef<&'db DB>>>,
    WrapDatabaseRef<&'db DB>: Database<Error = DatabaseError>,
{
    evm_with_inspector(WrapDatabaseRef(db), env, inspector, is_optimism)
}
