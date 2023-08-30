use crate::{
    eth::{
        backend::{db::Db, validate::TransactionValidator},
        error::InvalidTransactionError,
        pool::transactions::PoolTransaction,
    },
    mem::inspector::Inspector,
};
use anvil_core::eth::{
    block::{Block, BlockInfo, Header, PartialHeader},
    receipt::{EIP1559Receipt, EIP2930Receipt, EIP658Receipt, Log, TypedReceipt},
    transaction::{PendingTransaction, TransactionInfo, TypedTransaction},
    trie,
};
use ethers::{
    abi::ethereum_types::BloomInput,
    types::{Bloom, H256, U256},
    utils::rlp,
};
use foundry_evm::{
    executor::backend::DatabaseError,
    revm,
    revm::{
        interpreter::InstructionResult,
        primitives::{BlockEnv, CfgEnv, EVMError, Env, ExecutionResult, Output, SpecId},
    },
    trace::{node::CallTraceNode, CallTraceArena},
    utils::{
        b160_to_h160, eval_to_instruction_result, h160_to_b160, halt_to_instruction_result,
        ru256_to_u256,
    },
};
use std::sync::Arc;
use tracing::{trace, warn};

/// Represents an executed transaction (transacted on the DB)
pub struct ExecutedTransaction {
    transaction: Arc<PoolTransaction>,
    exit_reason: InstructionResult,
    out: Option<Output>,
    gas_used: u64,
    logs: Vec<Log>,
    traces: Vec<CallTraceNode>,
}

// == impl ExecutedTransaction ==

impl ExecutedTransaction {
    /// Creates the receipt for the transaction
    fn create_receipt(&self) -> TypedReceipt {
        let used_gas: U256 = self.gas_used.into();
        let mut bloom = Bloom::default();
        logs_bloom(self.logs.clone(), &mut bloom);
        let logs = self.logs.clone();

        // successful return see [Return]
        let status_code = u8::from(self.exit_reason as u8 <= InstructionResult::SelfDestruct as u8);
        match &self.transaction.pending_transaction.transaction.transaction {
            TypedTransaction::Legacy(_) => TypedReceipt::Legacy(EIP658Receipt {
                status_code,
                gas_used: used_gas,
                logs_bloom: bloom,
                logs,
            }),
            TypedTransaction::EIP2930(_) => TypedReceipt::EIP2930(EIP2930Receipt {
                status_code,
                gas_used: used_gas,
                logs_bloom: bloom,
                logs,
            }),
            TypedTransaction::EIP1559(_) => TypedReceipt::EIP1559(EIP1559Receipt {
                status_code,
                gas_used: used_gas,
                logs_bloom: bloom,
                logs,
            }),
        }
    }
}

/// Represents the outcome of mining a new block
#[derive(Debug, Clone)]
pub struct ExecutedTransactions {
    /// The block created after executing the `included` transactions
    pub block: BlockInfo,
    /// All transactions included in the
    pub included: Vec<Arc<PoolTransaction>>,
    /// All transactions that were invalid at the point of their execution and were not included in
    /// the block
    pub invalid: Vec<Arc<PoolTransaction>>,
}

/// An executor for a series of transactions
pub struct TransactionExecutor<'a, Db: ?Sized, Validator: TransactionValidator> {
    /// where to insert the transactions
    pub db: &'a mut Db,
    /// type used to validate before inclusion
    pub validator: Validator,
    /// all pending transactions
    pub pending: std::vec::IntoIter<Arc<PoolTransaction>>,
    pub block_env: BlockEnv,
    pub cfg_env: CfgEnv,
    pub parent_hash: H256,
    /// Cumulative gas used by all executed transactions
    pub gas_used: U256,
    pub enable_steps_tracing: bool,
}

