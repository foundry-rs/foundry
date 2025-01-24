use crate::{Cheatcode, CheatsCtxt, Result, Vm::*};
use alloy_primitives::TxKind;
use alloy_sol_types::SolValue;
use assertion_executor::{db::fork_db::ForkDb, store::MockStore, AssertionExecutorBuilder};
use foundry_evm_core::backend::{DatabaseError, DatabaseExt};
use revm::{
    primitives::{AccountInfo, Address, Bytecode, TxEnv, B256, U256},
    DatabaseCommit, DatabaseRef,
};
use std::sync::{Arc, Mutex};
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
        let Self { tx, assertionAdopter: assertion_adopter, assertions } = self;

        let spec_id = ccx.ecx.spec_id();
        let block = ccx.ecx.env.block.clone();
        let state = ccx.ecx.journaled_state.state.clone();
        let chain_id = ccx.ecx.env.cfg.chain_id;

        // Setup assertion database
        let db = ThreadSafeDb::new(ccx.ecx.db);

        // Prepare assertion store
        let assertions_bytecode =
            assertions.iter().map(|bytes| Bytecode::LegacyRaw(bytes.to_vec().into())).collect();

        let mut store = MockStore::new(spec_id, chain_id.clone());
        store.insert(*assertion_adopter, assertions_bytecode).expect("Failed to store assertions");

        let decoded_tx = AssertionExTransaction::abi_decode(&tx, true)?;

        let tx_env = TxEnv {
            caller: decoded_tx.from,
            gas_limit: ccx.ecx.env.block.gas_limit.try_into().unwrap_or(u64::MAX),
            transact_to: TxKind::Call(decoded_tx.to),
            value: decoded_tx.value,
            data: decoded_tx.data,
            chain_id: Some(chain_id),
            ..Default::default()
        };

        let rt = tokio::runtime::Runtime::new().unwrap();

        // Execute the future, blocking the current thread until completion
        match rt.block_on(async move {
            let cancellation_token = tokio_util::sync::CancellationToken::new();

            let (reader, handle) = store.cancellable_reader(cancellation_token.clone());

            let mut assertion_executor =
                AssertionExecutorBuilder::new(db, reader).with_chain_id(chain_id).build();

            // Commit current journal state so that it is available for assertions and
            // triggering tx
            let mut fork_db = ForkDb::new(assertion_executor.db.clone());
            fork_db.commit(state);

            // Store assertions
            let result = assertion_executor.validate_transaction(block, tx_env, &mut fork_db).await;

            cancellation_token.cancel();

            let _ = handle.await;

            result
        }) {
            Ok(val) => Ok(val.is_some().abi_encode()),
            Err(e) => Result::Err(format!("Error in assertionExCall: {:#?}", e).into()),
        }
    }
}
