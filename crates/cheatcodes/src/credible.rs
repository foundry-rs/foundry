use crate::{Cheatcode, CheatsCtxt, Result, Vm::*};
use alloy_consensus::{Transaction, TxEnvelope};
use alloy_primitives::TxKind;
use alloy_sol_types::SolValue;
use assertion_executor::{store::AssertionStore, AssertionExecutorBuilder};
use foundry_common::TransactionMaybeSigned;
use foundry_evm_core::backend::{DatabaseError, DatabaseExt};
use revm::primitives::{AccountInfo, Address, Bytecode, OptimismFields, TxEnv, B256, U256};
use revm::DatabaseRef;
use std::sync::{ Arc, Mutex};
use tokio;

/// Wrapper around DatabaseExt to make it thread-safe
#[derive(Clone)]
struct ThreadSafeDb<'a> {
    db: Arc<Mutex<&'a mut dyn DatabaseExt>>,
}

impl std::fmt::Debug for ThreadSafeDb<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ThreadSafeDb")
    }
}

/// Separate implementation block for constructor and helper methods
impl<'a> ThreadSafeDb<'a> {
    /// Creates a new thread-safe database wrapper
    pub fn new(db: &'a mut dyn DatabaseExt) -> Self {
        Self { db: Arc::new(Mutex::new(db)) }
    }
}

/// Keep DatabaseRef implementation separate
impl<'a> DatabaseRef for ThreadSafeDb<'a> {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.db.lock().unwrap().basic(address)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.db.lock().unwrap().code_by_hash(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.db.lock().unwrap().storage(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.db.lock().unwrap().block_hash(number)
    }
}

impl Cheatcode for assertionExCall {
    fn apply_stateful(&self, ccx: &mut CheatsCtxt) -> Result {
        let Self { tx, assertionAdopter, assertionBytecode } = self.clone();
        
        // Setup assertion store and database
        let store = AssertionStore::new(100);
        let db = ThreadSafeDb::new(ccx.ecx.db);
        
        // Prepare assertions data
        let block_number = ccx.ecx.env.block.number;
        let block = ccx.ecx.env.block.clone();
        let assertions = vec![(
            assertionAdopter,
            assertionBytecode
                .iter()
                .map(|bytes| Bytecode::LegacyRaw(bytes.to_owned().into()))
                .collect(),
        )];
        
        // Store assertions
        store.writer().write(block_number, assertions);
        
        // Execute assertions
        let assertion_executor = AssertionExecutorBuilder::new(db, store.reader()).build();
        
        // TODO: Add a function in assertion executor which returns:
        // 1. Which assertions were touched by which assertions
        // 2. Which assertion invalidated which transaction

        let transactions  = ccx.state.broadcastable_transactions;
        let results = transactions.into_iter().map(|btx| {
            let tx = btx.transaction;
            let cloned_db = db.clone();
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async move {
                    assertion_executor
                        .validate_transaction(block, envelope_to_environment(&tx), cloned_db)
                        .await
                })
                .expect("Assertion execution failed")
                .is_some()
            }).collect();
        Ok(results.abi_encode())
    }
}

pub fn envelope_to_environment(tx: &TransactionMaybeSigned) -> TxEnv {
    match tx {
        TransactionMaybeSigned::Signed{ tx, from} => 
            TxEnv {
            caller: *from,
            gas_limit: tx.gas_limit(),
            gas_price: U256::from(0),
            gas_priority_fee: None,
            transact_to: if tx.is_create() { TxKind::Create } else { TxKind::Call(tx.to().unwrap()) },
            value: tx.value(),
            data: tx.input().clone(),
            chain_id: tx.chain_id(),
            nonce: Some(tx.nonce()),
            .. Default::default()
        },
        _ => todo!(),
    }
}
