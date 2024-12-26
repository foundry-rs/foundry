use crate::{Cheatcode, CheatsCtxt, Result, Vm::*};
use alloy_primitives::TxKind;
use alloy_sol_types::{sol, SolValue};
use assertion_executor::db::fork_db::ForkDb;
use assertion_executor::{store::AssertionStore, AssertionExecutorBuilder};
use foundry_evm_core::backend::{DatabaseError, DatabaseExt};
use revm::primitives::{AccountInfo, Address, Bytecode, TxEnv, B256, U256};
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

sol ! {
    struct SimpleTransaction {
        address from;
        address to;
        uint256 value;
        bytes data;
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

        let decoded_tx= SimpleTransaction::abi_decode(&tx, true)?;
        let tx = TxEnv {
            caller: decoded_tx.from,
            gas_limit: 1000000,
            gas_price: U256::from(0),
            gas_priority_fee: None,
            transact_to: TxKind::Call(decoded_tx.to),
            value: decoded_tx.value,
            data: decoded_tx.data,
            chain_id: Some(ccx.ecx.env.cfg.chain_id),
            nonce:  None,
            .. Default::default()
        };
        
        
        
        // TODO: Add a function in assertion executor which returns:
        // 1. Which assertions were touched by which assertions
        // 2. Which assertion invalidated which transaction

        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                // Store assertions
                let _ = store.writer().write(block_number, assertions).await.expect("Failed to store assertions");
                let mut assertion_executor = AssertionExecutorBuilder::new(db.clone(), store.reader()).build();
                assertion_executor
                    .validate_transaction(block, tx, &mut ForkDb::new(db))
                    .await
            })
            .expect("Assertion execution failed")
            .is_some();
        Ok(result.abi_encode())
    }
}

