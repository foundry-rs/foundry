//! Foundry's executor backend: RPC forks, persistent accounts, snapshots and cheatcode policy over
//! evm2.
//!
//! # Execution contract
//!
//! [`Backend`] owns an [`ExecutionLifecycle`]: the accepted state of every execution context (the
//! local database and each fork), the active selection, persistent accounts and snapshots. It does
//! not implement a database trait; executions run on [`evm2::Evm`]s built from it:
//!
//! 1. [`Backend::create_evm`] builds an EVM over the active context. The EVM does not borrow the
//!    backend: it reads the context's accepted state through a shared view.
//! 2. While the EVM runs, the backend is free. Inspectors that implement cheatcodes hold `&mut
//!    Backend` (or a [`CowBackend`]) for the duration of the execution, and call its operations
//!    with the `&mut Evm` they receive from their hooks. Operations that act on the running
//!    transaction, such as fork selection, snapshots, `vm.transact` or account refreshes, take that
//!    EVM explicitly together with the [`ExecutionPhase`]; operations between executions, such as
//!    [`Backend::insert_account_info`], do not.
//! 3. After a committed execution, [`Backend::park`] accepts the EVM's state, environment and
//!    network state back into the active context. Dropping the EVM instead discards everything it
//!    did, as for calls.
//!
//! [`Backend::inspect_call`] and [`Backend::inspect_commit`] wrap these steps for executions that
//! need no backend access from their inspector.

use crate::{
    constants::{CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER},
    evm::{
        BackingDb, EnvTransition, EthEvmNetwork, EvmEnvFor, EvmFactoryFor, ExecutionLifecycle,
        ExecutionPhase, ForkDb, ForkInfo, FoundryDatabase, FoundryEvmExt, FoundryEvmFactory,
        FoundryEvmFor, FoundryEvmNetwork, NestedTx, Parked, SpecFor, TxFor, TxResultFor, TypesFor,
        block_txs, evm_env,
    },
    fork::{CreateFork, Fork as RemoteFork, ForkId, ForkResult, MultiFork},
    opts::{EvmOpts, ExecutionSpecContext},
    utils::{
        apply_chain_and_block_specific_env_changes_for_chain,
        apply_chain_specific_tx_replay_env_changes_for_chain,
    },
};
use alloy_chains::Chain;
use alloy_consensus::{BlockHeader, Typed2718, transaction::Recovered};
use alloy_eips::BlockNumHash;
use alloy_genesis::GenesisAccount;
use alloy_network::{AnyNetwork, AnyRpcBlock, AnyRpcTransaction, TransactionResponse};
use alloy_primitives::{
    Address, B256, ChainId, KECCAK256_EMPTY, TxKind, U256, keccak256, map::AddressSet, uint,
};
use alloy_rpc_types::{BlockNumberOrTag, BlockTransactions};
use evm2::{
    EvmFeatures, Inspector,
    bytecode::Bytecode,
    evm::{
        AccountInfo, Cache, CacheDB, DynDatabase, EmptyDB, PendingState,
        bal::{Bal, BlockAccessIndex},
    },
};
use eyre::WrapErr;
use foundry_common::{SYSTEM_TRANSACTION_TYPE, is_known_system_sender};
use foundry_evm_networks::NetworkConfigs;
pub use foundry_fork_db::{
    AccountFetchPolicy, BlockchainDb, ForkBlock, ForkBlockEnv, SharedBackend,
    cache::BlockchainDbMeta,
};
use std::{collections::BTreeMap, mem, sync::Arc, time::Instant};

mod diagnostic;
pub use diagnostic::RevertDiagnostic;

mod bal;

mod error;
pub use error::{
    BackendError, BackendResult, DatabaseError, DatabaseResult, handler_error, transaction_error,
};

mod cow;
pub use cow::CowBackend;

mod snapshot;
pub use snapshot::RevertStateSnapshotAction;

#[cfg(test)]
mod tests;

/// Represents a numeric `ForkId` valid only for the existence of the `Backend`.
///
/// The difference between `ForkId` and `LocalForkId` is that `ForkId` tracks pairs of `endpoint +
/// block` which can be reused by multiple tests, whereas the `LocalForkId` is unique within a test
pub type LocalForkId = U256;

/// Backing database of an RPC fork: the shared fork cache, read by evm2.
pub type RpcForkDb = ForkDb<SharedBackend<AnyNetwork, revm::context::BlockEnv>>;

/// All accounts that will have persistent storage across fork swaps.
const DEFAULT_PERSISTENT_ACCOUNTS: [Address; 3] =
    [CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, CALLER];

/// `bytes32("failed")`, as a storage slot key into [`CHEATCODE_ADDRESS`].
///
/// Used by all `forge-std` test contracts and newer `DSTest` test contracts as a global marker for
/// a failed test.
pub const GLOBAL_FAIL_SLOT: U256 =
    uint!(0x6661696c65640000000000000000000000000000000000000000000000000000_U256);

/// Account field changed by an out-of-band fork RPC mutation.
#[derive(Clone, Copy, Debug)]
pub enum ForkAccountField {
    Balance,
    Nonce,
    Code,
}

impl ForkAccountField {
    fn update(self, target: &mut AccountInfo, refreshed: &AccountInfo) {
        match self {
            Self::Balance => target.balance = refreshed.balance,
            Self::Nonce => target.nonce = refreshed.nonce,
            Self::Code => {
                target.code_hash = refreshed.code_hash;
                target.code = refreshed.code.clone();
            }
        }
    }
}

/// Block data required to execute or position a fork at a transaction.
struct TransactionForkTarget {
    /// Block whose state the transaction executes on.
    fork_block: BlockNumHash,
    /// The transaction.
    transaction: AnyRpcTransaction,
    /// Block containing the transaction, or the latest block for a pending transaction.
    block: AnyRpcBlock,
    /// Whether the transaction is mined.
    mined: bool,
    /// Index of the transaction in `block`, if it is mined.
    index: Option<usize>,
}

/// Foundry's executor backend.
///
/// See the [module documentation](self) for the execution contract. Cloning a backend clones the
/// accepted state of every context without copying it (contexts share their caches
/// copy-on-write), which parallel fuzzing relies on.
#[derive(Clone, Debug)]
pub struct Backend<FEN: FoundryEvmNetwork = EthEvmNetwork> {
    /// Network configuration of the executed chain.
    networks: NetworkConfigs,
    /// Manager of the remote fork caches.
    forks: MultiFork,
    /// Contexts, fork selection, persistent accounts and snapshots.
    lifecycle: ExecutionLifecycle<EvmFactoryFor<FEN>>,
    /// Accounts allowed to execute cheatcodes.
    cheatcode_access: AddressSet,
    /// Caller of the test.
    caller: Option<Address>,
    /// Whether a reverted state snapshot recorded a test failure.
    has_state_snapshot_failure: bool,
    /// Block number the active fork reports instead of its own.
    fork_block_number_override: Option<u64>,
    /// Block access list attached to EVMs created until the next park.
    bal: Option<(Arc<Bal>, BlockAccessIndex)>,
}

