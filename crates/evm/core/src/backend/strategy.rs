use std::{any::Any, fmt::Debug};

use crate::InspectorExt;

use super::{Backend, BackendInner, Fork, ForkDB, FoundryEvmInMemoryDB};
use alloy_primitives::Address;
use eyre::{Context, Result};
use revm::{
    db::CacheDB,
    primitives::{EnvWithHandlerCfg, ResultAndState},
    DatabaseRef, JournaledState,
};
use serde::{Deserialize, Serialize};

/// Context for [BackendStrategy].
pub trait BackendStrategyContext: Debug + Send + Sync + Any {
    /// Clone the strategy context.
    fn new_cloned(&self) -> Box<dyn BackendStrategyContext>;
    /// Alias as immutable reference of [Any].
    fn as_any_ref(&self) -> &dyn Any;
    /// Alias as mutable reference of [Any].
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl Clone for Box<dyn BackendStrategyContext> {
    fn clone(&self) -> Self {
        self.new_cloned()
    }
}

/// Default strategy context object.
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

/// Stateless strategy runner for [BackendStrategy].
pub trait BackendStrategyRunner: Debug + Send + Sync {
    /// Strategy name used when printing.
    fn name(&self) -> &'static str;

    /// Clone the strategy runner.
    fn new_cloned(&self) -> Box<dyn BackendStrategyRunner>;

    /// Executes the configured test call of the `env` without committing state changes.
    fn inspect(
        &self,
        backend: &mut Backend,
        env: &mut EnvWithHandlerCfg,
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

    /// Clones the account data from the `active` db into the `ForkDB`
    fn merge_db_account_data(
        &self,
        ctx: &mut dyn BackendStrategyContext,
        addr: Address,
        active: &ForkDB,
        fork_db: &mut ForkDB,
    );
}

impl Clone for Box<dyn BackendStrategyRunner> {
    fn clone(&self) -> Self {
        self.new_cloned()
    }
}

/// Implements [BackendStrategyRunner] for EVM.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EvmBackendStrategyRunner;

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
            merge_account_data(accounts, db, active_journaled_state, target_fork)
        } else {
            merge_account_data(accounts, mem_db, active_journaled_state, target_fork)
        }
    }
}

impl BackendStrategyRunner for EvmBackendStrategyRunner {
    fn name(&self) -> &'static str {
        "evm"
    }

    fn new_cloned(&self) -> Box<dyn BackendStrategyRunner> {
        Box::new(self.clone())
    }

    fn inspect(
        &self,
        backend: &mut Backend,
        env: &mut EnvWithHandlerCfg,
        inspector: &mut dyn InspectorExt,
        _inspect_ctx: Box<dyn Any>,
    ) -> Result<ResultAndState> {
        let mut evm = crate::utils::new_evm_with_inspector(backend, env.clone(), inspector);

        let res = evm.transact().wrap_err("EVM error")?;

        env.env = evm.context.evm.inner.env;

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
        merge_journaled_state_data(addr, active_journaled_state, fork_journaled_state);
    }

    fn merge_db_account_data(
        &self,
        _ctx: &mut dyn BackendStrategyContext,
        addr: Address,
        active: &ForkDB,
        fork_db: &mut ForkDB,
    ) {
        merge_db_account_data(addr, active, fork_db);
    }
}

/// Strategy for [Backend].
#[derive(Debug)]
pub struct BackendStrategy {
    /// Strategy runner.
    pub runner: Box<dyn BackendStrategyRunner>,
    /// Strategy context.
    pub context: Box<dyn BackendStrategyContext>,
}

impl BackendStrategy {
    /// Creates a new EVM strategy for the [Backend].
    pub fn new_evm() -> Self {
        Self { runner: Box::new(EvmBackendStrategyRunner), context: Box::new(()) }
    }
}

impl Clone for BackendStrategy {
    fn clone(&self) -> Self {
        Self { runner: self.runner.clone(), context: self.context.clone() }
    }
}

//// Clones the data of the given `accounts` from the `active` database into the `fork_db`
/// This includes the data held in storage (`CacheDB`) and kept in the `JournaledState`.
pub(crate) fn merge_account_data<ExtDB: DatabaseRef>(
    accounts: impl IntoIterator<Item = Address>,
    active: &CacheDB<ExtDB>,
    active_journaled_state: &mut JournaledState,
    target_fork: &mut Fork,
) {
    for addr in accounts.into_iter() {
        merge_db_account_data(addr, active, &mut target_fork.db);
        merge_journaled_state_data(addr, active_journaled_state, &mut target_fork.journaled_state);
    }

    // need to mock empty journal entries in case the current checkpoint is higher than the existing
    // journal entries
    while active_journaled_state.journal.len() > target_fork.journaled_state.journal.len() {
        target_fork.journaled_state.journal.push(Default::default());
    }

    *active_journaled_state = target_fork.journaled_state.clone();
}

/// Clones the account data from the `active_journaled_state`  into the `fork_journaled_state`
fn merge_journaled_state_data(
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
fn merge_db_account_data<ExtDB: DatabaseRef>(
    addr: Address,
    active: &CacheDB<ExtDB>,
    fork_db: &mut ForkDB,
) {
    trace!(?addr, "merging database data");

    let Some(acc) = active.accounts.get(&addr) else { return };

    // port contract cache over
    if let Some(code) = active.contracts.get(&acc.info.code_hash) {
        trace!("merging contract cache");
        fork_db.contracts.insert(acc.info.code_hash, code.clone());
    }

    // port account storage over
    use std::collections::hash_map::Entry;
    match fork_db.accounts.entry(addr) {
        Entry::Vacant(vacant) => {
            trace!("target account not present - inserting from active");
            // if the fork_db doesn't have the target account
            // insert the entire thing
            vacant.insert(acc.clone());
        }
        Entry::Occupied(mut occupied) => {
            trace!("target account present - merging storage slots");
            // if the fork_db does have the system,
            // extend the existing storage (overriding)
            let fork_account = occupied.get_mut();
            fork_account.storage.extend(&acc.storage);
        }
    }
}
