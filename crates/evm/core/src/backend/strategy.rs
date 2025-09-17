use std::{any::Any, fmt::Debug};

use crate::{
    Env, InspectorExt,
    backend::{JournaledState, update_state},
    env::AsEnvMut,
    evm::new_evm_with_inspector,
    utils::configure_tx_req_env,
};

use super::{Backend, BackendInner, Fork, ForkDB, FoundryEvmInMemoryDB};
use alloy_evm::Evm;
use alloy_primitives::Address;
use alloy_rpc_types::TransactionRequest;
use eyre::{Context, Result};
use revm::{
    DatabaseCommit, DatabaseRef, context_interface::result::ResultAndState, database::CacheDB,
};
use serde::{Deserialize, Serialize};

/// Context for [BackendStrategyRunner].
pub trait BackendStrategyContext: Debug + Send + Sync + Any {
    /// Clone the strategy context.
    fn new_cloned(&self) -> Box<dyn BackendStrategyContext>;
    /// Alias as immutable reference of [Any].
    fn as_any_ref(&self) -> &dyn Any;
    /// Alias as mutable reference of [Any].
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl BackendStrategyContext for () {
    fn new_cloned(&self) -> Box<dyn BackendStrategyContext> {
        Box::new(())
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Strategy for [super::Backend].
#[derive(Debug)]
pub struct BackendStrategy {
    /// Strategy runner.
    pub runner: &'static dyn BackendStrategyRunner,
    /// Strategy context.
    pub context: Box<dyn BackendStrategyContext>,
}

impl BackendStrategy {
    /// Create a new instance of [BackendStrategy]
    pub fn new_evm() -> Self {
        Self { runner: &EvmBackendStrategyRunner, context: Box::new(()) }
    }
}

impl Clone for BackendStrategy {
    fn clone(&self) -> Self {
        Self { runner: self.runner, context: self.context.new_cloned() }
    }
}

pub trait BackendStrategyRunner: Debug + Send + Sync {
    fn inspect(
        &self,
        backend: &mut Backend,
        env: &mut Env,
        inspector: &mut dyn InspectorExt,
        inspect_ctx: Box<dyn Any>,
    ) -> Result<ResultAndState>;

    /// When creating or switching forks, we update the AccountInfo of the contract
    fn update_fork_db(
        &self,
        ctx: &mut dyn BackendStrategyContext,
        active_fork: Option<&Fork>,
        mem_db: &FoundryEvmInMemoryDB,
        backend_inner: &BackendInner,
        active_journaled_state: &mut JournaledState,
        target_fork: &mut Fork,
    );

    /// Clones the account data from the `active_journaled_state` into the `fork_journaled_state`
    fn merge_journaled_state_data(
        &self,
        ctx: &mut dyn BackendStrategyContext,
        addr: Address,
        active_journaled_state: &JournaledState,
        fork_journaled_state: &mut JournaledState,
    );

    fn merge_db_account_data(
        &self,
        ctx: &mut dyn BackendStrategyContext,
        addr: Address,
        active: &ForkDB,
        fork_db: &mut ForkDB,
    );

    fn transact_from_tx(
        &self,
        backend: &mut Backend,
        tx: &TransactionRequest,
        env: Env,
        journaled_state: &mut JournaledState,
        inspector: &mut dyn InspectorExt,
        inspect_ctx: Box<dyn Any>,
    ) -> eyre::Result<()>;
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EvmBackendStrategyRunner;

impl BackendStrategyRunner for EvmBackendStrategyRunner {
    fn inspect(
        &self,
        backend: &mut Backend,
        env: &mut Env,
        inspector: &mut dyn InspectorExt,
        _inspect_ctx: Box<dyn Any>,
    ) -> Result<ResultAndState> {
        let mut evm = crate::evm::new_evm_with_inspector(backend, env.to_owned(), inspector);

        let res = evm.transact(env.tx.clone()).wrap_err("EVM error")?;

        *env = evm.as_env_mut().to_owned();

        Ok(res)
    }

    fn update_fork_db(
        &self,
        _ctx: &mut dyn BackendStrategyContext,
        active_fork: Option<&Fork>,
        mem_db: &FoundryEvmInMemoryDB,
        backend_inner: &BackendInner,
        active_journaled_state: &mut JournaledState,
        target_fork: &mut Fork,
    ) {
        self.update_fork_db_contracts(
            active_fork,
            mem_db,
            backend_inner,
            active_journaled_state,
            target_fork,
        )
    }

    fn merge_journaled_state_data(
        &self,
        _ctx: &mut dyn BackendStrategyContext,
        addr: Address,
        active_journaled_state: &JournaledState,
        fork_journaled_state: &mut JournaledState,
    ) {
        EvmBackendMergeStrategy::merge_journaled_state_data(
            addr,
            active_journaled_state,
            fork_journaled_state,
        );
    }

    fn merge_db_account_data(
        &self,
        _ctx: &mut dyn BackendStrategyContext,
        addr: Address,
        active: &ForkDB,
        fork_db: &mut ForkDB,
    ) {
        EvmBackendMergeStrategy::merge_db_account_data(addr, active, fork_db);
    }

    fn transact_from_tx(
        &self,
        backend: &mut Backend,
        tx: &TransactionRequest,
        mut env: Env,
        journaled_state: &mut JournaledState,
        inspector: &mut dyn InspectorExt,
        _inspect_ctx: Box<dyn Any>,
    ) -> eyre::Result<()> {
        backend.commit(journaled_state.state.clone());

        let res = {
            configure_tx_req_env(&mut env.as_env_mut(), tx, None)?;

            let mut db = backend.clone();
            let mut evm = new_evm_with_inspector(&mut db, env.to_owned(), inspector);
            evm.journaled_state.depth = journaled_state.depth + 1;
            evm.transact(env.tx)?
        };

        backend.commit(res.state);
        update_state(&mut journaled_state.state, backend, None)?;

        Ok(())
    }
}

impl EvmBackendStrategyRunner {
    /// Merges the state of all `accounts` from the currently active db into the given `fork`
    pub(crate) fn update_fork_db_contracts(
        &self,
        active_fork: Option<&Fork>,
        mem_db: &FoundryEvmInMemoryDB,
        backend_inner: &BackendInner,
        active_journaled_state: &mut JournaledState,
        target_fork: &mut Fork,
    ) {
        let accounts = backend_inner.persistent_accounts.iter().copied();
        if let Some(db) = active_fork.map(|f| &f.db) {
            EvmBackendMergeStrategy::merge_account_data(
                accounts,
                db,
                active_journaled_state,
                target_fork,
            )
        } else {
            EvmBackendMergeStrategy::merge_account_data(
                accounts,
                mem_db,
                active_journaled_state,
                target_fork,
            )
        }
    }
}

pub struct EvmBackendMergeStrategy;
impl EvmBackendMergeStrategy {
    /// Clones the data of the given `accounts` from the `active` database into the `fork_db`
    /// This includes the data held in storage (`CacheDB`) and kept in the `JournaledState`.
    pub fn merge_account_data<ExtDB: DatabaseRef>(
        accounts: impl IntoIterator<Item = Address>,
        active: &CacheDB<ExtDB>,
        active_journaled_state: &mut JournaledState,
        target_fork: &mut Fork,
    ) {
        for addr in accounts.into_iter() {
            Self::merge_db_account_data(addr, active, &mut target_fork.db);
            Self::merge_journaled_state_data(
                addr,
                active_journaled_state,
                &mut target_fork.journaled_state,
            );
        }

        *active_journaled_state = target_fork.journaled_state.clone();
    }

    /// Clones the account data from the `active_journaled_state`  into the `fork_journaled_state`
    pub fn merge_journaled_state_data(
        addr: Address,
        active_journaled_state: &JournaledState,
        fork_journaled_state: &mut JournaledState,
    ) {
        if let Some(mut acc) = active_journaled_state.state.get(&addr).cloned() {
            trace!(?addr, "updating journaled_state account data");
            if let Some(fork_account) = fork_journaled_state.state.get_mut(&addr) {
                // This will merge the fork's tracked storage with active storage and update values
                fork_account.storage.extend(std::mem::take(&mut acc.storage));
                // swap them so we can insert the account as whole in the next step
                std::mem::swap(&mut fork_account.storage, &mut acc.storage);
            }
            fork_journaled_state.state.insert(addr, acc);
        }
    }

    /// Clones the account data from the `active` db into the `ForkDB`
    pub fn merge_db_account_data<ExtDB: DatabaseRef>(
        addr: Address,
        active: &CacheDB<ExtDB>,
        fork_db: &mut ForkDB,
    ) {
        let mut acc = if let Some(acc) = active.cache.accounts.get(&addr).cloned() {
            acc
        } else {
            // Account does not exist
            return;
        };

        if let Some(code) = active.cache.contracts.get(&acc.info.code_hash).cloned() {
            fork_db.cache.contracts.insert(acc.info.code_hash, code);
        }

        if let Some(fork_account) = fork_db.cache.accounts.get_mut(&addr) {
            // This will merge the fork's tracked storage with active storage and update values
            fork_account.storage.extend(std::mem::take(&mut acc.storage));
            // swap them so we can insert the account as whole in the next step
            std::mem::swap(&mut fork_account.storage, &mut acc.storage);
        }

        fork_db.cache.accounts.insert(addr, acc);
    }
}