impl<FEN: FoundryEvmNetwork> Backend<FEN> {
    /// Creates a backend with a spawned fork manager.
    ///
    /// If `fork` is `Some`, execution starts on that fork with environment `env`, otherwise on an
    /// empty local database.
    pub fn spawn(fork: Option<CreateFork>, env: EvmEnvFor<FEN>) -> eyre::Result<Self> {
        Self::new(MultiFork::spawn(), fork, env)
    }

    /// Resolves the fork once, configuring its environment before initializing network state.
    pub fn spawn_fork(
        fork: CreateFork,
        configure_env: impl FnOnce(&mut EvmEnvFor<FEN>, ExecutionSpecContext),
    ) -> eyre::Result<Self> {
        let forks = MultiFork::spawn();
        let opts = &fork.evm_opts;
        let resolved = forks.create_fork(fork.clone())?;
        let context = resolved.fork.context();
        let chain_id = opts.chain_id_override().unwrap_or(context.execution_chain_id);
        let mut env = opts.fork_env_from_block::<FEN, AnyNetwork>(
            chain_id,
            context.source_chain_id,
            &resolved.fork.block,
        );
        configure_env(
            &mut env,
            ExecutionSpecContext::fork(context.source_chain_id, context.hardfork),
        );

        let mut backend = Self::with_fork_manager(forks, EmptyDB::default(), env);
        let id = backend.insert_prepared_fork(fork.url, resolved, env);
        let mut evm = backend.lifecycle.create_evm();
        let _ = backend.lifecycle.select_fork(&mut evm, Some(id), ExecutionPhase::Boundary)?;
        backend.lifecycle.park(evm);
        Ok(backend)
    }

    /// Creates a backend that uses `forks` for remote fork caches.
    ///
    /// If `fork` is `Some`, execution starts on that fork with environment `env` exactly, otherwise
    /// on an empty local database. Prefer [`Self::spawn`].
    pub fn new(
        forks: MultiFork,
        fork: Option<CreateFork>,
        env: EvmEnvFor<FEN>,
    ) -> eyre::Result<Self> {
        trace!(target: "backend", forking_mode=?fork.is_some(), "creating executor backend");
        let mut backend = Self::with_fork_manager(forks, EmptyDB::default(), env);
        if let Some(fork) = fork {
            let id = backend.insert_fork(fork, Some(env))?;
            let mut evm = backend.lifecycle.create_evm();
            let _ = backend.lifecycle.select_fork(&mut evm, Some(id), ExecutionPhase::Boundary)?;
            backend.lifecycle.park(evm);
        }
        Ok(backend)
    }

    /// Creates a backend executing locally over `db`, such as an in-memory database.
    pub fn with_db(db: impl BackingDb, env: EvmEnvFor<FEN>) -> Self {
        Self::with_fork_manager(MultiFork::spawn(), db, env)
    }

    fn with_fork_manager(forks: MultiFork, db: impl BackingDb, env: EvmEnvFor<FEN>) -> Self {
        let mut lifecycle = ExecutionLifecycle::new(EvmFactoryFor::<FEN>::default(), db, env);
        for account in DEFAULT_PERSISTENT_ACCOUNTS {
            lifecycle.forks_mut().add_persistent(account);
        }
        // Seed network state (genesis, sentinels) so reads between executions see it.
        let evm = lifecycle.create_evm();
        lifecycle.park(evm);
        Self {
            networks: NetworkConfigs::default(),
            forks,
            lifecycle,
            cheatcode_access: AddressSet::default(),
            caller: None,
            has_state_snapshot_failure: false,
            fork_block_number_override: None,
            bal: None,
        }
    }

    /// Clones execution state with an independent registry for newly created forks.
    ///
    /// Remote caches remain shared, but forks created by this execution are released with it.
    pub fn clone_with_fork_scope(&self) -> eyre::Result<Self> {
        let mut backend = self.clone();
        backend.forks = self.forks.scoped()?;
        Ok(backend)
    }

    /// Creates an empty local backend sharing this backend's fork manager and environment.
    pub fn clone_empty(&self) -> Self {
        let mut backend = Self::with_fork_manager(
            self.forks.clone(),
            EmptyDB::default(),
            self.lifecycle.active().env,
        );
        backend.networks = self.networks;
        backend
    }

    /// Replaces this backend with one starting on `fork`, reusing the fork manager's remote
    /// caches.
    pub fn replace_fork(
        &mut self,
        fork: Option<CreateFork>,
        env: EvmEnvFor<FEN>,
    ) -> eyre::Result<()> {
        let networks = self.networks;
        *self = Self::new(self.forks.clone(), fork, env)?;
        self.networks = networks;
        Ok(())
    }

    /// Returns the remote fork of the active context, if execution is forked over RPC.
    pub fn fork(&self) -> Option<RemoteFork> {
        self.active_fork_info()?.remote.clone()
    }

    /// Builds execution environments from this backend's selected remote block.
    pub async fn env(
        &self,
        opts: &EvmOpts,
    ) -> eyre::Result<(EvmEnvFor<FEN>, crate::evm::TransactionRequestFor<FEN>)> {
        opts.env_at_fork::<FEN>(self.fork().as_ref()).await
    }

    /// Checks library-linking availability against this backend's remote state.
    pub async fn can_use_create2_deployer(&self, deployer: Address) -> eyre::Result<bool> {
        let Some(fork) = self.fork() else {
            return Ok(deployer == DEFAULT_CREATE2_DEPLOYER);
        };
        let mut request = self
            .active_fork_options()
            .ok_or_else(|| eyre::eyre!("active fork configuration is unavailable"))?;
        request.evm_opts.create2_deployer = deployer;
        request.evm_opts.can_use_create2_deployer_at(Some(&fork)).await
    }

    /// Reads a sender nonce at this backend's remote block before local execution.
    pub async fn transaction_count(&self, sender: Address) -> eyre::Result<u64> {
        let request =
            self.active_fork_options().ok_or_else(|| eyre::eyre!("backend is not forked"))?;
        let fork = self.fork().ok_or_else(|| eyre::eyre!("backend is not forked over RPC"))?;
        request.evm_opts.transaction_count_at_fork(sender, &fork).await
    }

    /// Returns the active network configuration.
    pub const fn networks(&self) -> NetworkConfigs {
        self.networks
    }

    /// Sets the active network configuration.
    pub const fn set_networks(&mut self, networks: NetworkConfigs) {
        self.networks = networks;
    }

    /// Sets the block number the active fork reports while executing transactions of that block
    /// on top of its parent's state.
    pub const fn set_fork_block_number_override(&mut self, block_number: u64) {
        self.fork_block_number_override = Some(block_number);
    }

    /// Returns the EVM factory.
    pub const fn factory(&self) -> &EvmFactoryFor<FEN> {
        self.lifecycle.factory()
    }

    /// Returns the execution lifecycle, for read-only inspection of contexts and forks.
    pub const fn lifecycle(&self) -> &ExecutionLifecycle<EvmFactoryFor<FEN>> {
        &self.lifecycle
    }

    /// Returns the environment of the active context, as of the last park.
    pub fn evm_env(&self) -> EvmEnvFor<FEN> {
        self.lifecycle.active().env
    }