impl<'a, DB: Db + ?Sized, Validator: TransactionValidator> TransactionExecutor<'a, DB, Validator> {
    /// Executes all transactions and puts them in a new block with the provided `timestamp`
    pub fn execute(mut self) -> ExecutedTransactions {
        let mut transactions = Vec::new();
        let mut transaction_infos = Vec::new();
        let mut receipts = Vec::new();
        let mut bloom = Bloom::default();
        let mut cumulative_gas_used = U256::zero();
        let mut invalid = Vec::new();
        let mut included = Vec::new();
        let gas_limit = self.block_env.gas_limit;
        let parent_hash = self.parent_hash;
        let block_number = self.block_env.number;
        let difficulty = self.block_env.difficulty;
        let beneficiary = self.block_env.coinbase;
        let timestamp = ru256_to_u256(self.block_env.timestamp).as_u64();
        let base_fee = if (self.cfg_env.spec_id as u8) >= (SpecId::LONDON as u8) {
            Some(self.block_env.basefee)
        } else {
            None
        };

        for tx in self.into_iter() {
            let tx = match tx {
                TransactionExecutionOutcome::Executed(tx) => {
                    included.push(tx.transaction.clone());
                    tx
                }
                TransactionExecutionOutcome::Exhausted(_) => continue,
                TransactionExecutionOutcome::Invalid(tx, _) => {
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
            let receipt = tx.create_receipt();
            cumulative_gas_used = cumulative_gas_used.saturating_add(receipt.gas_used());
            let ExecutedTransaction { transaction, logs, out, traces, exit_reason: exit, .. } = tx;
            logs_bloom(logs.clone(), &mut bloom);

            let contract_address = if let Some(Output::Create(_, contract_address)) = out {
                trace!(target: "backend", "New contract deployed: at {:?}", contract_address);
                contract_address
            } else {
                None
            };

            let transaction_index = transaction_infos.len() as u32;
            let info = TransactionInfo {
                transaction_hash: *transaction.hash(),
                transaction_index,
                from: *transaction.pending_transaction.sender(),
                to: transaction.pending_transaction.transaction.to().copied(),
                contract_address: contract_address.map(b160_to_h160),
                logs,
                logs_bloom: *receipt.logs_bloom(),
                traces: CallTraceArena { arena: traces },
                exit,
                out: match out {
                    Some(Output::Call(b)) => Some(ethers::types::Bytes(b.0)),
                    Some(Output::Create(b, _)) => Some(ethers::types::Bytes(b.0)),
                    _ => None,
                },
            };

            transaction_infos.push(info);
            receipts.push(receipt);
            transactions.push(transaction.pending_transaction.transaction.clone());
        }

        let ommers: Vec<Header> = Vec::new();
        let receipts_root = trie::ordered_trie_root(receipts.iter().map(rlp::encode));

        let partial_header = PartialHeader {
            parent_hash,
            beneficiary: b160_to_h160(beneficiary),
            state_root: self.db.maybe_state_root().unwrap_or_default(),
            receipts_root,
            logs_bloom: bloom,
            difficulty: ru256_to_u256(difficulty),
            number: ru256_to_u256(block_number),
            gas_limit: ru256_to_u256(gas_limit),
            gas_used: cumulative_gas_used,
            timestamp,
            extra_data: Default::default(),
            mix_hash: Default::default(),
            nonce: Default::default(),
            base_fee: base_fee.map(|x| ru256_to_u256(x)),
        };

        let block = Block::new(partial_header, transactions.clone(), ommers);
        let block = BlockInfo { block, transactions: transaction_infos, receipts };
        ExecutedTransactions { block, included, invalid }
    }

    fn env_for(&self, tx: &PendingTransaction) -> Env {
        Env { cfg: self.cfg_env.clone(), block: self.block_env.clone(), tx: tx.to_revm_tx_env() }
    }
}

/// Represents the result of a single transaction execution attempt
pub enum TransactionExecutionOutcome {
    /// Transaction successfully executed
    Executed(ExecutedTransaction),
    /// Invalid transaction not executed
    Invalid(Arc<PoolTransaction>, InvalidTransactionError),
    /// Execution skipped because could exceed gas limit
    Exhausted(Arc<PoolTransaction>),
    /// When an error occurred during execution
    DatabaseError(Arc<PoolTransaction>, DatabaseError),
}

impl<'a, 'b, DB: Db + ?Sized, Validator: TransactionValidator> Iterator
    for &'b mut TransactionExecutor<'a, DB, Validator>
{
    type Item = TransactionExecutionOutcome;

    fn next(&mut self) -> Option<Self::Item> {
        let transaction = self.pending.next()?;
        let sender = *transaction.pending_transaction.sender();
        let account = match self.db.basic(h160_to_b160(sender)).map(|acc| acc.unwrap_or_default()) {
            Ok(account) => account,
            Err(err) => return Some(TransactionExecutionOutcome::DatabaseError(transaction, err)),
        };
        let env = self.env_for(&transaction.pending_transaction);
        // check that we comply with the block's gas limit
        let max_gas = self.gas_used.saturating_add(U256::from(env.tx.gas_limit));
        if max_gas > ru256_to_u256(env.block.gas_limit) {
            return Some(TransactionExecutionOutcome::Exhausted(transaction))
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

        let mut evm = revm::EVM::new();
        evm.env = env;
        evm.database(&mut self.db);

        // records all call and step traces
        let mut inspector = Inspector::default().with_tracing();
        if self.enable_steps_tracing {
            inspector = inspector.with_steps_tracing();
        }

        trace!(target: "backend", "[{:?}] executing", transaction.hash());
        // transact and commit the transaction
        let exec_result = match evm.inspect_commit(&mut inspector) {
            Ok(exec_result) => exec_result,
            Err(err) => {
                warn!(target: "backend", "[{:?}] failed to execute: {:?}", transaction.hash(), err);
                match err {
                    EVMError::Database(err) => {
                        return Some(TransactionExecutionOutcome::DatabaseError(transaction, err))
                    }
                    EVMError::Transaction(err) => {
                        return Some(TransactionExecutionOutcome::Invalid(transaction, err.into()))
                    }
                    // This will correspond to prevrandao not set, and it should never happen.
                    // If it does, it's a bug.
                    e => {
                        panic!("Failed to execute transaction. This is a bug.\n {:?}", e)
                    }
                }
            }
        };
        inspector.print_logs();

        let (exit_reason, gas_used, out, logs) = match exec_result {
            ExecutionResult::Success { reason, gas_used, logs, output, .. } => {
                (eval_to_instruction_result(reason), gas_used, Some(output), Some(logs))
            }
            ExecutionResult::Revert { gas_used, output } => {
                (InstructionResult::Revert, gas_used, Some(Output::Call(output)), None)
            }
            ExecutionResult::Halt { reason, gas_used } => {
                (halt_to_instruction_result(reason), gas_used, None, None)
            }
        };

        if exit_reason == InstructionResult::OutOfGas {
            // this currently useful for debugging estimations
            warn!(target: "backend", "[{:?}] executed with out of gas", transaction.hash())
        }

        trace!(target: "backend", ?exit_reason, ?gas_used, "[{:?}] executed with out={:?}", transaction.hash(), out);

        self.gas_used.saturating_add(U256::from(gas_used));

        trace!(target: "backend::executor", "transacted [{:?}], result: {:?} gas {}", transaction.hash(), exit_reason, gas_used);

        let tx = ExecutedTransaction {
            transaction,
            exit_reason,
            out,
            gas_used,
            logs: logs.unwrap_or_default().into_iter().map(Into::into).collect(),
            traces: inspector.tracer.unwrap_or_default().traces.arena,
        };

        Some(TransactionExecutionOutcome::Executed(tx))
    }
}

/// Inserts all logs into the bloom
fn logs_bloom(logs: Vec<Log>, bloom: &mut Bloom) {
    for log in logs {
        bloom.accrue(BloomInput::Raw(&log.address[..]));
        for topic in log.topics {
            bloom.accrue(BloomInput::Raw(&topic[..]));
        }
    }
}
