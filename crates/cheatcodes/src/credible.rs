use crate::{Cheatcode, CheatsCtxt, Result, Vm::*};
use alloy_sol_types::SolValue;
use assertion_executor::{store::AssertionStore, AssertionExecutorBuilder};
use foundry_evm_core::backend::{DatabaseError, DatabaseExt};
use revm::primitives::{Address, B256, U256, Bytecode, AccountInfo};
use revm::DatabaseRef;
use std::sync::{Arc, Mutex};

struct ThreadSafeDb<'a> {
    db: Mutex<&'a mut dyn DatabaseExt>
}

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
        let Self{tx, assertionAdopter, assertionBytecode} = self.clone();
        let store = AssertionStore::new(100, 100);
        let block_number = ccx.ecx.env.block.number;
        let db = ThreadSafeDb { db: Mutex::new(ccx.ecx.db) };
        let assertions = vec![(assertionAdopter, assertionBytecode.iter().map(|bytes| Bytecode::LegacyRaw(bytes.to_owned())).collect())];
        store.writer().write(block_number, assertions );
        let assertion_executor = AssertionExecutorBuilder::new(db, store.reader()).build();
        let result = assertion_executor.validate_transaction();
        Ok(true.abi_encode())
    }
}