    /// Returns the environment of the active context for modification between executions.
    pub fn evm_env_mut(&mut self) -> &mut EvmEnvFor<FEN> {
        &mut self.lifecycle.active_mut().env
    }

    /// Sets the spec of the active context, keeping Foundry's execution policy.
    pub fn set_spec_id(&mut self, spec: impl Into<SpecFor<FEN>>) -> &mut Self {
        let env = self.evm_env();
        *self.evm_env_mut() = self.lifecycle.factory().env_with_spec(env, spec.into());
        self
    }

    // ---- Executions ----

    /// Creates an EVM over the active context. See the [module documentation](self).
    pub fn create_evm<'a>(&self) -> FoundryEvmFor<'a, FEN> {
        let mut evm = self.lifecycle.create_evm();
        self.apply_bal(&mut evm);
        evm
    }

    /// Accepts a committed EVM's state, environment and network state into the active context,
    /// and detaches the block access list set with [`Self::set_bal`].
    pub fn park(&mut self, evm: FoundryEvmFor<'_, FEN>) {
        self.lifecycle.park(evm);
        self.bal = None;
    }

    /// Applies an environment change deferred by an in-flight operation. Call before every
    /// transaction executed on an EVM that was not freshly created.
    pub fn begin_transaction(&mut self, evm: &mut FoundryEvmFor<'_, FEN>) {
        self.lifecycle.begin_transaction(evm);
    }

