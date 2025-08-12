use crate::{
    PrecompileFactory,
    eth::{
        backend::{
            db::Db, env::Env, mem::op_haltreason_to_instruction_result,
            validate::TransactionValidator,
        },
        error::InvalidTransactionError,
        pool::transactions::PoolTransaction,
    },
    inject_precompiles,
    mem::inspector::AnvilInspector,
};
use alloy_consensus::{
    Receipt, ReceiptWithBloom, constants::EMPTY_WITHDRAWALS, proofs::calculate_receipt_root,
};
use alloy_eips::{eip7685::EMPTY_REQUESTS_HASH, eip7840::BlobParams};
use alloy_evm::{EthEvm, Evm, eth::EthEvmContext, precompiles::PrecompilesMap};
use alloy_op_evm::OpEvm;
use alloy_primitives::{B256, Bloom, BloomInput, Log};
use anvil_core::eth::{
    block::{Block, BlockInfo, PartialHeader},
    transaction::{
        DepositReceipt, PendingTransaction, TransactionInfo, TypedReceipt, TypedTransaction,
    },
};
use foundry_evm::{
    backend::DatabaseError,
    traces::{CallTraceDecoder, CallTraceNode},
};
use foundry_evm_core::either_evm::EitherEvm;
use op_revm::{L1BlockInfo, OpContext, precompiles::OpPrecompiles};
use revm::{
    Database, DatabaseRef, Inspector, Journal,
    context::{Block as RevmBlock, BlockEnv, CfgEnv, Evm as RevmEvm, JournalTr, LocalContext},
    context_interface::result::{EVMError, ExecutionResult, Output},
    database::WrapDatabaseRef,
    handler::{EthPrecompiles, instructions::EthInstructions},
    interpreter::InstructionResult,
    precompile::{
        PrecompileSpecId, Precompiles,
        secp256r1::{P256VERIFY, P256VERIFY_BASE_GAS_FEE},
    },
    primitives::hardfork::SpecId,
};
use std::{fmt::Debug, sync::Arc};

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
            TypedTransaction::Deposit(_tx) => TypedReceipt::Deposit(DepositReceipt {
                inner: receipt_with_bloom,
                deposit_nonce: Some(0),
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
    pub optimism: bool,
    pub print_logs: bool,
    pub print_traces: bool,
    /// Recorder used for decoding traces, used together with print_traces
    pub call_trace_decoder: Arc<CallTraceDecoder>,
    /// Precompiles to inject to the EVM.
    pub precompile_factory: Option<Arc<dyn PrecompileFactory>>,
    pub blob_params: BlobParams,
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
        let mix_hash = self.block_env.prevrandao;
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
                    continue;
                }
                TransactionExecutionOutcome::BlobGasExhausted(tx) => {
                    trace!(target: "backend",  blob_gas = %tx.pending_transaction.transaction.blob_gas().unwrap_or_default(), ?tx,  "block blob gas limit exhausting, skipping transaction");
                    continue;
                }
                TransactionExecutionOutcome::Invalid(tx, _) => {
                    trace!(target: "backend", ?tx,  "skipping invalid transaction");
                    invalid.push(tx);
                    continue;
                }
                TransactionExecutionOutcome::DatabaseError(_, err) => {
                    // Note: this is only possible in forking mode, if for example a rpc request
                    // failed
                    trace!(target: "backend", ?err,  "Failed to execute transaction due to database error");
                    continue;
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

        let receipts_root = calculate_receipt_root(&receipts);

        let partial_header = PartialHeader {
            parent_hash,
            beneficiary,
            state_root: self.db.maybe_state_root().unwrap_or_default(),
            receipts_root,
            logs_bloom: bloom,
            difficulty,
            number: block_number.saturating_to(),
            gas_limit,
            gas_used: cumulative_gas_used,
            timestamp: timestamp.saturating_to(),
            extra_data: Default::default(),
            mix_hash: mix_hash.unwrap_or_default(),
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
        let mut tx_env = tx.to_revm_tx_env();

        if self.optimism {
            tx_env.enveloped_tx = Some(alloy_rlp::encode(&tx.transaction.transaction).into());
        }

        Env::new(self.cfg_env.clone(), self.block_env.clone(), tx_env, self.optimism)
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
        let max_gas = self.gas_used.saturating_add(env.tx.base.gas_limit);
        if !env.evm_env.cfg_env.disable_block_gas_limit && max_gas > env.evm_env.block_env.gas_limit
        {
            return Some(TransactionExecutionOutcome::Exhausted(transaction));
        }

        // check that we comply with the block's blob gas limit
        let max_blob_gas = self.blob_gas_used.saturating_add(
            transaction.pending_transaction.transaction.transaction.blob_gas().unwrap_or(0),
        );
        if max_blob_gas > self.blob_params.max_blob_gas_per_block() {
            return Some(TransactionExecutionOutcome::BlobGasExhausted(transaction));
        }

        // validate before executing
        if let Err(err) = self.validator.validate_pool_transaction_for(
            &transaction.pending_transaction,
            &account,
            &env,
        ) {
            warn!(target: "backend", "Skipping invalid tx execution [{:?}] {}", transaction.hash(), err);
            return Some(TransactionExecutionOutcome::Invalid(transaction, err));
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

        let exec_result = {
            let mut evm = new_evm_with_inspector(&mut *self.db, &env, &mut inspector);

            if self.odyssey {
                inject_precompiles(&mut evm, vec![(P256VERIFY, P256VERIFY_BASE_GAS_FEE)]);
            }

            if let Some(factory) = &self.precompile_factory {
                inject_precompiles(&mut evm, factory.precompiles());
            }

            trace!(target: "backend", "[{:?}] executing", transaction.hash());
            // transact and commit the transaction
            match evm.transact_commit(env.tx) {
                Ok(exec_result) => exec_result,
                Err(err) => {
                    warn!(target: "backend", "[{:?}] failed to execute: {:?}", transaction.hash(), err);
                    match err {
                        EVMError::Database(err) => {
                            return Some(TransactionExecutionOutcome::DatabaseError(
                                transaction,
                                err,
                            ));
                        }
                        EVMError::Transaction(err) => {
                            return Some(TransactionExecutionOutcome::Invalid(
                                transaction,
                                err.into(),
                            ));
                        }
                        // This will correspond to prevrandao not set, and it should never happen.
                        // If it does, it's a bug.
                        e => panic!("failed to execute transaction: {e}"),
                    }
                }
            }
        };

        if self.print_traces {
            inspector.print_traces(self.call_trace_decoder.clone());
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
pub fn new_evm_with_inspector<DB, I>(
    db: DB,
    env: &Env,
    inspector: I,
) -> EitherEvm<DB, I, PrecompilesMap>
where
    DB: Database<Error = DatabaseError> + Debug,
    I: Inspector<EthEvmContext<DB>> + Inspector<OpContext<DB>>,
{
    if env.is_optimism {
        let op_cfg = env.evm_env.cfg_env.clone().with_spec(op_revm::OpSpecId::ISTHMUS);
        let op_context = OpContext {
            journaled_state: {
                let mut journal = Journal::new(db);
                // Converting SpecId into OpSpecId
                journal.set_spec_id(env.evm_env.cfg_env.spec);
                journal
            },
            block: env.evm_env.block_env.clone(),
            cfg: op_cfg.clone(),
            tx: env.tx.clone(),
            chain: L1BlockInfo::default(),
            local: LocalContext::default(),
            error: Ok(()),
        };

        let op_precompiles = OpPrecompiles::new_with_spec(op_cfg.spec).precompiles();
        let op_evm = op_revm::OpEvm(RevmEvm::new_with_inspector(
            op_context,
            inspector,
            EthInstructions::default(),
            PrecompilesMap::from_static(op_precompiles),
        ));

        let op = OpEvm::new(op_evm, true);

        EitherEvm::Op(op)
    } else {
        let spec = env.evm_env.cfg_env.spec;
        let eth_context = EthEvmContext {
            journaled_state: {
                let mut journal = Journal::new(db);
                journal.set_spec_id(spec);
                journal
            },
            block: env.evm_env.block_env.clone(),
            cfg: env.evm_env.cfg_env.clone(),
            tx: env.tx.base.clone(),
            chain: (),
            local: LocalContext::default(),
            error: Ok(()),
        };

        let eth_precompiles = EthPrecompiles {
            precompiles: Precompiles::new(PrecompileSpecId::from_spec_id(spec)),
            spec,
        }
        .precompiles;
        let eth_evm = RevmEvm::new_with_inspector(
            eth_context,
            inspector,
            EthInstructions::default(),
            PrecompilesMap::from_static(eth_precompiles),
        );

        let eth = EthEvm::new(eth_evm, true);

        EitherEvm::Eth(eth)
    }
}

/// Creates a new EVM with the given inspector and wraps the database in a `WrapDatabaseRef`.
pub fn new_evm_with_inspector_ref<'db, DB, I>(
    db: &'db DB,
    env: &Env,
    inspector: &'db mut I,
) -> EitherEvm<WrapDatabaseRef<&'db DB>, &'db mut I, PrecompilesMap>
where
    DB: DatabaseRef<Error = DatabaseError> + Debug + 'db + ?Sized,
    I: Inspector<EthEvmContext<WrapDatabaseRef<&'db DB>>>
        + Inspector<OpContext<WrapDatabaseRef<&'db DB>>>,
    WrapDatabaseRef<&'db DB>: Database<Error = DatabaseError>,
{
    new_evm_with_inspector(WrapDatabaseRef(db), env, inspector)
}
