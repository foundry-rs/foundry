use crate::{Cheatcode, CheatcodesExecutor, CheatsCtxt, Result, Vm::*};
use alloy_primitives::TxKind;
use alloy_sol_types::{Revert, SolError, SolValue};
use assertion_executor::{db::fork_db::ForkDb, store::MockStore, ExecutorConfig};
use foundry_evm_core::backend::{DatabaseError, DatabaseExt};
use revm::{
    primitives::{AccountInfo, Address, Bytecode, ExecutionResult, TxEnv, B256, U256},
    DatabaseCommit, DatabaseRef,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
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
    fn apply_full(&self, ccx: &mut CheatsCtxt, executor: &mut dyn CheatcodesExecutor) -> Result {
        let Self {
            tx,
            assertionAdopter: assertion_adopter,
            assertionContract,
            assertionContractLabel,
        } = self;

        let spec_id = ccx.ecx.spec_id();
        let block = ccx.ecx.env.block.clone();
        let state = ccx.ecx.journaled_state.state.clone();
        let chain_id = ccx.ecx.env.cfg.chain_id;

        // Setup assertion database
        let db = ThreadSafeDb::new(ccx.ecx.db);

        // Prepare assertion store
        let assertion_contract_bytecode = Bytecode::LegacyRaw(assertionContract.to_vec().into());

        let config = ExecutorConfig { spec_id, chain_id, assertion_gas_limit: 3_000_000 };

        let mut store = MockStore::new(config.clone());
        store
            .insert(*assertion_adopter, vec![assertion_contract_bytecode])
            .expect("Failed to store assertions");

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
        let tx_validation = rt
            .block_on(async move {
                let cancellation_token = tokio_util::sync::CancellationToken::new();

                let (reader, handle) = store.cancellable_reader(cancellation_token.clone());

                let mut assertion_executor = config.build(db, reader);

                // Commit current journal state so that it is available for assertions and
                // triggering tx
                let mut fork_db = ForkDb::new(assertion_executor.db.clone());
                fork_db.commit(state);

                // Store assertions
                let validate_result =
                    assertion_executor.validate_transaction(block, tx_env, &mut fork_db).await;

                cancellation_token.cancel();

                let _ = handle.await;

                validate_result
            })
            .map_err(|e| format!("Assertion Executor Error: {:#?}", e))?;
        let assertion_contract = tx_validation.assertions_executions.first().unwrap();
        let total_assertion_gas = tx_validation.total_assertions_gas();
        let total_assertions_ran = tx_validation.total_assertion_funcs_ran();
        let mut assertion_gas_message = format!(
            "Assertion Functions gas cost\n  Total gas cost: {}\n  Total assertions ran: {}\n",
            total_assertion_gas, total_assertions_ran
        );

        // Format individual assertion function results
        for (fn_selector_index, assertion_fn) in
            assertion_contract.assertion_fns_results.iter().enumerate()
        {
            assertion_gas_message.push_str(&format!(
                "   └─ [selector {}:index {}] gas cost: {}\n",
                assertion_fn.id.fn_selector,
                fn_selector_index,
                assertion_fn.as_result().gas_used()
            ));
        }
        executor.console_log(ccx, assertion_gas_message);

        if !tx_validation.is_valid() {
            if !tx_validation.result_and_state.result.is_success() {
                let decoded_error = decode_revert_error(&tx_validation.result_and_state.result);
                bail!("Transaction Execution Reverted: {}", decoded_error.reason());
            }

            let mut error_msg = format!("\n  {assertionContractLabel} Assertions Failed:\n");

            // Collect failed assertions
            let reverted_assertions: HashMap<_, _> = assertion_contract
                .assertion_fns_results
                .iter()
                .enumerate()
                .filter(|(_, assertion_fn)| !assertion_fn.is_success())
                .map(|(fn_selector_index, assertion_fn)| {
                    let key = format!(
                        "[selector {}:index {}]",
                        assertion_fn.id.fn_selector, fn_selector_index
                    );
                    let revert = decode_revert_error(assertion_fn.as_result());
                    (key, revert)
                })
                .collect();

            // Format error messages
            for (key, revert) in reverted_assertions {
                error_msg.push_str(&format!(
                    "   └─ {} - Revert Reason: {} \n",
                    key,
                    revert.reason()
                ));
            }

            executor.console_log(ccx, error_msg);
            bail!("Assertions Reverted");
        }
        Ok(Default::default())
    }
}

fn decode_revert_error(revert: &ExecutionResult) -> Revert {
    Revert::abi_decode(&revert.clone().into_output().unwrap_or_default(), false)
        .unwrap_or(Revert::new(("Unknown Revert Reason".to_string(),)))
}