    /// Executes `tx` over the active context with `inspector` and discards its state.
    pub fn inspect_call<'a>(
        &self,
        tx: &Recovered<TxFor<FEN>>,
        inspector: impl Inspector<TypesFor<FEN>> + 'a,
    ) -> eyre::Result<TxResultFor<FEN>> {
        let mut evm: FoundryEvmFor<'a, FEN> = self.create_evm();
        evm.set_inspector(inspector);
        evm.ext_mut().for_transaction(tx);
        let executed = evm.transact(tx).map_err(handler_error)?;
        Ok(executed.result().clone())
    }

    /// Executes `tx` over the active context with `inspector` and accepts its state.
    pub fn inspect_commit<'a>(
        &mut self,
        tx: &Recovered<TxFor<FEN>>,
        inspector: impl Inspector<TypesFor<FEN>> + 'a,
    ) -> eyre::Result<TxResultFor<FEN>> {
        let mut evm: FoundryEvmFor<'a, FEN> = self.create_evm();
        evm.set_inspector(inspector);
        evm.ext_mut().for_transaction(tx);
        let result = evm.transact(tx).map_err(handler_error)?.commit();
        evm.clear_inspector();
        self.park(evm);
        Ok(result)
    }

    /// Records the caller and test contract of the next test execution.
    ///
    /// The test contract is the call target, or the address `caller` creates at its current nonce.
    pub fn initialize(&mut self, caller: Address, kind: TxKind) {
        self.set_caller(caller);
        let test_contract = match kind {
            TxKind::Call(to) => to,
            TxKind::Create => {
                let nonce = self.basic(caller).ok().flatten().map_or(0, |account| account.nonce);
                caller.create(nonce)
            }
        };
        self.set_test_contract(test_contract);
    }

    // ---- Accepted state between executions ----

    /// Returns an account of the active context's accepted state.
    pub fn basic(&self, address: Address) -> StateResult<Option<AccountInfo>> {
        let mut account = self.lifecycle.accepted_db().get_account(&address)?;
        if let Some(account) = &mut account
            && account.code.is_none()
            && account.code_hash != KECCAK256_EMPTY
        {
            account.code = Some(self.code_by_hash(account.code_hash)?);
        }
        Ok(account)
    }

    /// Returns code of the active context's accepted state.
    pub fn code_by_hash(&self, code_hash: B256) -> StateResult<Bytecode> {
        self.lifecycle.accepted_db().get_code_by_hash(&code_hash)
    }

    /// Returns a storage slot of the active context's accepted state.
    pub fn storage(&self, address: Address, slot: U256) -> StateResult<U256> {
        self.lifecycle.accepted_db().get_storage(&address, &slot)
    }

    /// Returns a block hash of the active context.
    pub fn block_hash(&self, number: U256) -> StateResult<B256> {
        self.lifecycle.accepted_db().get_block_hash(&number)
    }

    /// Inserts account info into the active context's accepted state.
    ///
    /// Use this between executions only; a running EVM does not observe it.
    pub fn insert_account_info(&mut self, address: Address, mut account: AccountInfo) {
        let cache = self.lifecycle.accepted_mut();
        if let Some(code) = account.code.take()
            && !code.is_empty()
        {
            if account.code_hash == KECCAK256_EMPTY || account.code_hash.is_zero() {
                account.code_hash = code.hash_slow();
            }
            cache.contracts.insert(account.code_hash, code);
        }
        if account.code_hash.is_zero() {
            account.code_hash = KECCAK256_EMPTY;
        }
        cache.accounts.insert(address, Some(account));
    }

    /// Inserts a value into an account's storage without overriding account info.
    ///
    /// Use this between executions only; a running EVM does not observe it.
    pub fn insert_account_storage(&mut self, address: Address, slot: U256, value: U256) {
        self.lifecycle.accepted_mut().storage.entry(address).or_default().slots.insert(slot, value);
    }

    /// Replaces an account's storage without overriding account info.
    ///
    /// When forking, every slot not in `storage` reads as zero instead of being fetched. Use this
    /// between executions only; a running EVM does not observe it.
    pub fn replace_account_storage(
        &mut self,
        address: Address,
        storage: impl IntoIterator<Item = (U256, U256)>,
    ) {
        let account = self.lifecycle.accepted_mut().storage.entry(address).or_default();
        account.wipe();
        account.slots.extend(storage);
    }

    /// Switches the running `evm` to `env`, such as for `vm.chainId` or `vm.setEvmVersion`. See
    /// [`ExecutionLifecycle::set_env`].
    pub fn set_env(
        &mut self,
        evm: &mut FoundryEvmFor<'_, FEN>,
        env: EvmEnvFor<FEN>,
        phase: ExecutionPhase,
    ) -> EnvTransition {
        self.lifecycle.set_env(evm, env, phase)
    }

    /// Accepts a detached transaction's state, such as a call's, into the active context's
    /// accepted state.
    ///
    /// Use this between executions only; a running EVM does not observe it.
    pub fn commit_pending(&mut self, pending: &PendingState) {
        let cache = mem::take(self.lifecycle.accepted_mut());
        let mut db = CacheDB::new(EmptyDB::default());
        db.cache = cache;
        db.commit_pending(pending);
        *self.lifecycle.accepted_mut() = db.cache;
    }

    // ---- Test configuration ----

    /// Sets the address of the `DSTest` contract that is being executed.
    ///
    /// This marks the contract as persistent and grants it cheatcode access.
    pub fn set_test_contract(&mut self, account: Address) -> &mut Self {
        trace!(?account, "setting test account");
        self.lifecycle.forks_mut().add_persistent(account);
        self.cheatcode_access.insert(account);
        self
    }

    /// Sets the caller address and grants it cheatcode access.
    pub fn set_caller(&mut self, account: Address) -> &mut Self {
        trace!(?account, "setting caller account");
        self.caller = Some(account);
        self.cheatcode_access.insert(account);
        self
    }

    /// Returns the caller address.
    pub const fn caller_address(&self) -> Option<Address> {
        self.caller
    }

    /// Returns whether a reverted state snapshot recorded a test failure.
    ///
    /// If an error occurs in a restored state snapshot, the test is considered failed.
    pub const fn has_state_snapshot_failure(&self) -> bool {
        self.has_state_snapshot_failure
    }

    /// Sets the state snapshot failure flag.
    pub const fn set_state_snapshot_failure(&mut self, has_state_snapshot_failure: bool) {
        self.has_state_snapshot_failure = has_state_snapshot_failure;
    }

    /// Returns whether `address` is a precompile of the active context.
    pub fn is_existing_precompile(&self, address: &Address) -> bool {
        let context = self.lifecycle.active();
        self.lifecycle.factory().precompiles(&context.env, &context.ext.get()).contains(address)
    }

    // ---- Persistent accounts and cheatcode access ----

    /// Returns whether `account` persists across fork switches.
    pub fn is_persistent(&self, account: &Address) -> bool {
        self.lifecycle.forks().is_persistent(account)
    }

    /// Marks `account` as persistent across fork switches. Returns whether it was not yet.
    pub fn add_persistent_account(&mut self, account: Address) -> bool {
        trace!(?account, "add persistent account");
        self.lifecycle.forks_mut().add_persistent(account)
    }

    /// Stops `account` from persisting across fork switches. Returns whether it was persistent.
    pub fn remove_persistent_account(&mut self, account: &Address) -> bool {
        trace!(?account, "remove persistent account");
        self.lifecycle.forks_mut().remove_persistent(account)
    }

    /// Marks all `accounts` as persistent.
    pub fn extend_persistent_accounts(&mut self, accounts: impl IntoIterator<Item = Address>) {
        for account in accounts {
            self.add_persistent_account(account);
        }
    }

    /// Stops all `accounts` from persisting.
    pub fn remove_persistent_accounts(&mut self, accounts: impl IntoIterator<Item = Address>) {
        for account in accounts {
            self.remove_persistent_account(&account);
        }
    }

    /// Grants `account` cheatcode access. Returns whether it did not have access yet.
    pub fn allow_cheatcode_access(&mut self, account: Address) -> bool {
        trace!(?account, "allow cheatcode access");
        self.cheatcode_access.insert(account)
    }

    /// Revokes cheatcode access from `account`. Returns whether it had access.
    pub fn revoke_cheatcode_access(&mut self, account: &Address) -> bool {
        trace!(?account, "revoke cheatcode access");
        self.cheatcode_access.remove(account)
    }

    /// Returns whether `account` has cheatcode access.
    pub fn has_cheatcode_access(&self, account: &Address) -> bool {
        self.cheatcode_access.contains(account)
    }

    /// Ensures that `account` has cheatcode access.
    pub fn ensure_cheatcode_access(&self, account: &Address) -> Result<(), BackendError> {
        if !self.has_cheatcode_access(account) {
            return Err(BackendError::NoCheats(*account));
        }
        Ok(())
    }

    /// Like [`Self::ensure_cheatcode_access`], but only enforced while forked.
    pub fn ensure_cheatcode_access_forking_mode(
        &self,
        account: &Address,
    ) -> Result<(), BackendError> {
        if self.is_forked_mode() {
            return self.ensure_cheatcode_access(account);
        }
        Ok(())
    }

    // ---- Fork queries ----

    /// Returns the active fork, or `None` for local execution.
    pub const fn active_fork_id(&self) -> Option<LocalForkId> {
        self.lifecycle.forks().active()
    }

    /// Returns whether execution is forked.
    pub const fn is_forked_mode(&self) -> bool {
        self.active_fork_id().is_some()
    }

    /// Returns whether `id` is the active fork.
    pub fn is_active_fork(&self, id: LocalForkId) -> bool {
        self.active_fork_id() == Some(id)
    }

    /// Returns the identity of the active fork.
    pub fn active_fork_info(&self) -> Option<&ForkInfo> {
        Some(&self.lifecycle.forks().get(self.active_fork_id()?)?.info)
    }

    /// Returns the RPC URL of the active fork.
    pub fn active_fork_url(&self) -> Option<String> {
        Some(self.active_fork_info()?.url.clone())
    }

    /// Returns the configuration the active fork was created with.
    pub fn active_fork_options(&self) -> Option<CreateFork> {
        self.forks.get_fork_options(self.active_fork_info()?.id.clone()).ok()?
    }

    /// Returns the source chain ID of the active fork, independent of chain ID overrides.
    pub fn active_fork_source_chain_id(&self) -> Option<u64> {
        self.lifecycle.active().db.active_fork_source_chain_id()
    }

    /// Returns the RPC block number of the active fork.
    pub fn active_fork_block_number(&self) -> Option<u64> {
        if let Some(block_number) = self.fork_block_number_override {
            return Some(block_number);
        }
        self.active_fork_info()?.block
    }

    /// Returns `id`, or the active fork for `None`, if it exists.
    pub fn ensure_fork(&self, id: Option<LocalForkId>) -> eyre::Result<LocalForkId> {
        match id.or_else(|| self.active_fork_id()) {
            Some(id) if self.lifecycle.forks().get(id).is_some() => Ok(id),
            Some(id) => eyre::bail!("Requested fork `{id}` does not exist"),
            None => eyre::bail!("No fork active"),
        }
    }

    /// Returns the remote fork identifier of `id`.
    pub fn ensure_fork_id(&self, id: LocalForkId) -> eyre::Result<&ForkId> {
        Ok(&self.fork_info(id)?.id)
    }

    fn fork_info(&self, id: LocalForkId) -> eyre::Result<&ForkInfo> {
        self.lifecycle
            .forks()
            .get(id)
            .map(|entry| &entry.info)
            .ok_or_else(|| eyre::eyre!("Requested fork `{id}` does not exist"))
    }

    fn set_fork_info(&mut self, id: LocalForkId, info: ForkInfo) {
        self.lifecycle.forks_mut().get_mut(id).expect("fork exists").info = info;
    }

    fn fork_options(&self, id: &ForkId) -> eyre::Result<CreateFork> {
        self.forks
            .get_fork_options(id.clone())?
            .ok_or_else(|| eyre::eyre!("Requested fork `{id}` does not exist"))
    }

    // ---- Fork lifecycle ----

    /// Creates a fork without selecting it.
    pub fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<LocalForkId> {
        trace!("create fork");
        self.insert_fork(fork, None)
    }

    /// Creates a fork positioned right before `transaction` without selecting it.
    pub fn create_fork_at_transaction(
        &mut self,
        fork: CreateFork,
        transaction: B256,
    ) -> eyre::Result<LocalForkId> {
        trace!(?transaction, "create fork at transaction");
        let id = self.insert_fork(fork, None)?;
        let spec = self.evm_env().spec;
        let (db, env, cache, info) = self.prepare_roll_to_transaction(id, transaction, spec)?;
        self.lifecycle.replace_inactive(Some(id), db, env, cache)?;
        self.set_fork_info(id, info);
        Ok(id)
    }

    /// Creates a fork and selects it.
    pub fn create_select_fork(
        &mut self,
        fork: CreateFork,
        evm: &mut FoundryEvmFor<'_, FEN>,
        phase: ExecutionPhase,
    ) -> eyre::Result<(LocalForkId, EnvTransition)> {
        let id = self.create_fork(fork)?;
        let transition = self.select_fork(id, evm, phase)?;
        Ok((id, transition))
    }

    /// Creates a fork positioned right before `transaction` and selects it.
    pub fn create_select_fork_at_transaction(
        &mut self,
        fork: CreateFork,
        evm: &mut FoundryEvmFor<'_, FEN>,
        phase: ExecutionPhase,
        transaction: B256,
    ) -> eyre::Result<(LocalForkId, EnvTransition)> {
        let id = self.create_fork_at_transaction(fork, transaction)?;
        let transition = self.select_fork(id, evm, phase)?;
        Ok((id, transition))
    }

    /// Makes fork `id` the running context of `evm`.
    ///
    /// The running spec and fee-charge policy carry over to the fork; persistent accounts carry
    /// their state into it.
    pub fn select_fork(
        &mut self,
        id: LocalForkId,
        evm: &mut FoundryEvmFor<'_, FEN>,
        phase: ExecutionPhase,
    ) -> eyre::Result<EnvTransition> {
        trace!(?id, "select fork");
        self.ensure_fork(Some(id))?;
        if self.is_active_fork(id) {
            return Ok(EnvTransition::Applied);
        }
        let running = evm_env(evm);
        let factory = self.lifecycle.factory().clone();
        let context = self.lifecycle.forks_mut().context_mut(Some(id)).expect("fork exists");
        let mut env = factory.env_with_spec(context.env, running.spec);
        env.set_feature(
            EvmFeatures::FEE_CHARGE,
            running.version.features.contains(EvmFeatures::FEE_CHARGE),
        );
        context.env = env;
        self.lifecycle.select_fork(evm, Some(id), phase)
    }

    /// Moves fork `id`, or the active fork for `None`, to `block_number`.
    pub fn roll_fork(
        &mut self,
        id: Option<LocalForkId>,
        block_number: u64,
        evm: &mut FoundryEvmFor<'_, FEN>,
        phase: ExecutionPhase,
    ) -> eyre::Result<EnvTransition> {
        trace!(?id, ?block_number, "roll fork");
        let id = self.ensure_fork(id)?;
        let info = self.fork_info(id)?.clone();
        let options = self.fork_options(&info.id)?;
        let ForkResult { id: fork_id, backend, fork: remote } =
            self.forks.roll_fork(info.id.clone(), block_number)?;
        let env = self.fork_env(&options.evm_opts, &remote, &remote.block, evm_env(evm).spec);
        let db = ForkDb::new(backend, remote.context().source_chain_id);
        let transition = self.lifecycle.roll_fork(evm, Some(id), db, env, phase)?;
        let block = Some(remote.number());
        self.set_fork_info(
            id,
            ForkInfo { id: fork_id, url: info.url, block, remote: Some(remote) },
        );
        Ok(transition)
    }

    /// Moves fork `id`, or the active fork for `None`, to the state right before `transaction`:
    /// the transaction's block environment over its parent's state with the block's preceding
    /// transactions replayed natively.
    pub fn roll_fork_to_transaction(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        evm: &mut FoundryEvmFor<'_, FEN>,
        phase: ExecutionPhase,
    ) -> eyre::Result<EnvTransition> {
        trace!(?id, ?transaction, "roll fork to transaction");
        let id = self.ensure_fork(id)?;
        let (db, env, cache, info) =
            self.prepare_roll_to_transaction(id, transaction, evm_env(evm).spec)?;
        let transition = self.lifecycle.roll_fork_with(evm, Some(id), db, env, cache, phase)?;
        self.set_fork_info(id, info);
        Ok(transition)
    }

    /// Executes the mined `transaction` of fork `id`, or of the active fork for `None`, with its
    /// block's environment, such as for `vm.transact`.
    ///
    /// On the active fork the transaction runs as a nested transaction of the running one, whose
    /// writes are journaled; on another fork it is accepted there and its changes to persistent
    /// accounts are merged into the running transaction.
    pub fn transact(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        evm: &mut FoundryEvmFor<'_, FEN>,
        inspector: Option<&mut dyn Inspector<TypesFor<FEN>>>,
    ) -> eyre::Result<TxResultFor<FEN>> {
        trace!(?id, ?transaction, "execute transaction");
        let id = self.ensure_fork(id)?;
        let info = self.fork_info(id)?;
        eyre::ensure!(
            !info.remote.as_ref().is_some_and(|remote| remote.state_by_number),
            "transaction replay requires hash-addressed state; create a transaction-targeted fork \
             or disable fork_state_by_number"
        );
        let source_chain_id = self.fork_source_chain_id(id)?;

        // The user transacts an arbitrary mined transaction in the current context; execute it
        // as it was mined, in its block's environment.
        // <https://github.com/foundry-rs/foundry/issues/6538>
        let target = self.transaction_target(id, transaction)?;
        let tx = FEN::tx_from_any_rpc(&target.transaction)?;
        let active = self.is_active_fork(id);
        let mut env = if active {
            evm_env(evm)
        } else {
            self.lifecycle.forks().context(Some(id)).expect("fork exists").env
        };
        // The mined transaction is signed for its source chain, whatever chain ID the running
        // transaction or the fork's environment uses.
        env.set_chain_id(source_chain_id);
        env.block = crate::utils::block_env_from_header::<TypesFor<FEN>>(&target.block.header);
        apply_chain_and_block_specific_env_changes_for_chain::<AnyNetwork, _>(
            &mut env,
            &target.block,
            source_chain_id,
            self.networks,
        );
        apply_chain_specific_tx_replay_env_changes_for_chain(&mut env, source_chain_id);

        let ext = if self.networks.is_monad() {
            let index = target
                .index
                .ok_or_else(|| eyre::eyre!("transaction {transaction} has no block position"))?;
            let shared = self.shared_backend(id)?;
            let context = block_context::<FEN>(&shared, &target.block)?;
            let mut ext = self.lifecycle.factory().evm_ext(&env);
            context.prepare(&mut ext, index);
            Some(ext)
        } else {
            None
        };
        let nested = NestedTx { env: Some(env), ext, inspector };
        if active {
            self.lifecycle.nested_transaction_with(evm, &tx, nested)
        } else {
            self.lifecycle.transact_on_inactive(evm, Some(id), &tx, nested)
        }
    }

    /// Executes `tx` as a nested transaction of the running one, in `env` or the running
    /// environment for `None`, and merges its changes into the running transaction.
    pub fn transact_from_tx(
        &mut self,
        tx: &Recovered<TxFor<FEN>>,
        evm: &mut FoundryEvmFor<'_, FEN>,
        env: Option<EvmEnvFor<FEN>>,
        inspector: Option<&mut dyn Inspector<TypesFor<FEN>>>,
    ) -> eyre::Result<TxResultFor<FEN>> {
        trace!("execute signed transaction");
        self.lifecycle.nested_transaction_with(evm, tx, NestedTx { env, ext: None, inspector })
    }

    /// Builds the EVM a nested transaction of the running one executes on, such as an isolated
    /// call. See [`ExecutionLifecycle::nested_evm`]; merge a successful transaction's changes
    /// back with [`merge_nested_changes`](crate::evm::merge_nested_changes).
    pub fn nested_evm<'a>(
        &self,
        evm: &mut FoundryEvmFor<'_, FEN>,
        tx: &Recovered<TxFor<FEN>>,
        env: Option<EvmEnvFor<FEN>>,
    ) -> FoundryEvmFor<'a, FEN> {
        self.lifecycle.nested_evm(evm, tx, env, None)
    }

    /// Moves the running transaction onto a separate frame EVM. See
    /// [`ExecutionLifecycle::begin_frame_evm`].
    pub fn begin_frame_evm<'a>(&self, evm: &mut FoundryEvmFor<'_, FEN>) -> FoundryEvmFor<'a, FEN> {
        self.lifecycle.begin_frame_evm(evm)
    }

    /// Moves the running transaction onto a separate frame EVM executing with `env`. See
    /// [`ExecutionLifecycle::begin_frame_evm_with_env`].
    pub fn begin_frame_evm_with_env<'a>(
        &self,
        evm: &mut FoundryEvmFor<'_, FEN>,
        env: EvmEnvFor<FEN>,
    ) -> FoundryEvmFor<'a, FEN> {
        self.lifecycle.begin_frame_evm_with_env(evm, env)
    }

    /// Hands a frame EVM's state back to `evm`. See [`ExecutionLifecycle::end_frame_evm`].
    pub fn end_frame_evm(evm: &mut FoundryEvmFor<'_, FEN>, frame: FoundryEvmFor<'_, FEN>) {
        ExecutionLifecycle::<EvmFactoryFor<FEN>>::end_frame_evm(evm, frame);
    }

    // ---- Running-state operations ----

    /// Explains a revert of a call to `callee` in multi-fork mode: whether `callee` has no code on
    /// the active fork, and on which other forks it does.
    ///
    /// Handling multiple accounts/new contracts in a multifork environment can be challenging since
    /// every fork has its own standalone storage section:
    ///
    /// ```solidity
    /// function testCanDeploy() public {
    ///    vm.selectFork(mainnetFork);
    ///    // contract created while on `mainnetFork`
    ///    DummyContract dummy = new DummyContract();
    ///    // this will succeed
    ///    dummy.hello();
    ///
    ///    vm.selectFork(optimismFork);
    ///
    ///    vm.expectRevert();
    ///    // this will revert since `dummy` contract only exists on `mainnetFork`
    ///    dummy.hello();
    /// }
    /// ```
    pub fn diagnose_revert(
        &self,
        callee: Address,
        evm: &mut FoundryEvmFor<'_, FEN>,
    ) -> Option<RevertDiagnostic> {
        let active = self.active_fork_id()?;
        if self.lifecycle.forks().len() == 1 {
            // Only diagnose in multi-fork mode with more than one fork.
            return None;
        }
        if is_contract(evm.read_account_info(&callee).ok().flatten()) {
            return None;
        }
        let available_on = self
            .lifecycle
            .forks()
            .iter()
            .filter(|(id, entry)| {
                *id != active && is_contract(context_account(&entry.context, &callee))
            })
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        Some(if available_on.is_empty() {
            RevertDiagnostic::ContractDoesNotExist {
                contract: callee,
                active,
                persistent: self.is_persistent(&callee),
            }
        } else {
            RevertDiagnostic::ContractExistsOnOtherForks { contract: callee, active, available_on }
        })
    }

    /// Loads genesis `allocs` into the running transaction as journaled writes.
    pub fn load_allocs(
        &self,
        allocs: &BTreeMap<Address, GenesisAccount>,
        evm: &mut FoundryEvmFor<'_, FEN>,
    ) -> Result<(), BackendError> {
        for (address, account) in allocs {
            self.clone_account(account, address, evm)?;
        }
        Ok(())
    }

    /// Writes the code, storage, nonce and balance of `source` to `target` in the running
    /// transaction as journaled writes. Code is kept when `source` has none.
    pub fn clone_account(
        &self,
        source: &GenesisAccount,
        target: &Address,
        evm: &mut FoundryEvmFor<'_, FEN>,
    ) -> Result<(), BackendError> {
        let state = evm.state_mut();
        {
            let mut account = state.account(target)?;
            if let Some(code) = &source.code {
                account.set_code(keccak256(code), Bytecode::new_raw(code.clone()));
            }
            account.set_balance(source.balance);
            account.set_nonce(source.nonce.unwrap_or_default());
            // Touch the account so the loaded information persists if called in `setUp`.
            account.touch();
        }
        for (slot, value) in source.storage.iter().flatten() {
            state.storage_slot(target, (*slot).into())?.set((*value).into());
        }
        Ok(())
    }

    /// Sets the hash of block `number` in the running context.
    pub fn set_blockhash(&self, number: U256, hash: B256, evm: &mut FoundryEvmFor<'_, FEN>) {
        evm.state_mut().overlay_db_mut().insert_block_hash(&number, &hash);
    }

    /// Refetches `field` of `address` from the active fork's remote, after an out-of-band RPC
    /// mutation, and updates the running context with it.
    ///
    /// Balance and nonce are overridden without journaling, so they survive reverts; code is set
    /// as a journaled write.
    pub fn refresh_fork_account(
        &self,
        address: Address,
        field: ForkAccountField,
        evm: &mut FoundryEvmFor<'_, FEN>,
    ) -> Result<(), BackendError> {
        if !self.is_forked_mode() {
            return Ok(());
        }
        trace!(?address, ?field, "refresh fork account");
        let db = &self.lifecycle.active().db;
        db.evict_account(&address);
        let mut refreshed = db.clone_backing().get_account(&address)?.unwrap_or_default();
        if matches!(field, ForkAccountField::Code) && refreshed.code.is_none() {
            refreshed.code = Some(db.clone_backing().get_code_by_hash(&refreshed.code_hash)?);
        }

        self.lifecycle.materialize(evm);
        let state = evm.state_mut();
        let cache = &mut state.overlay_db_mut().cache;
        if let Some(Some(cached)) = cache.accounts.get_mut(&address) {
            field.update(cached, &refreshed);
            cached.code = None;
            if matches!(field, ForkAccountField::Code)
                && let Some(code) = &refreshed.code
            {
                cache.contracts.insert(refreshed.code_hash, code.clone());
            }
        }
        let mut account = state.account(&address)?;
        match field {
            ForkAccountField::Balance => account.override_balance(refreshed.balance),
            ForkAccountField::Nonce => account.override_nonce(refreshed.nonce),
            ForkAccountField::Code => {
                account.set_code(refreshed.code_hash, refreshed.code.unwrap_or_default())
            }
        }
        Ok(())
    }

    /// Refetches storage `slot` of `address` from the active fork's remote, after an out-of-band
    /// RPC mutation, and updates the running context with it.
    ///
    /// A slot the running transaction already loaded is updated as a journaled write.
    pub fn refresh_fork_storage(
        &self,
        address: Address,
        slot: U256,
        evm: &mut FoundryEvmFor<'_, FEN>,
    ) -> Result<(), BackendError> {
        if !self.is_forked_mode() {
            return Ok(());
        }
        trace!(?address, ?slot, "refresh fork storage");
        let db = &self.lifecycle.active().db;
        db.evict_storage(&address, &slot);
        let value = db.clone_backing().get_storage(&address, &slot)?;

        self.lifecycle.materialize(evm);
        let state = evm.state_mut();
        if let Some(storage) = state.overlay_db_mut().cache.storage.get_mut(&address)
            && storage.slots.contains_key(&slot)
        {
            storage.slots.insert(slot, value);
        }
        let storage = state.storage(&address)?;
        if storage.is_loaded(&slot) {
            storage.into_slot(slot)?.set(value);
        }
        Ok(())
    }

    // ---- Snapshots ----

    /// Captures the running context and every fork, and returns the snapshot ID.
    pub fn snapshot_state(&mut self, evm: &mut FoundryEvmFor<'_, FEN>) -> U256 {
        let id = self.lifecycle.snapshot(evm, ());
        trace!(target: "backend", "Created new snapshot {}", id);
        id
    }

    /// Restores snapshot `id`, or returns `None` if it does not exist.
    ///
    /// Logs emitted since the snapshot, persistent accounts and forks created after it survive.
    /// A test failure recorded in the reverted state sets [`Self::has_state_snapshot_failure`].
    pub fn revert_state(
        &mut self,
        id: U256,
        evm: &mut FoundryEvmFor<'_, FEN>,
        action: RevertStateSnapshotAction,
        phase: ExecutionPhase,
    ) -> Option<EnvTransition> {
        trace!(?id, "revert snapshot");
        if !self.lifecycle.has_snapshot(id) {
            warn!(target: "backend", "No snapshot to revert for {}", id);
            return None;
        }
        // DSTest contracts don't have snapshot functionality, so this slot is enough to check for
        // a failure during or before the snapshot.
        // <https://github.com/foundry-rs/foundry/issues/3055>
        if evm
            .state_mut()
            .storage_slot_untracked(&CHEATCODE_ADDRESS, &GLOBAL_FAIL_SLOT)
            .is_ok_and(|value| !value.is_zero())
        {
            self.has_state_snapshot_failure = true;
        }
        self.lifecycle.revert(evm, id, action, phase).map(|((), transition)| transition)
    }

    /// Deletes a snapshot, returning whether it existed.
    pub fn delete_state_snapshot(&mut self, id: U256) -> bool {
        self.lifecycle.delete_snapshot(id)
    }

    /// Deletes every snapshot.
    pub fn delete_state_snapshots(&mut self) {
        self.lifecycle.delete_snapshots();
    }

    // ---- Internals ----

    /// Registers an RPC fork, with `env` or the environment derived from its block.
    fn insert_fork(
        &mut self,
        fork: CreateFork,
        env: Option<EvmEnvFor<FEN>>,
    ) -> eyre::Result<LocalForkId> {
        let url = fork.url.clone();
        let opts = fork.evm_opts.clone();
        let resolved = self.forks.create_fork(fork)?;
        let env = env.unwrap_or_else(|| {
            let remote = &resolved.fork;
            self.fork_env(&opts, remote, &remote.block, self.lifecycle.active().env.spec)
        });
        Ok(self.insert_prepared_fork(url, resolved, env))
    }

    /// Registers a resolved fork without refetching its block.
    fn insert_prepared_fork(
        &mut self,
        url: String,
        resolved: ForkResult,
        env: EvmEnvFor<FEN>,
    ) -> LocalForkId {
        let ForkResult { id, backend, fork: remote } = resolved;
        let db = ForkDb::new(backend, remote.context().source_chain_id);
        let info = ForkInfo { id, url, block: Some(remote.number()), remote: Some(remote) };
        self.lifecycle.create_fork(info, db, env)
    }

    /// Returns the environment of `remote` at `block` with `spec`.
    fn fork_env(
        &self,
        opts: &EvmOpts,
        remote: &RemoteFork,
        block: &AnyRpcBlock,
        spec: SpecFor<FEN>,
    ) -> EvmEnvFor<FEN> {
        let context = remote.context();
        let chain_id = opts.chain_id_override().unwrap_or(context.execution_chain_id);
        let env =
            opts.fork_env_from_block::<FEN, AnyNetwork>(chain_id, context.source_chain_id, block);
        self.lifecycle.factory().env_with_spec(env, spec)
    }

    fn fork_source_chain_id(&self, id: LocalForkId) -> eyre::Result<ChainId> {
        self.lifecycle
            .forks()
            .context(Some(id))
            .and_then(|context| context.db.active_fork_source_chain_id())
            .ok_or_else(|| eyre::eyre!("fork `{id}` is not backed by an RPC endpoint"))
    }

    fn shared_backend(
        &self,
        id: LocalForkId,
    ) -> eyre::Result<SharedBackend<AnyNetwork, revm::context::BlockEnv>> {
        let fork_id = self.fork_info(id)?.id.clone();
        self.forks
            .get_fork(fork_id.clone())?
            .ok_or_else(|| eyre::eyre!("fork `{fork_id}` is not backed by an RPC endpoint"))
    }

    /// Fetches `transaction` and the block it executes in from fork `id`.
    fn transaction_target(
        &self,
        id: LocalForkId,
        transaction: B256,
    ) -> eyre::Result<TransactionForkTarget> {
        let backend = self.shared_backend(id)?;
        let tx = backend.get_transaction(transaction)?;
        let Some(tx_block) = tx.block_number() else {
            eyre::ensure!(
                !self.networks.is_monad(),
                "transaction {transaction} is pending and has no canonical block context"
            );
            let block = backend.get_full_block(BlockNumberOrTag::Latest)?;
            let fork_block = BlockNumHash::new(block.header.number(), block.header.hash);
            return Ok(TransactionForkTarget {
                fork_block,
                transaction: tx,
                block,
                mined: false,
                index: None,
            });
        };
        let tx_block_hash = tx
            .block_hash()
            .ok_or_else(|| eyre::eyre!("mined transaction is missing its block hash"))?;
        let block = backend.get_full_block(tx_block_hash)?;
        eyre::ensure!(
            block.header.number() == tx_block && block.header.hash == tx_block_hash,
            "transaction block changed: expected {tx_block} ({tx_block_hash}), got {} ({})",
            block.header.number(),
            block.header.hash
        );
        let BlockTransactions::Full(transactions) = &block.transactions else {
            eyre::bail!("block {} does not contain full transactions", block.header.number());
        };
        let index = transactions.iter().position(|tx| tx.tx_hash() == transaction);
        eyre::ensure!(
            index.is_some(),
            "transaction {transaction} is missing from block {}",
            block.header.number()
        );
        // Execute on the state before the block.
        let fork_block = BlockNumHash::new(
            tx_block
                .checked_sub(1)
                .ok_or_else(|| eyre::eyre!("cannot replay a transaction in the genesis block"))?,
            block.header.parent_hash(),
        );
        Ok(TransactionForkTarget { fork_block, transaction: tx, block, mined: true, index })
    }

    /// Resolves the backing database, environment, replayed accepted state and identity of fork
    /// `id` positioned right before `transaction`.
    fn prepare_roll_to_transaction(
        &self,
        id: LocalForkId,
        transaction: B256,
        spec: SpecFor<FEN>,
    ) -> eyre::Result<(RpcForkDb, EvmEnvFor<FEN>, Option<Cache>, ForkInfo)> {
        let info = self.fork_info(id)?.clone();
        let options = self.fork_options(&info.id)?;
        let target = self.transaction_target(id, transaction)?;
        let ForkResult { id: fork_id, backend, fork: remote } = self
            .forks
            .roll_fork_exact_with_bal(info.id.clone(), target.fork_block, target.mined)?;
        let source_chain_id = remote.context().source_chain_id;
        let env = self.fork_env(&options.evm_opts, &remote, &target.block, spec);
        let db = ForkDb::new(backend, source_chain_id);
        let cache = if target.mined {
            let index = target.index.expect("mined transaction has a block position");
            let context = if self.networks.is_monad() {
                Some(block_context::<FEN>(db.inner(), &target.block)?)
            } else {
                None
            };
            Some(self.replay_until(&db, env, &target.block, index, context, source_chain_id)?)
        } else {
            None
        };
        let block = Some(target.block.header.number());
        Ok((db, env, cache, ForkInfo { id: fork_id, url: info.url, block, remote: Some(remote) }))
    }

    /// Replays the transactions of `block` before `index` over `db` and returns the resulting
    /// accepted state.
    ///
    /// `context` positions network state that depends on the block, such as Monad's. System
    /// transactions are skipped, except deposits, which networks that support them execute
    /// natively.
    fn replay_until(
        &self,
        db: &impl BackingDb,
        mut env: EvmEnvFor<FEN>,
        block: &AnyRpcBlock,
        index: usize,
        context: Option<crate::evm::BlockContext<FEN>>,
        source_chain_id: ChainId,
    ) -> eyre::Result<Cache> {
        let BlockTransactions::Full(transactions) = &block.transactions else {
            eyre::bail!("block {} does not contain full transactions", block.header.number());
        };
        apply_chain_specific_tx_replay_env_changes_for_chain(&mut env, source_chain_id);

        let now = Instant::now();
        let mut evm = self.lifecycle.factory().create_evm(db.clone_backing(), env);
        for (position, rpc_tx) in transactions[..index].iter().enumerate() {
            let is_system =
                is_known_system_sender(rpc_tx.from()) || rpc_tx.ty() == SYSTEM_TRANSACTION_TYPE;
            if is_system && rpc_tx.ty() != SYSTEM_TRANSACTION_TYPE {
                // Protocol system transactions have no native representation.
                continue;
            }
            let tx = match FEN::tx_from_any_rpc(rpc_tx) {
                Ok(tx) => tx,
                // Deposits of networks without deposit support.
                Err(_) if is_system => continue,
                Err(err) => return Err(err),
            };
            match &context {
                Some(context) => context.prepare(evm.ext_mut(), position),
                None => evm.ext_mut().for_transaction(&tx),
            }
            trace!(tx=?rpc_tx.tx_hash(), "committing transaction");
            let _ = evm
                .transact(&tx)
                .map_err(handler_error)
                .wrap_err("backend: failed replaying transaction")?
                .commit();
        }
        trace!(elapsed=?now.elapsed(), count=index, "replayed transactions");
        Ok(mem::take(&mut evm.state_mut().overlay_db_mut().cache))
    }
}

/// Result of a read of the backend's accepted state.
pub type StateResult<T> = Result<T, evm2::DatabaseError>;

/// Returns the transactions of `block` and its two ancestors, fetched through `backend`.
fn block_context<FEN: FoundryEvmNetwork>(
    backend: &SharedBackend<AnyNetwork, revm::context::BlockEnv>,
    block: &AnyRpcBlock,
) -> eyre::Result<crate::evm::BlockContext<FEN>> {
    let parent = parent_block(backend, block)?;
    let grandparent =
        parent.as_ref().map(|parent| parent_block(backend, parent)).transpose()?.flatten();
    Ok(crate::evm::BlockContext::new(
        grandparent.as_ref().map(block_txs::<FEN>).transpose()?.unwrap_or_default(),
        parent.as_ref().map(block_txs::<FEN>).transpose()?.unwrap_or_default(),
        block_txs::<FEN>(block)?,
    ))
}

/// Fetches the parent of `block`, or `None` for genesis.
fn parent_block(
    backend: &SharedBackend<AnyNetwork, revm::context::BlockEnv>,
    block: &AnyRpcBlock,
) -> eyre::Result<Option<AnyRpcBlock>> {
    let parent_hash = block.header.parent_hash();
    if parent_hash.is_zero() {
        return Ok(None);
    }
    let parent = backend
        .get_full_block(parent_hash)
        .wrap_err_with(|| format!("failed to fetch ancestor block {parent_hash}"))?;
    eyre::ensure!(
        parent.header.hash == parent_hash
            && Some(parent.header.number()) == block.header.number().checked_sub(1),
        "ancestor block {parent_hash} does not match its child"
    );
    Ok(Some(parent))
}

/// Returns an account of a context that is not running.
fn context_account<T: evm2::EvmTypesHost>(
    context: &Parked<T>,
    address: &Address,
) -> Option<AccountInfo> {
    if let Some(cache) = &context.cache
        && let Some(account) = cache.accounts.get(address)
    {
        return account.clone();
    }
    context.db.clone_backing().get_account(address).ok().flatten()
}

fn is_contract(account: Option<AccountInfo>) -> bool {
    account
        .is_some_and(|account| account.code_hash != KECCAK256_EMPTY && !account.code_hash.is_zero())
}

/// Returns `account` as execution at `spec` loads it.
///
/// RPC endpoints report a missing account as empty. From Spurious Dragon on an empty account does
/// not exist (EIP-161), so it is loaded as absent; loading it as existing would, for example,
/// refund the EIP-7702 authorization of a fresh authority.
pub fn existing_account(
    spec: impl Into<evm2::SpecId>,
    account: Option<AccountInfo>,
) -> Option<AccountInfo> {
    if spec.into().enables(evm2::SpecId::SPURIOUS_DRAGON) {
        account.filter(|account| !account.is_empty())
    } else {
        account
    }
}

/// Returns the account-loading policy required by a fork source.
///
/// A recognized source chain ID is authoritative. The endpoint's execution profile is only a
/// source hint for custom chain IDs, where the chain ID cannot identify the RPC semantics.
pub fn account_fetch_policy_for_source(
    source_chain_id: ChainId,
    network_profile: NetworkConfigs,
) -> AccountFetchPolicy {
    let source_chain = Chain::from_id(source_chain_id);
    let is_tempo =
        source_chain.is_tempo() || (source_chain.named().is_none() && network_profile.is_tempo());
    if is_tempo { AccountFetchPolicy::RequireAccountInfo } else { AccountFetchPolicy::Auto }
}
