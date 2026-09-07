//! Foundry's main executor backend abstraction and implementation.

use crate::{
    FoundryBlock, FoundryChain, FoundryInspectorExt, FoundryTransaction, FromAnyRpcTransaction,
    constants::{CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, TEST_CONTRACT_ADDRESS},
    evm::{
        BlockContext, BlockEnvFor, ChainFor, EthEvmNetwork, EvmEnvFor, FoundryContextFor,
        FoundryEvmFactory, FoundryEvmNetwork, HaltReasonFor, SpecFor, TxEnvFor,
    },
    fork::{CreateFork, ForkId, ForkResult, MultiFork},
    state_snapshot::StateSnapshots,
    utils::{
        apply_chain_and_block_specific_env_changes_for_chain,
        apply_chain_specific_tx_replay_env_changes_for_chain, get_blob_base_fee_update_fraction,
    },
};
use alloy_consensus::{BlockHeader, Typed2718};
use alloy_eips::BlockNumHash;
use alloy_evm::{Evm, EvmEnv, EvmFactory, precompiles::PrecompilesMap};
use alloy_genesis::GenesisAccount;
use alloy_network::{
    AnyNetwork, AnyRpcBlock, AnyRpcTransaction, BlockResponse, Network, TransactionResponse,
};
use alloy_primitives::{Address, B256, ChainId, TxKind, U256, keccak256, map::AddressSet, uint};
use alloy_rpc_types::{BlockNumberOrTag, BlockTransactions};
use eyre::Context;
use foundry_common::{SYSTEM_TRANSACTION_TYPE, is_known_system_sender};
use foundry_evm_networks::{NetworkConfigs, apply_bsc_p256_precompile};
pub use foundry_fork_db::{
    BlockchainDb, ForkBlock, ForkBlockEnv, SharedBackend, cache::BlockchainDbMeta,
};
use revm::{
    Database, DatabaseCommit, JournalEntry,
    bytecode::Bytecode,
    context::{Block, BlockEnv, CfgEnv, ContextTr, JournalInner, Transaction},
    context_interface::{journaled_state::account::JournaledAccountTr, result::ResultAndState},
    database::{AccountState, CacheDB, DatabaseRef, EmptyDB},
    primitives::{AddressMap, HashMap as Map, KECCAK_EMPTY, Log, hardfork::SpecId},
    state::{Account, AccountInfo, EvmState, EvmStorageSlot, TransactionId},
};
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Debug,
    time::Instant,
};

mod diagnostic;
pub use diagnostic::RevertDiagnostic;

mod error;
pub use error::{BackendError, BackendResult, DatabaseError, DatabaseResult};

mod cow;
pub use cow::CowBackend;

mod in_memory_db;
pub use in_memory_db::{EmptyDBWrapper, FoundryEvmInMemoryDB, MemDb};

mod snapshot;
pub use snapshot::{BackendStateSnapshot, RevertStateSnapshotAction, StateSnapshot};

// A `revm::Database` that is used in forking mode
type ForkDB<N, B> = CacheDB<SharedBackend<N, B>>;

/// Represents a numeric `ForkId` valid only for the existence of the `Backend`.
///
/// The difference between `ForkId` and `LocalForkId` is that `ForkId` tracks pairs of `endpoint +
/// block` which can be reused by multiple tests, whereas the `LocalForkId` is unique within a test
pub type LocalForkId = U256;

/// Transaction-context update required after a fork operation.
#[cfg(feature = "monad")]
pub enum ContextUpdate<C> {
    /// The operation did not change the active chain cursor or outer journal state.
    Unchanged,
    /// The active fork cursor changed and provides replacement chain context.
    Replace(C),
    /// The outer journal changed while the active chain context remained unchanged.
    Rebase,
}

/// Transaction-context update required after a fork operation, for a given [`FoundryEvmFactory`].
///
/// Only Monad's family-owned chain context needs to observe fork operations; every other network
/// has no use for this signal, so it collapses to `()` without the `monad` feature.
#[cfg(feature = "monad")]
pub type ContextUpdateFor<F> = ContextUpdate<<F as FoundryEvmFactory>::Chain>;
#[cfg(not(feature = "monad"))]
pub type ContextUpdateFor<F> = std::marker::PhantomData<F>;

/// Represents the index of a fork in the created forks vector
/// This is used for fast lookup
type ForkLookupIndex = usize;

/// Inputs that define one transaction execution.
struct TransactionInputs<FEN: FoundryEvmNetwork> {
    evm_env: EvmEnvFor<FEN>,
    tx_env: TxEnvFor<FEN>,
    chain_context: ChainFor<FEN>,
    rpc_block_number: u64,
}

/// Environment and network configuration used while replaying transactions.
struct ReplayInputs<FEN: FoundryEvmNetwork> {
    evm_env: EvmEnvFor<FEN>,
    networks: NetworkConfigs,
}

/// Block data required to execute or position a fork at a transaction.
struct TransactionForkTarget {
    fork_block: BlockNumHash,
    transaction: AnyRpcTransaction,
    block: AnyRpcBlock,
    mined: bool,
    position: Option<TransactionPosition>,
}

/// Position of a transaction in its canonical block.
#[derive(Clone, Copy)]
struct TransactionPosition {
    index: usize,
    count: usize,
}

/// A fork roll prepared for atomic publication.
#[cfg(feature = "monad")]
struct StagedForkRoll<FEN: FoundryEvmNetwork> {
    local_id: LocalForkId,
    fork_id: ForkId,
    fork_index: ForkLookupIndex,
    fork: Fork<AnyNetwork, BlockEnvFor<FEN>>,
}

/// Canonical chain position used to reconstruct network-specific transaction context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ForkPosition {
    /// The database contains all transactions through the fork's current block.
    AfterBlock { block: BlockNumHash },
    /// The database contains the transactions before `transaction_index` in `block`.
    BeforeTransaction { block: BlockNumHash, transaction_index: usize },
}

impl ForkPosition {
    /// Returns the canonical position after committing `transaction_index`, if it immediately
    /// follows this position.
    fn after_transaction(
        self,
        block: BlockNumHash,
        parent_hash: B256,
        transaction_index: usize,
        transaction_count: usize,
    ) -> Option<Self> {
        let is_next = match self {
            Self::AfterBlock { block: previous } => {
                transaction_index == 0
                    && previous.number.checked_add(1) == Some(block.number)
                    && previous.hash == parent_hash
            }
            Self::BeforeTransaction { block: current, transaction_index: current_index } => {
                current == block && current_index == transaction_index
            }
        };
        if !is_next {
            return None;
        }
        let next_index = transaction_index.checked_add(1)?;
        if next_index > transaction_count {
            return None;
        }

        Some(if next_index == transaction_count {
            Self::AfterBlock { block }
        } else {
            Self::BeforeTransaction { block, transaction_index: next_index }
        })
    }
}

/// All accounts that will have persistent storage across fork swaps.
const DEFAULT_PERSISTENT_ACCOUNTS: [Address; 3] =
    [CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, CALLER];

/// `bytes32("failed")`, as a storage slot key into [`CHEATCODE_ADDRESS`].
///
/// Used by all `forge-std` test contracts and newer `DSTest` test contracts as a global marker for
/// a failed test.
pub const GLOBAL_FAIL_SLOT: U256 =
    uint!(0x6661696c65640000000000000000000000000000000000000000000000000000_U256);

pub type JournaledState = JournalInner<JournalEntry>;

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

/// An extension trait that allows us to easily extend the `revm::Inspector` capabilities
#[auto_impl::auto_impl(&mut)]
pub trait DatabaseExt<F: FoundryEvmFactory>:
    Database<Error = DatabaseError> + DatabaseCommit + Debug
{
    /// Creates a new state snapshot at the current point of execution.
    ///
    /// A state snapshot is associated with a new unique id that's created for the snapshot.
    /// State snapshots can be reverted: [DatabaseExt::revert_state], however, depending on the
    /// [RevertStateSnapshotAction], it will keep the snapshot alive or delete it.
    fn snapshot_state(
        &mut self,
        journaled_state: &JournaledState,
        evm_env: &EvmEnv<F::Spec, F::BlockEnv>,
    ) -> U256;

    /// Reverts the snapshot if it exists
    ///
    /// Returns `true` if the snapshot was successfully reverted, `false` if no snapshot for that id
    /// exists.
    ///
    /// **N.B.** While this reverts the state of the evm to the snapshot, it keeps new logs made
    /// since the snapshots was created. This way we can show logs that were emitted between
    /// snapshot and its revert.
    /// This will also revert any changes in the `EvmEnv` and `TxEnv` and replace them with the
    /// captured values from `Self::snapshot_state`.
    ///
    /// Depending on [RevertStateSnapshotAction] it will keep the snapshot alive or delete it.
    fn revert_state(
        &mut self,
        id: U256,
        journaled_state: &JournaledState,
        evm_env: &mut EvmEnv<F::Spec, F::BlockEnv>,
        caller: Address,
        action: RevertStateSnapshotAction,
    ) -> Option<JournaledState>;

    /// Deletes the state snapshot with the given `id`
    ///
    /// Returns `true` if the snapshot was successfully deleted, `false` if no snapshot for that id
    /// exists.
    fn delete_state_snapshot(&mut self, id: U256) -> bool;

    /// Deletes all state snapshots.
    fn delete_state_snapshots(&mut self);

    /// Creates and also selects a new fork
    ///
    /// This is basically `create_fork` + `select_fork`
    fn create_select_fork(
        &mut self,
        fork: CreateFork,
        evm_env: &mut EvmEnv<F::Spec, F::BlockEnv>,
        tx_env: &mut F::Tx,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<(LocalForkId, ContextUpdateFor<F>)> {
        let id = self.create_fork(fork)?;
        let context = self.select_fork(id, evm_env, tx_env, journaled_state)?;
        Ok((id, context))
    }

    /// Creates and also selects a new fork
    ///
    /// This is basically `create_fork` + `select_fork`
    fn create_select_fork_at_transaction(
        &mut self,
        fork: CreateFork,
        evm_env: &mut EvmEnv<F::Spec, F::BlockEnv>,
        tx_env: &mut F::Tx,
        journaled_state: &mut JournaledState,
        transaction: B256,
    ) -> eyre::Result<(LocalForkId, ContextUpdateFor<F>)> {
        let id = self.create_fork_at_transaction(fork, transaction)?;
        let context = self.select_fork(id, evm_env, tx_env, journaled_state)?;
        Ok((id, context))
    }

    /// Creates a new fork but does _not_ select it
    fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<LocalForkId>;

    /// Creates a new fork but does _not_ select it
    fn create_fork_at_transaction(
        &mut self,
        fork: CreateFork,
        transaction: B256,
    ) -> eyre::Result<LocalForkId>;

    /// Selects the fork's state
    ///
    /// This will also modify the current `EvmEnv` and `TxEnv`.
    ///
    /// **Note**: this does not change the local state, but swaps the remote state
    ///
    /// # Errors
    ///
    /// Returns an error if no fork with the given `id` exists
    fn select_fork(
        &mut self,
        id: LocalForkId,
        evm_env: &mut EvmEnv<F::Spec, F::BlockEnv>,
        tx_env: &mut F::Tx,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<ContextUpdateFor<F>>;

    /// Updates the fork to given block number.
    ///
    /// This will essentially create a new fork at the given block height.
    ///
    /// # Errors
    ///
    /// Returns an error if not matching fork was found.
    fn roll_fork(
        &mut self,
        id: Option<LocalForkId>,
        block_number: u64,
        evm_env: &mut EvmEnv<F::Spec, F::BlockEnv>,
        tx_env: &F::Tx,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<ContextUpdateFor<F>>;

    /// Updates the fork to given transaction hash
    ///
    /// This will essentially create a new fork at the block this transaction was mined and replays
    /// all transactions up until the given transaction.
    ///
    /// # Errors
    ///
    /// Returns an error if not matching fork was found.
    fn roll_fork_to_transaction(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        evm_env: &mut EvmEnv<F::Spec, F::BlockEnv>,
        tx_env: &F::Tx,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<ContextUpdateFor<F>>;

    /// Fetches the given transaction for the fork and executes it, committing the state in the DB
    fn transact(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        evm_env: EvmEnv<F::Spec, F::BlockEnv>,
        outer_tx_env: &F::Tx,
        journaled_state: &mut JournaledState,
        inspector: &mut dyn for<'db> FoundryInspectorExt<F::FoundryContext<'db>>,
    ) -> eyre::Result<ContextUpdateFor<F>>;

    /// Executes a given TransactionRequest, commits the new state to the DB
    fn transact_from_tx(
        &mut self,
        tx_env: F::Tx,
        evm_env: EvmEnv<F::Spec, F::BlockEnv>,
        journaled_state: &mut JournaledState,
        inspector: &mut dyn for<'db> FoundryInspectorExt<F::FoundryContext<'db>>,
    ) -> eyre::Result<()>;

    /// Returns transaction-position context for a synthetic transaction on the active database.
    fn chain_context_for_synthetic_transaction(&self, tx: &F::Tx) -> eyre::Result<F::Chain> {
        Ok(F::Chain::for_transaction(tx))
    }

    /// Returns the `ForkId` that's currently used in the database, if fork mode is on
    fn active_fork_id(&self) -> Option<LocalForkId>;

    /// Returns the Fork url that's currently used in the database, if fork mode is on
    fn active_fork_url(&self) -> Option<String>;

    /// Returns the active fork's current fork block number, if any.
    fn active_fork_block_number(&self) -> Option<u64> {
        None
    }

    /// Whether the database is currently in forked mode.
    fn is_forked_mode(&self) -> bool {
        self.active_fork_id().is_some()
    }

    /// Ensures that an appropriate fork exists
    ///
    /// If `id` contains a requested `Fork` this will ensure it exists.
    /// Otherwise, this returns the currently active fork.
    ///
    /// # Errors
    ///
    /// Returns an error if the given `id` does not match any forks
    ///
    /// Returns an error if no fork exists
    fn ensure_fork(&self, id: Option<LocalForkId>) -> eyre::Result<LocalForkId>;

    /// Ensures that a corresponding `ForkId` exists for the given local `id`
    fn ensure_fork_id(&self, id: LocalForkId) -> eyre::Result<&ForkId>;

    /// Handling multiple accounts/new contracts in a multifork environment can be challenging since
    /// every fork has its own standalone storage section. So this can be a common error to run
    /// into:
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
    ///
    /// If this happens (`dummy.hello()`), or more general, a call on an address that's not a
    /// contract, revm will revert without useful context. This call will check in this context if
    /// `address(dummy)` belongs to an existing contract and if not will check all other forks if
    /// the contract is deployed there.
    ///
    /// Returns a more useful error message if that's the case
    fn diagnose_revert(&self, callee: Address, evm_state: &EvmState) -> Option<RevertDiagnostic>;

    /// Loads the account allocs from the given `allocs` map into the passed [JournaledState].
    ///
    /// Returns [Ok] if all accounts were successfully inserted into the journal, [Err] otherwise.
    fn load_allocs(
        &mut self,
        allocs: &BTreeMap<Address, GenesisAccount>,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError>;

    /// Copies bytecode, storage, nonce and balance from the given genesis account to the target
    /// address.
    ///
    /// Returns [Ok] if data was successfully inserted into the journal, [Err] otherwise.
    fn clone_account(
        &mut self,
        source: &GenesisAccount,
        target: &Address,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError>;

    /// Returns true if the given account is currently marked as persistent.
    fn is_persistent(&self, acc: &Address) -> bool;

    /// Refreshes an account field changed out-of-band on the active fork in any already-loaded
    /// cache and journal entries.
    fn refresh_fork_account(
        &mut self,
        address: Address,
        field: ForkAccountField,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError>;

    /// Like [`refresh_fork_account`](Self::refresh_fork_account), but for a single storage slot.
    fn refresh_fork_storage(
        &mut self,
        address: Address,
        slot: U256,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError>;

    /// Revokes persistent status from the given account.
    fn remove_persistent_account(&mut self, account: &Address) -> bool;

    /// Marks the given account as persistent.
    fn add_persistent_account(&mut self, account: Address) -> bool;

    /// Removes persistent status from all given accounts.
    #[auto_impl(keep_default_for(&, &mut, Rc, Arc, Box))]
    fn remove_persistent_accounts(&mut self, accounts: impl IntoIterator<Item = Address>)
    where
        Self: Sized,
    {
        for acc in accounts {
            self.remove_persistent_account(&acc);
        }
    }

    /// Extends the persistent accounts with the accounts the iterator yields.
    #[auto_impl(keep_default_for(&, &mut, Rc, Arc, Box))]
    fn extend_persistent_accounts(&mut self, accounts: impl IntoIterator<Item = Address>)
    where
        Self: Sized,
    {
        for acc in accounts {
            self.add_persistent_account(acc);
        }
    }

    /// Grants cheatcode access for the given `account`
    ///
    /// Returns true if the `account` already has access
    fn allow_cheatcode_access(&mut self, account: Address) -> bool;

    /// Revokes cheatcode access for the given account
    ///
    /// Returns true if the `account` was previously allowed cheatcode access
    fn revoke_cheatcode_access(&mut self, account: &Address) -> bool;

    /// Returns `true` if the given account is allowed to execute cheatcodes
    fn has_cheatcode_access(&self, account: &Address) -> bool;

    /// Ensures that `account` is allowed to execute cheatcodes
    ///
    /// Returns an error if [`Self::has_cheatcode_access`] returns `false`
    fn ensure_cheatcode_access(&self, account: &Address) -> Result<(), BackendError> {
        if !self.has_cheatcode_access(account) {
            return Err(BackendError::NoCheats(*account));
        }
        Ok(())
    }

    /// Same as [`Self::ensure_cheatcode_access()`] but only enforces it if the backend is currently
    /// in forking mode
    fn ensure_cheatcode_access_forking_mode(&self, account: &Address) -> Result<(), BackendError> {
        if self.is_forked_mode() {
            return self.ensure_cheatcode_access(account);
        }
        Ok(())
    }

    /// Set the blockhash for a given block number.
    ///
    /// # Arguments
    ///
    /// * `number` - The block number to set the blockhash for
    /// * `hash` - The blockhash to set
    ///
    /// # Note
    ///
    /// This function mimics the EVM limits of the `blockhash` operation:
    /// - It sets the blockhash for blocks where `block.number - 256 <= number < block.number`
    /// - Setting a blockhash for the current block (number == block.number) has no effect
    /// - Setting a blockhash for future blocks (number > block.number) has no effect
    /// - Setting a blockhash for blocks older than `block.number - 256` has no effect
    fn set_blockhash(&mut self, block_number: U256, block_hash: B256);
}

/// Provides the underlying `revm::Database` implementation.
///
/// A `Backend` can be initialised in two forms:
///
/// # 1. Empty in-memory Database
/// This is the default variant: an empty `revm::Database`
///
/// # 2. Forked Database
/// A `revm::Database` that forks off a remote client
///
///
/// In addition to that we support forking manually on the fly.
/// Additional forks can be created. Each unique fork is identified by its unique `ForkId`. We treat
/// forks as unique if they have the same `(endpoint, block number)` pair.
///
/// When it comes to testing, it's intended that each contract will use its own `Backend`
/// (`Backend::clone`). This way each contract uses its own encapsulated evm state. For in-memory
/// testing, the database is just an owned `revm::InMemoryDB`.
///
/// Each `Fork`, identified by a unique id, uses completely separate storage, write operations are
/// performed only in the fork's own database, `ForkDB`.
///
/// A `ForkDB` consists of 2 halves:
///   - everything fetched from the remote is readonly
///   - all local changes (instructed by the contract) are written to the backend's `db` and don't
///     alter the state of the remote client.
///
/// # Fork swapping
///
/// Multiple "forks" can be created `Backend::create_fork()`, however only 1 can be used by the
/// `db`. However, their state can be hot-swapped by swapping the read half of `db` from one fork to
/// another.
/// When swapping forks (`Backend::select_fork()`) we also update the current `EvmEnv` of the `EVM`
/// accordingly, so that all `block.*` config values match
///
/// When another for is selected [`DatabaseExt::select_fork()`] the entire storage, including
/// `JournaledState` is swapped, but the storage of the caller's and the test contract account is
/// _always_ cloned. This way a fork has entirely separate storage but data can still be shared
/// across fork boundaries via stack and contract variables.
///
/// # Snapshotting
///
/// A snapshot of the current overall state can be taken at any point in time. A snapshot is
/// identified by a unique id that's returned when a snapshot is created. A snapshot can only be
/// reverted _once_. After a successful revert, the same snapshot id cannot be used again. Reverting
/// a snapshot replaces the current active state with the snapshot state, the snapshot is deleted
/// afterwards, as well as any snapshots taken after the reverted snapshot, (e.g.: reverting to id
/// 0x1 will delete snapshots with ids 0x1, 0x2, etc.)
///
/// **Note:** State snapshots work across fork-swaps, e.g. if fork `A` is currently active, then a
/// snapshot is created before fork `B` is selected, then fork `A` will be the active fork again
/// after reverting the snapshot.
#[must_use]
pub struct Backend<FEN: FoundryEvmNetwork = EthEvmNetwork> {
    /// Active network configuration.
    // TODO(monad-fen-dispatch): Remove this post-dispatch configuration. Extract Monad replay and
    // fork positioning into concrete Monad code, and pass family-neutral chain data directly to
    // ordinary block/environment updates.
    networks: NetworkConfigs,
    /// The access point for managing forks
    forks: MultiFork<AnyNetwork, SpecFor<FEN>, BlockEnvFor<FEN>>,
    // The default in memory db
    mem_db: FoundryEvmInMemoryDB,
    /// The journaled_state to use to initialize new forks with
    ///
    /// The way [`JournaledState`] works is, that it holds the "hot" accounts loaded from the
    /// underlying `Database` that feeds the Account and State data to the journaled_state so it
    /// can apply changes to the state while the EVM executes.
    ///
    /// In a way the `JournaledState` is something like a cache that
    /// 1. check if account is already loaded (hot)
    /// 2. if not load from the `Database` (this will then retrieve the account via RPC in forking
    ///    mode)
    ///
    /// To properly initialize we store the `JournaledState` before the first fork is selected
    /// ([`DatabaseExt::select_fork`]).
    ///
    /// This will be an empty `JournaledState`, which will be populated with persistent accounts,
    /// See [`Self::update_fork_db()`].
    fork_init_journaled_state: JournaledState,
    /// The currently active fork database
    ///
    /// If this is set, then the Backend is currently in forking mode
    active_fork_ids: Option<(LocalForkId, ForkLookupIndex)>,
    /// RPC block number exposed while executing a historical transaction in a temporary backend.
    fork_block_number_override: Option<u64>,
    /// holds additional Backend data
    inner: BackendInner<FEN>,
}

impl<FEN: FoundryEvmNetwork> Clone for Backend<FEN> {
    fn clone(&self) -> Self {
        Self {
            networks: self.networks,
            forks: self.forks.clone(),
            mem_db: self.mem_db.clone(),
            fork_init_journaled_state: self.fork_init_journaled_state.clone(),
            active_fork_ids: self.active_fork_ids,
            fork_block_number_override: self.fork_block_number_override,
            inner: self.inner.clone(),
        }
    }
}

impl<FEN: FoundryEvmNetwork> Debug for Backend<FEN> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend")
            .field("networks", &self.networks)
            .field("forks", &self.forks)
            .field("mem_db", &self.mem_db)
            .field("fork_init_journaled_state", &self.fork_init_journaled_state)
            .field("active_fork_ids", &self.active_fork_ids)
            .field("inner", &self.inner)
            .finish()
    }
}

impl<FEN: FoundryEvmNetwork> Backend<FEN> {
    /// Creates a new Backend with a spawned multi fork thread.
    ///
    /// If `fork` is `Some` this will use a `fork` database, otherwise with an in-memory
    /// database.
    pub fn spawn(fork: Option<CreateFork>) -> eyre::Result<Self> {
        Self::new(MultiFork::<AnyNetwork, SpecFor<FEN>, BlockEnvFor<FEN>>::spawn(), fork)
    }

    /// Creates a new instance of `Backend`
    ///
    /// If `fork` is `Some` this will use a `fork` database, otherwise with an in-memory
    /// database.
    ///
    /// Prefer using [`spawn`](Self::spawn) instead.
    pub fn new(
        forks: MultiFork<AnyNetwork, SpecFor<FEN>, BlockEnvFor<FEN>>,
        fork: Option<CreateFork>,
    ) -> eyre::Result<Self> {
        trace!(target: "backend", forking_mode=?fork.is_some(), "creating executor backend");
        // Note: this will take of registering the `fork`
        let persistent_accounts = AddressSet::from_iter(DEFAULT_PERSISTENT_ACCOUNTS);
        let inner = BackendInner { persistent_accounts, ..Default::default() };

        let mut backend = Self {
            networks: NetworkConfigs::default(),
            forks,
            mem_db: CacheDB::new(Default::default()),
            fork_init_journaled_state: inner.new_journaled_state(),
            active_fork_ids: None,
            fork_block_number_override: None,
            inner,
        };

        if let Some(fork) = fork {
            let ForkResult { id: fork_id, backend: fork, resolved, .. } =
                backend.forks.create_fork(fork)?;
            let context = resolved.context();
            let block = resolved.block();
            let fork_db = ForkDB::new(fork);
            let fork_ids = backend.inner.insert_new_fork(
                fork_id.clone(),
                block,
                context.source_chain_id,
                fork_db,
                backend.inner.new_journaled_state(),
            );
            backend.inner.launched_with_fork = Some((fork_id, fork_ids.0, fork_ids.1));
            backend.active_fork_ids = Some(fork_ids);
        }

        trace!(target: "backend", forking_mode=? backend.active_fork_ids.is_some(), "created executor backend");

        Ok(backend)
    }

    /// Creates a new instance of `Backend` with fork added to the fork database and sets the fork
    /// as active
    pub(crate) fn new_with_fork(
        id: &ForkId,
        mut fork: Fork<AnyNetwork, BlockEnvFor<FEN>>,
        journaled_state: JournaledState,
        networks: NetworkConfigs,
    ) -> eyre::Result<Self> {
        let mut backend = Self::spawn(None)?;
        backend.networks = networks;
        fork.journaled_state = journaled_state;
        let fork_ids = backend.inner.insert_fork(id.clone(), fork);
        backend.inner.launched_with_fork = Some((id.clone(), fork_ids.0, fork_ids.1));
        backend.active_fork_ids = Some(fork_ids);
        Ok(backend)
    }

    /// Creates a new instance with a `BackendDatabase::InMemory` cache layer for the `CacheDB`
    pub fn clone_empty(&self) -> Self {
        Self {
            networks: self.networks,
            forks: self.forks.clone(),
            mem_db: CacheDB::new(Default::default()),
            fork_init_journaled_state: self.inner.new_journaled_state(),
            active_fork_ids: None,
            fork_block_number_override: None,
            inner: Default::default(),
        }
    }

    /// Returns the active network configuration.
    pub const fn networks(&self) -> NetworkConfigs {
        self.networks
    }

    /// Sets the active network configuration.
    pub const fn set_networks(&mut self, networks: NetworkConfigs) {
        self.networks = networks;
    }

    pub fn insert_account_info(&mut self, address: Address, account: AccountInfo) {
        if let Some(db) = self.active_fork_db_mut() {
            db.insert_account_info(address, account)
        } else {
            self.mem_db.insert_account_info(address, account)
        }
    }

    /// Inserts a value on an account's storage without overriding account info
    pub fn insert_account_storage(
        &mut self,
        address: Address,
        slot: U256,
        value: U256,
    ) -> Result<(), DatabaseError> {
        if let Some(db) = self.active_fork_db_mut() {
            db.insert_account_storage(address, slot, value)
        } else {
            self.mem_db.insert_account_storage(address, slot, value)
        }
    }

    /// Completely replace an account's storage without overriding account info.
    ///
    /// When forking, this causes the backend to assume a `0` value for all
    /// unset storage slots instead of trying to fetch it.
    pub fn replace_account_storage(
        &mut self,
        address: Address,
        storage: Map<U256, U256>,
    ) -> Result<(), DatabaseError> {
        if let Some(db) = self.active_fork_db_mut() {
            db.replace_account_storage(address, storage.into_iter().collect())
        } else {
            self.mem_db.replace_account_storage(address, storage.into_iter().collect())
        }
    }

    /// Returns all snapshots created in this backend
    #[allow(clippy::type_complexity)]
    pub const fn state_snapshots(
        &self,
    ) -> &StateSnapshots<
        BackendStateSnapshot<
            BackendDatabaseSnapshot<AnyNetwork, BlockEnvFor<FEN>>,
            SpecFor<FEN>,
            BlockEnvFor<FEN>,
        >,
    > {
        &self.inner.state_snapshots
    }

    /// Sets the address of the `DSTest` contract that is being executed
    ///
    /// This will also mark the caller as persistent and remove the persistent status from the
    /// previous test contract address
    ///
    /// This will also grant cheatcode access to the test account
    pub fn set_test_contract(&mut self, acc: Address) -> &mut Self {
        trace!(?acc, "setting test account");
        self.inner.persistent_accounts.insert(acc);
        self.inner.cheatcode_access_accounts.insert(acc);
        self
    }

    /// Sets the caller address
    pub fn set_caller(&mut self, acc: Address) -> &mut Self {
        trace!(?acc, "setting caller account");
        self.inner.caller = Some(acc);
        self.inner.cheatcode_access_accounts.insert(acc);
        self
    }

    /// Sets the current spec id
    pub fn set_spec_id(&mut self, spec_id: impl Into<SpecFor<FEN>>) -> &mut Self {
        self.inner.spec_id = spec_id.into();
        self
    }

    /// Returns the set caller address
    pub const fn caller_address(&self) -> Option<Address> {
        self.inner.caller
    }

    /// Failures occurred in state snapshots are tracked when the state snapshot is reverted.
    ///
    /// If an error occurs in a restored state snapshot, the test is considered failed.
    ///
    /// This returns whether there was a reverted state snapshot that recorded an error.
    pub const fn has_state_snapshot_failure(&self) -> bool {
        self.inner.has_state_snapshot_failure
    }

    /// Sets the state snapshot failure flag.
    pub const fn set_state_snapshot_failure(&mut self, has_state_snapshot_failure: bool) {
        self.inner.has_state_snapshot_failure = has_state_snapshot_failure
    }

    /// When creating or switching forks, we update the AccountInfo of the contract
    pub(crate) fn update_fork_db(
        &self,
        active_journaled_state: &mut JournaledState,
        target_fork: &mut Fork<AnyNetwork, BlockEnvFor<FEN>>,
    ) {
        self.update_fork_db_contracts(
            self.inner.persistent_accounts.iter().copied(),
            active_journaled_state,
            target_fork,
        )
    }

    /// Merges the state of all `accounts` from the currently active db into the given `fork`
    pub(crate) fn update_fork_db_contracts(
        &self,
        accounts: impl IntoIterator<Item = Address>,
        active_journaled_state: &mut JournaledState,
        target_fork: &mut Fork<AnyNetwork, BlockEnvFor<FEN>>,
    ) {
        if let Some(db) = self.active_fork_db() {
            merge_account_data(accounts, db, active_journaled_state, target_fork)
        } else {
            merge_account_data(accounts, &self.mem_db, active_journaled_state, target_fork)
        }
    }

    /// Returns the memory db used if not in forking mode
    pub const fn mem_db(&self) -> &FoundryEvmInMemoryDB {
        &self.mem_db
    }

    /// Returns true if the `id` is currently active
    pub fn is_active_fork(&self, id: LocalForkId) -> bool {
        self.active_fork_ids.map(|(i, _)| i == id).unwrap_or_default()
    }

    /// Returns `true` if the `Backend` is currently in forking mode
    pub fn is_in_forking_mode(&self) -> bool {
        self.active_fork().is_some()
    }

    /// Returns the currently active `Fork`, if any
    pub fn active_fork(&self) -> Option<&Fork<AnyNetwork, BlockEnvFor<FEN>>> {
        self.active_fork_ids.map(|(_, idx)| self.inner.get_fork(idx))
    }

    /// Returns the currently active `Fork`, if any
    pub fn active_fork_mut(&mut self) -> Option<&mut Fork<AnyNetwork, BlockEnvFor<FEN>>> {
        self.active_fork_ids.map(|(_, idx)| self.inner.get_fork_mut(idx))
    }

    /// Returns the currently active `ForkDB`, if any
    pub fn active_fork_db(&self) -> Option<&ForkDB<AnyNetwork, BlockEnvFor<FEN>>> {
        self.active_fork().map(|f| &f.db)
    }

    /// Returns the currently active `ForkDB`, if any
    pub fn active_fork_db_mut(&mut self) -> Option<&mut ForkDB<AnyNetwork, BlockEnvFor<FEN>>> {
        self.active_fork_mut().map(|f| &mut f.db)
    }

    /// Returns the current database implementation as a `&dyn` value.
    pub fn db(&self) -> &dyn Database<Error = DatabaseError> {
        match self.active_fork_db() {
            Some(fork_db) => fork_db,
            None => &self.mem_db,
        }
    }

    /// Returns the current database implementation as a `&mut dyn` value.
    pub fn db_mut(&mut self) -> &mut dyn Database<Error = DatabaseError> {
        match self.active_fork_ids.map(|(_, idx)| &mut self.inner.get_fork_mut(idx).db) {
            Some(fork_db) => fork_db,
            None => &mut self.mem_db,
        }
    }

    /// Creates a snapshot of the currently active database
    pub(crate) fn create_db_snapshot(
        &self,
    ) -> BackendDatabaseSnapshot<AnyNetwork, BlockEnvFor<FEN>> {
        if let Some((id, idx)) = self.active_fork_ids {
            let fork = self.inner.get_fork(idx).clone();
            let fork_id = self.inner.ensure_fork_id(id).cloned().expect("Exists; qed");
            BackendDatabaseSnapshot::Forked(id, fork_id, idx, Box::new(fork))
        } else {
            BackendDatabaseSnapshot::InMemory(self.mem_db.clone())
        }
    }

    /// Since each `Fork` tracks logs separately, we need to merge them to get _all_ of them
    pub fn merged_logs(&self, mut logs: Vec<Log>) -> Vec<Log> {
        if let Some((_, active)) = self.active_fork_ids {
            let mut all_logs = Vec::with_capacity(logs.len());

            self.inner
                .forks
                .iter()
                .enumerate()
                .filter_map(|(idx, f)| f.as_ref().map(|f| (idx, f)))
                .for_each(|(idx, f)| {
                    if idx == active {
                        all_logs.append(&mut logs);
                    } else {
                        all_logs.extend(f.journaled_state.logs.clone())
                    }
                });
            return all_logs;
        }

        logs
    }

    /// Initializes settings we need to keep track of.
    ///
    /// We need to track these mainly to prevent issues when switching between different evms
    pub(crate) fn initialize(
        &mut self,
        spec_id: impl Into<SpecFor<FEN>>,
        caller: Address,
        tx_kind: TxKind,
    ) {
        self.set_caller(caller);
        self.set_spec_id(spec_id);

        let test_contract = match tx_kind {
            TxKind::Call(to) => to,
            TxKind::Create => {
                let nonce =
                    self.basic_ref(caller).map(|b| b.unwrap_or_default().nonce).unwrap_or_default();
                caller.create(nonce)
            }
        };
        self.set_test_contract(test_contract);
    }

    /// Executes the configured test call of the `env` without committing state changes.
    ///
    /// Note: in case there are any cheatcodes executed that modify the environment, this will
    /// update the given `env` with the new values.
    #[instrument(name = "inspect", level = "debug", skip_all)]
    pub fn inspect<I: for<'db> FoundryInspectorExt<FoundryContextFor<'db, FEN>>>(
        &mut self,
        evm_env: &mut EvmEnvFor<FEN>,
        tx_env: &mut TxEnvFor<FEN>,
        inspector: I,
    ) -> eyre::Result<ResultAndState<HaltReasonFor<FEN>>> {
        let chain_context = self.chain_context_for_synthetic_transaction(tx_env)?;
        self.inspect_with_context(evm_env, tx_env, chain_context, inspector)
    }

    /// Executes the configured test call with explicit network-specific context.
    #[instrument(name = "inspect", level = "debug", skip_all)]
    pub fn inspect_with_context<I: for<'db> FoundryInspectorExt<FoundryContextFor<'db, FEN>>>(
        &mut self,
        evm_env: &mut EvmEnvFor<FEN>,
        tx_env: &mut TxEnvFor<FEN>,
        chain_context: ChainFor<FEN>,
        inspector: I,
    ) -> eyre::Result<ResultAndState<HaltReasonFor<FEN>>> {
        self.initialize(evm_env.cfg_env.spec, tx_env.caller(), tx_env.kind());
        let factory = FEN::EvmFactory::default();
        let mut evm = factory.create_foundry_evm_with_inspector(
            self,
            evm_env.to_owned(),
            chain_context,
            inspector,
        );
        let res = evm.transact(tx_env.clone()).wrap_err("EVM error")?;

        *tx_env = evm.tx().clone();
        *evm_env = evm.finish().1;

        Ok(res)
    }

    /// Returns true if the address is a precompile
    pub fn is_existing_precompile(&self, addr: &Address) -> bool {
        self.inner.precompile_addresses().contains(addr)
    }

    /// Sets the initial journaled state to use when initializing forks
    #[inline]
    fn set_init_journaled_state(&mut self, journaled_state: JournaledState) {
        trace!("recording fork init journaled_state");
        self.fork_init_journaled_state = journaled_state;
    }

    /// Cleans up already loaded accounts that would be initialized without the correct data from
    /// the fork.
    ///
    /// It can happen that an account is loaded before the first fork is selected, like
    /// `getNonce(addr)`, which will load an empty account by default.
    ///
    /// This account data then would not match the account data of a fork if it exists.
    /// So when the first fork is initialized we replace these accounts with the actual account as
    /// it exists on the fork.
    fn prepare_init_journal_state(&mut self) -> Result<(), BackendError> {
        let loaded_accounts = self
            .fork_init_journaled_state
            .state
            .iter()
            .filter(|(addr, _)| {
                !self.is_existing_precompile(addr)
                    && !self.inner.persistent_accounts.contains(*addr)
            })
            .map(|(addr, _)| addr)
            .copied()
            .collect::<Vec<_>>();

        for fork in self.inner.forks_iter_mut() {
            let mut journaled_state = self.fork_init_journaled_state.clone();
            for loaded_account in loaded_accounts.iter().copied() {
                trace!(?loaded_account, "replacing account on init");
                let init_account =
                    journaled_state.state.get_mut(&loaded_account).expect("exists; qed");

                // here's an edge case where we need to check if this account has been created, in
                // which case we don't need to replace it with the account from the fork because the
                // created account takes precedence: for example contract creation in setups
                if init_account.is_created() {
                    trace!(?loaded_account, "skipping created account");
                    continue;
                }

                // otherwise we need to replace the account's info with the one from the fork's
                // database
                let fork_account = Database::basic(&mut fork.db, loaded_account)?
                    .ok_or(BackendError::MissingAccount(loaded_account))?;
                init_account.info = fork_account;
            }
            fork.journaled_state = journaled_state;
        }
        Ok(())
    }

    /// Returns the block numbers required for replaying a transaction
    fn get_block_number_and_block_for_transaction(
        &self,
        id: LocalForkId,
        transaction: B256,
    ) -> eyre::Result<TransactionForkTarget> {
        let fork = self.inner.get_fork_by_id(id)?;
        let tx = fork.backend().get_transaction(transaction)?;

        // get the block number we need to fork
        if let Some(tx_block) = tx.block_number() {
            let tx_block_hash = tx
                .block_hash()
                .ok_or_else(|| eyre::eyre!("mined transaction is missing its block hash"))?;
            let block = fork.backend().get_full_block(tx_block_hash)?;
            eyre::ensure!(
                block.header().number() == tx_block && block.header().hash == tx_block_hash,
                "transaction block changed: expected {} ({}), got {} ({})",
                tx_block,
                tx_block_hash,
                block.header().number(),
                block.header().hash
            );
            let position = if let BlockTransactions::Full(transactions) = block.transactions() {
                let index = transactions.iter().position(|tx| tx.tx_hash() == transaction);
                if self.networks.is_monad() && index.is_none() {
                    eyre::bail!(
                        "transaction {transaction:?} is missing from block {}",
                        block.header().number()
                    );
                }
                index.map(|index| TransactionPosition { index, count: transactions.len() })
            } else {
                if self.networks.is_monad() {
                    eyre::bail!(
                        "block {} does not contain full transactions",
                        block.header().number()
                    );
                }
                None
            };

            // we need to subtract 1 here because we want the state before the transaction
            // was mined
            let fork_block = BlockNumHash::new(
                tx_block.checked_sub(1).ok_or_else(|| {
                    eyre::eyre!("cannot replay a transaction in the genesis block")
                })?,
                block.header().parent_hash(),
            );
            Ok(TransactionForkTarget { fork_block, transaction: tx, block, mined: true, position })
        } else {
            if self.networks.is_monad() {
                eyre::bail!(
                    "transaction {transaction} is pending and has no canonical block context"
                );
            }
            let block = fork.backend().get_full_block(BlockNumberOrTag::Latest)?;

            let fork_block = BlockNumHash::new(block.header().number(), block.header().hash);

            Ok(TransactionForkTarget {
                fork_block,
                transaction: tx,
                block,
                mined: false,
                position: None,
            })
        }
    }

    /// Converts all transactions in a full RPC block into this backend's transaction environment.
    fn full_block_tx_envs(block: &AnyRpcBlock) -> eyre::Result<Vec<TxEnvFor<FEN>>> {
        let BlockTransactions::Full(transactions) = block.transactions() else {
            eyre::bail!("block {} does not contain full transactions", block.header().number());
        };
        transactions.iter().map(TxEnvFor::<FEN>::from_any_rpc_transaction).collect()
    }

    /// Converts a replayable transaction while preserving the established behavior of skipping
    /// system envelopes that this build cannot decode.
    fn replay_tx_env(tx: &AnyRpcTransaction) -> eyre::Result<Option<TxEnvFor<FEN>>> {
        let is_system = is_known_system_sender(tx.from()) || tx.ty() == SYSTEM_TRANSACTION_TYPE;
        if is_system {
            #[cfg(not(feature = "monad"))]
            return Ok(None);
            #[cfg(feature = "monad")]
            return Ok(TxEnvFor::<FEN>::from_any_rpc_transaction(tx).ok());
        }

        TxEnvFor::<FEN>::from_any_rpc_transaction(tx).map(Some)
    }

    /// Returns the transaction environments needed to construct exact block context.
    fn block_context_inputs_from_backend(
        backend: &SharedBackend<AnyNetwork, BlockEnvFor<FEN>>,
        block: &AnyRpcBlock,
    ) -> eyre::Result<BlockContext<FEN>> {
        let current = Self::full_block_tx_envs(block)?;

        let parent_hash = block.header().parent_hash();
        let parent_block = if parent_hash.is_zero() {
            None
        } else {
            let parent_number = block.header().number().checked_sub(1).ok_or_else(|| {
                eyre::eyre!("genesis block has non-zero parent hash {parent_hash}")
            })?;
            let parent = backend
                .get_full_block(parent_hash)
                .wrap_err_with(|| format!("failed to fetch parent block {parent_hash}"))?;
            ensure_block_identity(
                &parent,
                BlockNumHash::new(parent_number, parent_hash),
                "parent",
            )?;
            Some(parent)
        };
        let parent =
            parent_block.as_ref().map(Self::full_block_tx_envs).transpose()?.unwrap_or_default();

        let grandparent = if let Some(parent_block) = &parent_block {
            let grandparent_hash = parent_block.header().parent_hash();
            if grandparent_hash.is_zero() {
                Vec::new()
            } else {
                let block = backend.get_full_block(grandparent_hash).wrap_err_with(|| {
                    format!("failed to fetch grandparent block {grandparent_hash}")
                })?;
                let grandparent_number =
                    parent_block.header().number().checked_sub(1).ok_or_else(|| {
                        eyre::eyre!("genesis block has non-zero parent hash {grandparent_hash}")
                    })?;
                ensure_block_identity(
                    &block,
                    BlockNumHash::new(grandparent_number, grandparent_hash),
                    "grandparent",
                )?;
                Self::full_block_tx_envs(&block)?
            }
        } else {
            Vec::new()
        };

        Ok(BlockContext::new(grandparent, parent, current))
    }

    /// Returns the transaction environments needed to construct exact block context for a fork.
    fn block_context_inputs(
        &self,
        id: LocalForkId,
        block: &AnyRpcBlock,
    ) -> eyre::Result<BlockContext<FEN>> {
        let fork = self.inner.get_fork_by_id(id)?;
        Self::block_context_inputs_from_backend(fork.backend(), block)
    }

    /// Builds transaction context for `tx` at a known position in `block_context`.
    #[cfg(feature = "monad")]
    fn context_for_block_position(
        block_context: BlockContext<FEN>,
        position: ForkPosition,
        tx: &TxEnvFor<FEN>,
    ) -> eyre::Result<ChainFor<FEN>> {
        let cursor = match position {
            ForkPosition::AfterBlock { .. } => block_context.into_child(),
            ForkPosition::BeforeTransaction { transaction_index, .. } => {
                block_context.before_transaction(transaction_index)?
            }
        };
        Ok(cursor.next_transaction(tx))
    }

    /// Builds context for a synthetic transaction at a fork's current position.
    #[cfg(feature = "monad")]
    fn context_for_fork_synthetic_transaction(
        &self,
        id: LocalForkId,
        tx: &TxEnvFor<FEN>,
    ) -> eyre::Result<ChainFor<FEN>> {
        if !self.networks.is_monad() {
            return Ok(ChainFor::<FEN>::for_transaction(tx));
        }

        let fork = self.inner.get_fork_by_id(id)?;
        let (position_block, position) = match fork.position {
            position @ (ForkPosition::AfterBlock { block }
            | ForkPosition::BeforeTransaction { block, .. }) => (block, position),
        };
        let block = fork.backend().get_full_block(position_block.hash).wrap_err_with(|| {
            format!(
                "failed to fetch fork block {} ({})",
                position_block.number, position_block.hash
            )
        })?;
        ensure_block_identity(&block, position_block, "fork")?;
        let context = Self::block_context_inputs_from_backend(fork.backend(), &block)?;
        Self::context_for_block_position(context, position, tx)
    }

    /// Returns the block cursor matching the active fork's database position.
    pub fn block_context_for_synthetic_transaction(
        &self,
    ) -> eyre::Result<Option<BlockContext<FEN>>> {
        if !self.networks.is_monad() {
            return Ok(None);
        }
        let Some(id) = self.active_fork_id() else {
            return Ok(None);
        };

        let fork = self.inner.get_fork_by_id(id)?;
        let (position_block, transaction_index) = match fork.position {
            ForkPosition::AfterBlock { block } => (block, None),
            ForkPosition::BeforeTransaction { block, transaction_index } => {
                (block, Some(transaction_index))
            }
        };
        let block = fork.backend().get_full_block(position_block.hash).wrap_err_with(|| {
            format!(
                "failed to fetch active fork block {} ({})",
                position_block.number, position_block.hash
            )
        })?;
        ensure_block_identity(&block, position_block, "active fork")?;
        let context = Self::block_context_inputs_from_backend(fork.backend(), &block)?;

        match transaction_index {
            Some(index) => context.before_transaction(index).map(Some),
            None => Ok(Some(context.into_child())),
        }
    }

    /// Applies replay changes using the targeted fork's chain rather than the active environment.
    fn apply_fork_tx_replay_env_changes(
        &self,
        id: LocalForkId,
        evm_env: &mut EvmEnvFor<FEN>,
    ) -> eyre::Result<()> {
        let fork_id = self.inner.ensure_fork_id(id).cloned()?;
        let source_chain_id = self.inner.get_fork_by_id(id)?.source_chain_id;
        self.apply_fork_tx_replay_env_changes_for(&fork_id, source_chain_id, evm_env)
    }

    /// Applies replay changes using an explicitly staged fork identity.
    fn apply_fork_tx_replay_env_changes_for(
        &self,
        fork_id: &ForkId,
        source_chain_id: ChainId,
        evm_env: &mut EvmEnvFor<FEN>,
    ) -> eyre::Result<()> {
        let fork_evm_env = self
            .forks
            .get_evm_env(fork_id.clone())?
            .ok_or_else(|| eyre::eyre!("Requested fork `{fork_id}` does not exist"))?;
        evm_env.cfg_env.chain_id = fork_evm_env.cfg_env.chain_id;
        apply_chain_specific_tx_replay_env_changes_for_chain(evm_env, source_chain_id);
        Ok(())
    }

    /// Populates a rolled active fork and the outer journal at the new block state.
    fn populate_rolled_active_fork(
        fork: &mut Fork<AnyNetwork, BlockEnvFor<FEN>>,
        persistent_accounts: &AddressSet,
        caller: Option<Address>,
        journaled_state: &mut JournaledState,
    ) {
        let mut persistent_addrs = persistent_accounts.clone();
        persistent_addrs.extend(caller);

        fork.journaled_state.depth = journaled_state.depth;

        for addr in persistent_addrs {
            merge_journaled_state_data(addr, journaled_state, &mut fork.journaled_state);
        }

        for (addr, acc) in &journaled_state.state {
            if acc.is_created() && acc.is_touched() {
                merge_journaled_state_data(*addr, journaled_state, &mut fork.journaled_state);
            } else if !acc.is_created() {
                let _ = fork.journaled_state.load_account(&mut fork.db, *addr);
            }
        }

        *journaled_state = fork.journaled_state.clone();
    }

    /// Reinitializes a rolled active fork before populating its journal at the new block state.
    fn reset_rolled_active_fork(
        fork: &mut Fork<AnyNetwork, BlockEnvFor<FEN>>,
        fork_init_journaled_state: &JournaledState,
        persistent_accounts: &AddressSet,
        caller: Option<Address>,
        journaled_state: &mut JournaledState,
    ) {
        fork.journaled_state = fork_init_journaled_state.clone();
        Self::populate_rolled_active_fork(fork, persistent_accounts, caller, journaled_state);
    }

    /// Rolls a fork while preparing active transaction context before publishing the new fork.
    fn roll_fork_with_context(
        &mut self,
        id: Option<LocalForkId>,
        block_number: u64,
        evm_env: &mut EvmEnvFor<FEN>,
        tx_env: Option<&TxEnvFor<FEN>>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<ContextUpdateFor<FEN::EvmFactory>> {
        trace!(?id, ?block_number, "roll fork");
        let id = self.ensure_fork(id)?;
        let rolled = self.forks.roll_fork(self.inner.ensure_fork_id(id).cloned()?, block_number)?;
        self.apply_rolled_fork_with_context(id, rolled, evm_env, tx_env, journaled_state)
    }

    fn roll_fork_exact_with_context(
        &mut self,
        id: LocalForkId,
        block: BlockNumHash,
        evm_env: &mut EvmEnvFor<FEN>,
        tx_env: Option<&TxEnvFor<FEN>>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<ContextUpdateFor<FEN::EvmFactory>> {
        trace!(?id, ?block, "roll fork to exact block");
        let rolled = self.forks.roll_fork_exact(self.inner.ensure_fork_id(id).cloned()?, block)?;
        self.apply_rolled_fork_with_context(id, rolled, evm_env, tx_env, journaled_state)
    }

    fn apply_rolled_fork_with_context(
        &mut self,
        id: LocalForkId,
        rolled: ForkResult<AnyNetwork, SpecFor<FEN>, BlockEnvFor<FEN>>,
        evm_env: &mut EvmEnvFor<FEN>,
        _tx_env: Option<&TxEnvFor<FEN>>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<ContextUpdateFor<FEN::EvmFactory>> {
        let ForkResult { id: fork_id, backend, env: fork_env, resolved } = rolled;
        let context = resolved.context();
        let block = resolved.block();
        let _affects_active = self.is_active_fork(id);

        #[cfg(feature = "monad")]
        let context_update = if _affects_active && let Some(tx) = _tx_env {
            let chain_context = if self.networks.is_monad() {
                let block_data = backend.get_full_block(block.hash).wrap_err_with(|| {
                    format!("failed to fetch rolled fork block {} ({})", block.number, block.hash)
                })?;
                ensure_block_identity(&block_data, block, "rolled fork")?;
                let block_context = Self::block_context_inputs_from_backend(&backend, &block_data)?;
                block_context.into_child().next_transaction(tx)
            } else {
                ChainFor::<FEN>::for_transaction(tx)
            };
            ContextUpdate::Replace(chain_context)
        } else {
            ContextUpdate::Unchanged
        };
        #[cfg(not(feature = "monad"))]
        let context_update = std::marker::PhantomData;

        // Update the local mapping only after all context fetches and decoding have succeeded.
        self.inner.roll_fork(id, fork_id, block, context.source_chain_id, backend)?;

        if let Some((active_id, active_idx)) = self.active_fork_ids
            && active_id == id
        {
            let preserved_spec = evm_env.cfg_env.spec;
            *evm_env = fork_env;
            evm_env.cfg_env.set_spec_and_mainnet_gas_params(preserved_spec);

            let persistent_accounts = self.inner.persistent_accounts.clone();
            let caller = self.inner.caller;
            let active = self.inner.get_fork_mut(active_idx);
            Self::reset_rolled_active_fork(
                active,
                &self.fork_init_journaled_state,
                &persistent_accounts,
                caller,
                journaled_state,
            );
        }

        Ok(context_update)
    }

    /// Rolls a fork to a transaction while reusing one precomputed block context for replay and
    /// the final active-fork cursor.
    fn roll_fork_to_transaction_with_context(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        evm_env: &mut EvmEnvFor<FEN>,
        tx_env: Option<&TxEnvFor<FEN>>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<ContextUpdateFor<FEN::EvmFactory>> {
        if !self.networks.is_monad() {
            return self.roll_fork_to_transaction_inner(
                id,
                transaction,
                evm_env,
                tx_env,
                journaled_state,
            );
        }

        #[cfg(not(feature = "monad"))]
        unreachable!("block context is only required when Monad support is enabled");

        #[cfg(feature = "monad")]
        {
            trace!(?id, ?transaction, "roll fork to transaction");
            let id = self.ensure_fork(id)?;
            let affects_active = self.is_active_fork(id);
            let TransactionForkTarget { fork_block, block, position, .. } =
                self.get_block_number_and_block_for_transaction(id, transaction)?;
            let position = position.expect("Monad transaction target includes canonical position");
            let block_context = self.block_context_inputs(id, &block)?;
            let context_update = if affects_active && let Some(tx) = tx_env {
                let fork_position = ForkPosition::BeforeTransaction {
                    block: BlockNumHash::new(block.header().number(), block.header().hash),
                    transaction_index: position.index,
                };
                ContextUpdate::Replace(Self::context_for_block_position(
                    block_context.clone(),
                    fork_position,
                    tx,
                )?)
            } else if affects_active {
                ContextUpdate::Unchanged
            } else {
                ContextUpdate::Rebase
            };

            let current_fork_id = self.inner.ensure_fork_id(id).cloned()?;
            let ForkResult { id: fork_id, backend, env: fork_env, resolved } =
                self.forks.roll_fork_exact(current_fork_id, fork_block)?;
            let staged_fork_journaled_state = if affects_active {
                self.fork_init_journaled_state.clone()
            } else {
                self.inner.get_fork_by_id(id)?.journaled_state.clone()
            };
            let mut staged_fork = self.inner.stage_fork_roll(
                id,
                fork_id,
                fork_block,
                resolved.context().source_chain_id,
                backend,
                staged_fork_journaled_state,
            )?;
            let mut staged_evm_env = evm_env.clone();
            let mut staged_journaled_state = journaled_state.clone();

            if affects_active {
                let preserved_spec = staged_evm_env.cfg_env.spec;
                staged_evm_env = fork_env;
                staged_evm_env.cfg_env.set_spec_and_mainnet_gas_params(preserved_spec);
                Self::populate_rolled_active_fork(
                    &mut staged_fork.fork,
                    &self.inner.persistent_accounts,
                    self.inner.caller,
                    &mut staged_journaled_state,
                );
            }

            update_env_block::<AnyNetwork, _, _>(
                &mut staged_evm_env,
                &block,
                staged_fork.fork.source_chain_id,
                self.networks,
            );
            let mut replay_env = staged_evm_env.clone();
            self.apply_fork_tx_replay_env_changes_for(
                &staged_fork.fork_id,
                staged_fork.fork.source_chain_id,
                &mut replay_env,
            )?;
            let target = Self::replay_until(
                &mut staged_fork.fork,
                ReplayInputs { evm_env: replay_env, networks: self.networks },
                &block,
                Some(&block_context),
                transaction,
                &mut staged_journaled_state,
                &self.inner.persistent_accounts,
            )?;
            eyre::ensure!(
                target.is_some(),
                "transaction {transaction:?} is missing from block {}",
                block.header().number()
            );
            staged_fork.fork.position = ForkPosition::BeforeTransaction {
                block: BlockNumHash::new(block.header().number(), block.header().hash),
                transaction_index: position.index,
            };

            // Once the handler update is enqueued, all remaining publication is infallible.
            self.forks
                .update_block_env(staged_fork.fork_id.clone(), staged_evm_env.block_env.clone())?;
            self.inner.publish_fork_roll(staged_fork);
            *evm_env = staged_evm_env;
            *journaled_state = staged_journaled_state;
            Ok(context_update)
        }
    }

    /// Performs a transaction-level roll on the provided backend, environment, and journal.
    fn roll_fork_to_transaction_inner(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        evm_env: &mut EvmEnvFor<FEN>,
        _tx_env: Option<&TxEnvFor<FEN>>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<ContextUpdateFor<FEN::EvmFactory>> {
        trace!(?id, ?transaction, "roll fork to transaction");
        let id = self.ensure_fork(id)?;
        let _affects_active = self.is_active_fork(id);

        let TransactionForkTarget { fork_block, block, mined, position, .. } =
            self.get_block_number_and_block_for_transaction(id, transaction)?;
        let block_context = if self.networks.is_monad() {
            Some(self.block_context_inputs(id, &block)?)
        } else {
            None
        };
        #[cfg(feature = "monad")]
        let context_update = if _affects_active && let Some(tx) = _tx_env {
            let chain_context = if let Some(context) = &block_context {
                let fork_position = ForkPosition::BeforeTransaction {
                    block: BlockNumHash::new(block.header().number(), block.header().hash),
                    transaction_index: position
                        .expect("Monad transaction target includes canonical position")
                        .index,
                };
                Self::context_for_block_position(context.clone(), fork_position, tx)?
            } else {
                ChainFor::<FEN>::for_transaction(tx)
            };
            ContextUpdate::Replace(chain_context)
        } else if _affects_active {
            ContextUpdate::Unchanged
        } else {
            ContextUpdate::Rebase
        };
        #[cfg(not(feature = "monad"))]
        let context_update = std::marker::PhantomData;

        // The parent roll must not prepare an intermediate synthetic context.
        self.roll_fork_exact_with_context(id, fork_block, evm_env, None, journaled_state)?;

        let source_chain_id = self.inner.get_fork_by_id(id)?.source_chain_id;
        update_env_block::<AnyNetwork, _, _>(evm_env, &block, source_chain_id, self.networks);

        let mut replay_env = evm_env.clone();
        self.apply_fork_tx_replay_env_changes(id, &mut replay_env)?;
        let persistent_accounts = self.inner.persistent_accounts.clone();
        let target = if mined {
            let fork = self.inner.get_fork_by_id_mut(id)?;
            Self::replay_until(
                fork,
                ReplayInputs { evm_env: replay_env, networks: self.networks },
                &block,
                block_context.as_ref(),
                transaction,
                journaled_state,
                &persistent_accounts,
            )?
        } else {
            None
        };
        if target.is_some()
            && let Some(position) = position
        {
            self.inner.get_fork_by_id_mut(id)?.position = ForkPosition::BeforeTransaction {
                block: BlockNumHash::new(block.header().number(), block.header().hash),
                transaction_index: position.index,
            };
        }

        // Replay uses the staged environment directly. Publish the handler environment only after
        // all fallible replay and cursor updates have succeeded.
        let fork_id = self.inner.ensure_fork_id(id).cloned().expect("fork was resolved above");
        let _ = self.forks.update_block_env(fork_id, evm_env.block_env.clone());

        Ok(context_update)
    }

    /// Replays all the transactions at the forks current block that were mined before the `tx`
    ///
    /// Returns the _unmined_ transaction that corresponds to the given `tx_hash`
    fn replay_until(
        fork: &mut Fork<AnyNetwork, BlockEnvFor<FEN>>,
        replay: ReplayInputs<FEN>,
        full_block: &AnyRpcBlock,
        block_context: Option<&BlockContext<FEN>>,
        tx_hash: B256,
        journaled_state: &mut JournaledState,
        persistent_accounts: &AddressSet,
    ) -> eyre::Result<Option<AnyRpcTransaction>> {
        let ReplayInputs { evm_env, networks } = replay;
        trace!(?tx_hash, "replay until transaction");
        eyre::ensure!(
            !networks.is_monad() || block_context.is_some(),
            "block context is required to replay transactions for this network"
        );

        let BlockTransactions::Full(transactions) = full_block.transactions() else {
            eyre::bail!(
                "block {} does not contain full transactions",
                full_block.header().number()
            );
        };
        let Some(target_index) = transactions.iter().position(|tx| tx.tx_hash() == tx_hash) else {
            return Ok(None);
        };
        if networks.is_monad() {
            eyre::ensure!(
                fork.position
                    .after_transaction(
                        BlockNumHash::new(full_block.header().number(), full_block.header().hash),
                        full_block.header().parent_hash(),
                        0,
                        transactions.len(),
                    )
                    .is_some(),
                "block {} does not immediately follow the active fork position",
                full_block.header().number()
            );
        }
        let target_tx = transactions[target_index].clone();
        let factory = FEN::EvmFactory::default();
        let mut txs_to_replay = Vec::with_capacity(target_index);
        for (index, tx) in transactions[..target_index].iter().enumerate() {
            let Some(tx_env) = Self::replay_tx_env(tx)? else { continue };
            let is_system = is_known_system_sender(tx.from()) || tx.ty() == SYSTEM_TRANSACTION_TYPE;
            txs_to_replay.push((index, tx.clone(), tx_env, is_system));
        }

        // Replay all preceding transactions against a cloned ForkDB.
        if !txs_to_replay.is_empty() {
            let now = Instant::now();

            // Clone the fork's CacheDB once. The underlying SharedBackend is Arc-backed,
            // so only the local cache layer is actually duplicated.
            let chain_id = evm_env.cfg_env.chain_id;
            let timestamp = evm_env.block_env.timestamp().saturating_to();
            let mut replay_db = fork.db.clone();

            if let Some(context) = block_context {
                for (index, tx, tx_env, is_system) in &txs_to_replay {
                    let chain_context = context.transaction(*index);
                    let mut evm =
                        factory.create_evm_with_context(replay_db, evm_env.clone(), chain_context);
                    inject_replay_precompiles(networks, evm.precompiles_mut(), chain_id, timestamp);
                    trace!(tx=?tx.tx_hash(), "committing transaction");
                    let result = if *is_system {
                        #[cfg(feature = "monad")]
                        let Some(result) = factory
                            .try_transact_system_replay(&mut evm, tx_env)
                            .wrap_err("backend: failed replaying system transaction")?
                        else {
                            replay_db = evm.into_db();
                            continue;
                        };
                        #[cfg(not(feature = "monad"))]
                        unreachable!("system transactions are filtered without Monad support");
                        #[cfg(feature = "monad")]
                        result
                    } else {
                        evm.transact(tx_env.clone())
                            .wrap_err("backend: failed replaying transaction")?
                    };
                    evm.db_mut().commit(result.state);
                    replay_db = evm.into_db();
                }
            } else {
                let mut evm = factory.create_evm(replay_db, evm_env);
                inject_replay_precompiles(networks, evm.precompiles_mut(), chain_id, timestamp);
                for (_, tx, tx_env, is_system) in &txs_to_replay {
                    trace!(tx=?tx.tx_hash(), "committing transaction");
                    let result = if *is_system {
                        #[cfg(feature = "monad")]
                        let Some(result) = factory
                            .try_transact_system_replay(&mut evm, tx_env)
                            .wrap_err("backend: failed replaying system transaction")?
                        else {
                            continue;
                        };
                        #[cfg(not(feature = "monad"))]
                        unreachable!("system transactions are filtered without Monad support");
                        #[cfg(feature = "monad")]
                        result
                    } else {
                        evm.transact(tx_env.clone())
                            .wrap_err("backend: failed replaying transaction")?
                    };
                    evm.db_mut().commit(result.state);
                }
                replay_db = evm.into_db();
            }

            // Extract the DB back and replace the fork's database with the replayed state.
            fork.db = replay_db;

            // Refresh journaled states from the updated database, preserving persistent
            // accounts (cheatcode address, CREATE2 deployer, test contract, etc.).
            fork.refresh_journaled_states(journaled_state, persistent_accounts)?;

            trace!(elapsed=?now.elapsed(), count=txs_to_replay.len(), "replayed transactions");
        }

        Ok(Some(target_tx))
    }
}

fn ensure_block_identity(
    block: &AnyRpcBlock,
    expected: BlockNumHash,
    relation: &str,
) -> eyre::Result<()> {
    eyre::ensure!(
        block.header().number() == expected.number && block.header().hash == expected.hash,
        "{relation} block changed: expected {} ({}), got {} ({})",
        expected.number,
        expected.hash,
        block.header().number(),
        block.header().hash
    );
    Ok(())
}

impl<FEN: FoundryEvmNetwork> DatabaseExt<FEN::EvmFactory> for Backend<FEN> {
    fn chain_context_for_synthetic_transaction(
        &self,
        tx: &TxEnvFor<FEN>,
    ) -> eyre::Result<ChainFor<FEN>> {
        self.block_context_for_synthetic_transaction()?.map_or_else(
            || Ok(ChainFor::<FEN>::for_transaction(tx)),
            |context| Ok(context.next_transaction(tx)),
        )
    }

    fn snapshot_state(
        &mut self,
        journaled_state: &JournaledState,
        evm_env: &EvmEnvFor<FEN>,
    ) -> U256 {
        trace!("create snapshot");
        let id = self.inner.state_snapshots.insert(BackendStateSnapshot::new(
            self.create_db_snapshot(),
            journaled_state.clone(),
            evm_env.clone(),
        ));
        trace!(target: "backend", "Created new snapshot {}", id);
        id
    }

    fn revert_state(
        &mut self,
        id: U256,
        current_state: &JournaledState,
        evm_env: &mut EvmEnvFor<FEN>,
        caller: Address,
        action: RevertStateSnapshotAction,
    ) -> Option<JournaledState> {
        trace!(?id, "revert snapshot");
        if let Some(mut snapshot) = self.inner.state_snapshots.remove_at(id) {
            // Re-insert snapshot to persist it
            if action.is_keep() {
                self.inner.state_snapshots.insert_at(snapshot.clone(), id);
            }

            // https://github.com/foundry-rs/foundry/issues/3055
            // Check if an error occurred either during or before the snapshot.
            // DSTest contracts don't have snapshot functionality, so this slot is enough to check
            // for failure here.
            if let Some(account) = current_state.state.get(&CHEATCODE_ADDRESS)
                && let Some(slot) = account.storage.get(&GLOBAL_FAIL_SLOT)
                && !slot.present_value.is_zero()
            {
                self.set_state_snapshot_failure(true);
            }

            // merge additional logs
            snapshot.merge(current_state);
            let BackendStateSnapshot { db, mut journaled_state, snap_evm_env } = snapshot;
            match db {
                BackendDatabaseSnapshot::InMemory(mem_db) => {
                    self.mem_db = mem_db;
                }
                BackendDatabaseSnapshot::Forked(id, fork_id, idx, mut fork) => {
                    // there might be the case where the snapshot was created during `setUp` with
                    // another caller, so we need to ensure the caller account is present in the
                    // journaled state and database
                    journaled_state.state.entry(caller).or_insert_with(|| {
                        let caller_account = current_state
                            .state
                            .get(&caller)
                            .map(|acc| acc.info.clone())
                            .unwrap_or_default();

                        if !fork.db.cache.accounts.contains_key(&caller) {
                            // update the caller account which is required by the evm
                            fork.db.insert_account_info(caller, caller_account.clone());
                        }
                        caller_account.into()
                    });
                    self.inner.revert_state_snapshot(id, fork_id, idx, *fork);
                    self.active_fork_ids = Some((id, idx))
                }
            }

            *evm_env = snap_evm_env;
            trace!(target: "backend", "Reverted snapshot {}", id);

            Some(journaled_state)
        } else {
            warn!(target: "backend", "No snapshot to revert for {}", id);
            None
        }
    }

    fn delete_state_snapshot(&mut self, id: U256) -> bool {
        self.inner.state_snapshots.remove_at(id).is_some()
    }

    fn delete_state_snapshots(&mut self) {
        self.inner.state_snapshots.clear()
    }

    fn create_fork(&mut self, create_fork: CreateFork) -> eyre::Result<LocalForkId> {
        trace!("create fork");
        let ForkResult { id: fork_id, backend: fork, resolved, .. } =
            self.forks.create_fork(create_fork)?;
        let context = resolved.context();
        let block = resolved.block();
        let fork_db = ForkDB::new(fork);
        let (id, _) = self.inner.insert_new_fork(
            fork_id,
            block,
            context.source_chain_id,
            fork_db,
            self.fork_init_journaled_state.clone(),
        );
        Ok(id)
    }

    fn create_fork_at_transaction(
        &mut self,
        fork: CreateFork,
        transaction: B256,
    ) -> eyre::Result<LocalForkId> {
        trace!(?transaction, "create fork at transaction");
        let id = self.create_fork(fork)?;
        let fork_id = self.ensure_fork_id(id).cloned()?;
        let mut evm_env = self
            .forks
            .get_evm_env(fork_id)?
            .ok_or_else(|| eyre::eyre!("Requested fork `{}` does not exist", id))?;

        // we still need to roll to the transaction, but we only need an empty dummy state since we
        // don't need to update the active journaled state yet
        self.roll_fork_to_transaction_with_context(
            Some(id),
            transaction,
            &mut evm_env,
            None,
            &mut self.inner.new_journaled_state(),
        )?;

        Ok(id)
    }

    /// Select an existing fork by id.
    /// When switching forks we copy the shared state
    fn select_fork(
        &mut self,
        id: LocalForkId,
        evm_env: &mut EvmEnvFor<FEN>,
        tx_env: &mut TxEnvFor<FEN>,
        active_journaled_state: &mut JournaledState,
    ) -> eyre::Result<ContextUpdateFor<FEN::EvmFactory>> {
        trace!(?id, "select fork");
        if self.is_active_fork(id) {
            // nothing to do
            #[cfg(feature = "monad")]
            return Ok(ContextUpdate::Unchanged);
            #[cfg(not(feature = "monad"))]
            return Ok(std::marker::PhantomData);
        }

        #[cfg(feature = "monad")]
        let chain_context = self.context_for_fork_synthetic_transaction(id, tx_env)?;

        // Update block number and timestamp of active fork (if any) with current env values,
        // in order to preserve values changed by using `roll` and `warp` cheatcodes.
        if let Some(active_fork_id) = self.active_fork_id() {
            self.forks.update_block(
                self.ensure_fork_id(active_fork_id).cloned()?,
                evm_env.block_env.number(),
                evm_env.block_env.timestamp(),
            )?;
        }

        let fork_id = self.ensure_fork_id(id).cloned()?;
        let idx = self.inner.ensure_fork_index(&fork_id)?;
        let fork_evm_env = self
            .forks
            .get_evm_env(fork_id)?
            .ok_or_else(|| eyre::eyre!("Requested fork `{}` does not exist", id))?;

        // If we're currently in forking mode we need to update the journaled_state to this point,
        // this ensures the changes performed while the fork was active are recorded
        if let Some(active) = self.active_fork_mut() {
            active.journaled_state = active_journaled_state.clone();

            let caller = tx_env.caller();
            let caller_account = active.journaled_state.state.get(&caller).cloned();
            let target_fork = self.inner.get_fork_mut(idx);

            // depth 0 will be the default value when the fork was created
            if target_fork.journaled_state.depth == 0 {
                // Initialize caller with its fork info
                if let Some(mut acc) = caller_account {
                    let fork_account = Database::basic(&mut target_fork.db, caller)?
                        .ok_or(BackendError::MissingAccount(caller))?;

                    acc.info = fork_account;
                    target_fork.journaled_state.state.insert(caller, acc);
                }
            }
        } else {
            // this is the first time a fork is selected. This means up to this point all changes
            // are made in a single `JournaledState`, for example after a `setup` that only created
            // different forks. Since the `JournaledState` is valid for all forks until the
            // first fork is selected, we need to update it for all forks and use it as init state
            // for all future forks

            self.set_init_journaled_state(active_journaled_state.clone());
            self.prepare_init_journal_state()?;

            // Make sure that the next created fork has a depth of 0.
            self.fork_init_journaled_state.depth = 0;
        }

        {
            // update the shared state and track
            let mut fork = self.inner.take_fork(idx);

            // Make sure all persistent accounts on the newly selected fork reflect same state as
            // the active db / previous fork.
            // This can get out of sync when multiple forks are created on test `setUp`, then a
            // fork is selected and persistent contract is changed. If first action in test is to
            // select a different fork, then the persistent contract state won't reflect changes
            // done in `setUp` for the other fork.
            // See <https://github.com/foundry-rs/foundry/issues/10296> and <https://github.com/foundry-rs/foundry/issues/10552>.
            let persistent_accounts = self.inner.persistent_accounts.clone();
            if let Some(db) = self.active_fork_db_mut() {
                for addr in persistent_accounts {
                    let Ok(db_account) = db.load_account(addr) else { continue };

                    let Some(fork_account) = fork.journaled_state.state.get_mut(&addr) else {
                        continue;
                    };

                    for (key, val) in &db_account.storage {
                        if let Some(fork_storage) = fork_account.storage.get_mut(key) {
                            fork_storage.present_value = *val;
                        }
                    }
                }
            }

            // since all forks handle their state separately, the depth can drift
            // this is a handover where the target fork starts at the same depth where it was
            // selected. This ensures that there are no gaps in depth which would
            // otherwise cause issues with the tracer
            fork.journaled_state.depth = active_journaled_state.depth;

            // another edge case where a fork is created and selected during setup with not
            // necessarily the same caller as for the test, however we must always
            // ensure that fork's state contains the current sender
            let caller = tx_env.caller();
            fork.journaled_state.state.entry(caller).or_insert_with(|| {
                let caller_account = active_journaled_state
                    .state
                    .get(&caller)
                    .map(|acc| acc.info.clone())
                    .unwrap_or_default();

                if !fork.db.cache.accounts.contains_key(&caller) {
                    // update the caller account which is required by the evm
                    fork.db.insert_account_info(caller, caller_account.clone());
                }
                caller_account.into()
            });

            self.update_fork_db(active_journaled_state, &mut fork);

            // insert the fork back
            self.inner.set_fork(idx, fork);
        }

        self.active_fork_ids = Some((id, idx));
        // Update current environment with environment of newly selected fork.
        // Preserve the configured spec (evm_version) from the current environment — the fork's
        // evm_env is built with SPEC::default() and must not override the user's hardfork setting.
        let preserved_spec = evm_env.cfg_env.spec;
        tx_env.set_chain_id(Some(fork_evm_env.cfg_env.chain_id));
        *evm_env = fork_evm_env;
        evm_env.cfg_env.set_spec_and_mainnet_gas_params(preserved_spec);

        #[cfg(feature = "monad")]
        return Ok(ContextUpdate::Replace(chain_context));
        #[cfg(not(feature = "monad"))]
        Ok(std::marker::PhantomData)
    }

    /// This is effectively the same as [`Self::create_select_fork()`] but updating an existing
    /// [ForkId] that is mapped to the [LocalForkId]
    fn roll_fork(
        &mut self,
        id: Option<LocalForkId>,
        block_number: u64,
        evm_env: &mut EvmEnvFor<FEN>,
        tx_env: &TxEnvFor<FEN>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<ContextUpdateFor<FEN::EvmFactory>> {
        self.roll_fork_with_context(id, block_number, evm_env, Some(tx_env), journaled_state)
    }

    fn roll_fork_to_transaction(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        evm_env: &mut EvmEnvFor<FEN>,
        tx_env: &TxEnvFor<FEN>,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<ContextUpdateFor<FEN::EvmFactory>> {
        self.roll_fork_to_transaction_with_context(
            id,
            transaction,
            evm_env,
            Some(tx_env),
            journaled_state,
        )
    }

    fn transact(
        &mut self,
        maybe_id: Option<LocalForkId>,
        transaction: B256,
        mut evm_env: EvmEnvFor<FEN>,
        _outer_tx_env: &TxEnvFor<FEN>,
        journaled_state: &mut JournaledState,
        inspector: &mut dyn for<'db> FoundryInspectorExt<
            <FEN::EvmFactory as FoundryEvmFactory>::FoundryContext<'db>,
        >,
    ) -> eyre::Result<ContextUpdateFor<FEN::EvmFactory>> {
        trace!(?maybe_id, ?transaction, "execute transaction");
        let persistent_accounts = self.inner.persistent_accounts.clone();
        let id = self.ensure_fork(maybe_id)?;
        let _affects_active = self.is_active_fork(id);
        let fork_id = self.ensure_fork_id(id).cloned()?;

        // This is a bit ambiguous because the user wants to transact an arbitrary transaction in
        // the current context, but we're assuming the user wants to transact the transaction as it
        // was mined. Usually this is used in a combination of a fork at the transaction's parent
        // transaction in the block and then the transaction is transacted:
        // <https://github.com/foundry-rs/foundry/issues/6538>
        // So we modify the env to match the transaction's block.
        let TransactionForkTarget { transaction: tx, block, position, .. } =
            self.get_block_number_and_block_for_transaction(id, transaction)?;
        let tx_env = TxEnvFor::<FEN>::from_any_rpc_transaction(&tx)?;
        let source_chain_id = self.inner.get_fork_by_id(id)?.source_chain_id;
        update_env_block::<AnyNetwork, _, _>(&mut evm_env, &block, source_chain_id, self.networks);
        self.apply_fork_tx_replay_env_changes(id, &mut evm_env)?;

        let block_context = if self.networks.is_monad() {
            Some(self.block_context_inputs(id, &block)?)
        } else {
            None
        };
        let chain_context = if let Some(context) = &block_context {
            context.transaction(
                position.expect("Monad transaction target includes canonical position").index,
            )
        } else {
            ChainFor::<FEN>::for_transaction(&tx_env)
        };

        let next_position = if block_context.is_some() {
            let position = position.expect("Monad transaction target includes canonical position");
            Some(
                self.inner
                    .get_fork_by_id(id)?
                    .position
                    .after_transaction(
                        BlockNumHash::new(block.header().number(), block.header().hash),
                        block.header().parent_hash(),
                        position.index,
                        position.count,
                    )
                    .ok_or_else(|| {
                        eyre::eyre!(
                            "transaction {transaction} does not immediately follow the active \
                             fork position"
                        )
                    })?,
            )
        } else {
            None
        };
        #[cfg(feature = "monad")]
        let context_update = if _affects_active {
            ContextUpdate::Replace(if let Some(context) = block_context {
                Self::context_for_block_position(
                    context,
                    next_position.expect("block context has a next position"),
                    _outer_tx_env,
                )?
            } else {
                ChainFor::<FEN>::for_transaction(_outer_tx_env)
            })
        } else {
            ContextUpdate::Rebase
        };
        #[cfg(not(feature = "monad"))]
        let context_update = std::marker::PhantomData;

        let fork = self.inner.get_fork_by_id_mut(id)?;
        commit_transaction::<FEN>(
            TransactionInputs {
                evm_env,
                tx_env,
                chain_context,
                rpc_block_number: block.header().number(),
            },
            journaled_state,
            fork,
            &fork_id,
            self.networks,
            &persistent_accounts,
            inspector,
        )?;
        if let Some(position) = next_position {
            fork.position = position;
        }
        Ok(context_update)
    }

    fn transact_from_tx(
        &mut self,
        tx_env: TxEnvFor<FEN>,
        evm_env: EvmEnvFor<FEN>,
        journaled_state: &mut JournaledState,
        inspector: &mut dyn for<'db> FoundryInspectorExt<
            <FEN::EvmFactory as FoundryEvmFactory>::FoundryContext<'db>,
        >,
    ) -> eyre::Result<()> {
        trace!("execute signed transaction");

        self.commit(journaled_state.state.clone());

        let res = {
            let mut db = self.clone();
            let depth = journaled_state.depth + 1;
            let factory = FEN::EvmFactory::default();
            let chain_context = self.chain_context_for_synthetic_transaction(&tx_env)?;
            let mut evm =
                factory.create_foundry_nested_evm(&mut db, evm_env, chain_context, inspector);
            evm.journal_inner_mut().depth = depth;
            evm.transact_raw(tx_env)?
        };

        self.commit(res.state);
        update_state(&mut journaled_state.state, self, None)?;

        Ok(())
    }

    fn active_fork_id(&self) -> Option<LocalForkId> {
        self.active_fork_ids.map(|(id, _)| id)
    }

    fn active_fork_url(&self) -> Option<String> {
        let fork = self.inner.issued_local_fork_ids.get(&self.active_fork_id()?)?;
        self.forks.get_fork_url(fork.clone()).ok()?
    }

    fn active_fork_block_number(&self) -> Option<u64> {
        if let Some(block_number) = self.fork_block_number_override {
            return Some(block_number);
        }
        let fork = self.inner.get_fork_by_id(self.active_fork_id()?).ok()?;
        Some(match fork.position {
            ForkPosition::AfterBlock { block } | ForkPosition::BeforeTransaction { block, .. } => {
                block.number
            }
        })
    }

    fn ensure_fork(&self, id: Option<LocalForkId>) -> eyre::Result<LocalForkId> {
        if let Some(id) = id {
            if self.inner.issued_local_fork_ids.contains_key(&id) {
                return Ok(id);
            }
            eyre::bail!("Requested fork `{}` does not exist", id);
        }
        if let Some(id) = self.active_fork_id() {
            Ok(id)
        } else {
            eyre::bail!("No fork active");
        }
    }

    fn ensure_fork_id(&self, id: LocalForkId) -> eyre::Result<&ForkId> {
        self.inner.ensure_fork_id(id)
    }

    fn diagnose_revert(&self, callee: Address, evm_state: &EvmState) -> Option<RevertDiagnostic> {
        let active_id = self.active_fork_id()?;
        let active_fork = self.active_fork()?;

        if self.inner.forks.len() == 1 {
            // we only want to provide additional diagnostics here when in multifork mode with > 1
            // forks
            return None;
        }

        if !active_fork.is_contract(callee) && !is_contract_in_state(evm_state, callee) {
            // no contract for `callee` available on current fork, check if available on other forks
            let mut available_on = Vec::new();
            for (id, fork) in self.inner.forks_iter().filter(|(id, _)| *id != active_id) {
                trace!(?id, address=?callee, "checking if account exists");
                if fork.is_contract(callee) {
                    available_on.push(id);
                }
            }

            return if available_on.is_empty() {
                Some(RevertDiagnostic::ContractDoesNotExist {
                    contract: callee,
                    active: active_id,
                    persistent: self.is_persistent(&callee),
                })
            } else {
                // likely user error: called a contract that's not available on active fork but is
                // present other forks
                Some(RevertDiagnostic::ContractExistsOnOtherForks {
                    contract: callee,
                    active: active_id,
                    available_on,
                })
            };
        }
        None
    }

    /// Loads the account allocs from the given `allocs` map into the passed [JournaledState].
    ///
    /// Returns [Ok] if all accounts were successfully inserted into the journal, [Err] otherwise.
    fn load_allocs(
        &mut self,
        allocs: &BTreeMap<Address, GenesisAccount>,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError> {
        // Loop through all of the allocs defined in the map and commit them to the journal.
        for (addr, acc) in allocs {
            self.clone_account(acc, addr, journaled_state)?;
        }

        Ok(())
    }

    /// Copies bytecode, storage, nonce and balance from the given genesis account to the target
    /// address.
    ///
    /// Returns [Ok] if data was successfully inserted into the journal, [Err] otherwise.
    fn clone_account(
        &mut self,
        source: &GenesisAccount,
        target: &Address,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError> {
        // Fetch the account from the journaled state. Will create a new account if it does
        // not already exist.
        let mut state_acc = journaled_state.load_account_mut(self, *target)?;

        // Set the account's bytecode and code hash, if the `bytecode` field is present.
        if let Some(bytecode) = source.code.as_ref() {
            let bytecode_hash = keccak256(bytecode);
            let bytecode = Bytecode::new_raw(bytecode.0.clone().into());
            state_acc.set_code(bytecode_hash, bytecode);
        }

        // Set the account's balance.
        state_acc.set_balance(source.balance);

        // Set the account's storage, if the `storage` field is present.
        if let Some(acc) = journaled_state.state.get_mut(target) {
            if let Some(storage) = source.storage.as_ref() {
                for (slot, value) in storage {
                    let slot = U256::from_be_bytes(slot.0);
                    acc.storage.insert(
                        slot,
                        EvmStorageSlot::new_changed(
                            acc.storage.get(&slot).map(|s| s.present_value).unwrap_or_default(),
                            U256::from_be_bytes(value.0),
                            TransactionId::ZERO,
                        ),
                    );
                }
            }

            // Set the account's nonce.
            acc.info.nonce = source.nonce.unwrap_or_default();
        };

        // Touch the account to ensure the loaded information persists if called in `setUp`.
        journaled_state.touch(*target);

        Ok(())
    }

    fn add_persistent_account(&mut self, account: Address) -> bool {
        trace!(?account, "add persistent account");
        self.inner.persistent_accounts.insert(account)
    }

    fn refresh_fork_account(
        &mut self,
        address: Address,
        field: ForkAccountField,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError> {
        let Some(fork) = self.active_fork_mut() else { return Ok(()) };
        trace!(?address, ?field, "refresh fork account");
        fork.db.db.data().accounts.write().remove(&address);

        let cache_modified = fork
            .db
            .cache
            .accounts
            .get(&address)
            .is_some_and(|account| account.account_state != AccountState::None);
        if !cache_modified {
            fork.db.cache.accounts.remove(&address);
        }

        if !cache_modified
            && !journaled_state.state.contains_key(&address)
            && !fork.journaled_state.state.contains_key(&address)
        {
            return Ok(());
        }

        let mut refreshed = DatabaseRef::basic_ref(&fork.db.db, address)?.unwrap_or_default();
        if matches!(field, ForkAccountField::Code) {
            fork.db.insert_contract(&mut refreshed);
        }
        if let Some(cached) = fork.db.cache.accounts.get_mut(&address) {
            field.update(&mut cached.info, &refreshed);
            if cached.account_state == AccountState::NotExisting && !refreshed.is_empty() {
                cached.account_state = AccountState::None;
            }
        }
        if let Some(journaled_account) = journaled_state.state.get_mut(&address) {
            field.update(&mut journaled_account.info, &refreshed);
        }
        if let Some(journaled_account) = fork.journaled_state.state.get_mut(&address) {
            field.update(&mut journaled_account.info, &refreshed);
        }
        Ok(())
    }

    fn refresh_fork_storage(
        &mut self,
        address: Address,
        slot: U256,
        journaled_state: &mut JournaledState,
    ) -> Result<(), BackendError> {
        let Some(fork) = self.active_fork_mut() else { return Ok(()) };
        trace!(?address, ?slot, "refresh fork storage");
        if let Some(storage) = fork.db.db.data().storage.write().get_mut(&address) {
            storage.remove(&slot);
        }
        let cache_modified = fork
            .db
            .cache
            .accounts
            .get(&address)
            .is_some_and(|account| account.account_state != AccountState::None);
        if !cache_modified && let Some(account) = fork.db.cache.accounts.get_mut(&address) {
            account.storage.remove(&slot);
        }

        let outer_loaded = journaled_state
            .state
            .get(&address)
            .is_some_and(|account| account.storage.contains_key(&slot));
        let fork_loaded = fork
            .journaled_state
            .state
            .get(&address)
            .is_some_and(|account| account.storage.contains_key(&slot));
        if !cache_modified && !outer_loaded && !fork_loaded {
            return Ok(());
        }

        let value = DatabaseRef::storage_ref(&fork.db.db, address, slot)?;
        if let Some(account) = fork.db.cache.accounts.get_mut(&address) {
            account.storage.insert(slot, value);
            if account.account_state == AccountState::NotExisting && !value.is_zero() {
                account.account_state = AccountState::None;
            }
        }
        if let Some(storage) = journaled_state
            .state
            .get_mut(&address)
            .and_then(|account| account.storage.get_mut(&slot))
        {
            storage.present_value = value;
        }
        if let Some(storage) = fork
            .journaled_state
            .state
            .get_mut(&address)
            .and_then(|account| account.storage.get_mut(&slot))
        {
            storage.present_value = value;
        }
        Ok(())
    }

    fn remove_persistent_account(&mut self, account: &Address) -> bool {
        trace!(?account, "remove persistent account");
        self.inner.persistent_accounts.remove(account)
    }

    fn is_persistent(&self, acc: &Address) -> bool {
        self.inner.persistent_accounts.contains(acc)
    }

    fn allow_cheatcode_access(&mut self, account: Address) -> bool {
        trace!(?account, "allow cheatcode access");
        self.inner.cheatcode_access_accounts.insert(account)
    }

    fn revoke_cheatcode_access(&mut self, account: &Address) -> bool {
        trace!(?account, "revoke cheatcode access");
        self.inner.cheatcode_access_accounts.remove(account)
    }

    fn has_cheatcode_access(&self, account: &Address) -> bool {
        self.inner.cheatcode_access_accounts.contains(account)
    }

    fn set_blockhash(&mut self, block_number: U256, block_hash: B256) {
        if let Some(db) = self.active_fork_db_mut() {
            db.cache.block_hashes.insert(block_number.saturating_to(), block_hash);
        } else {
            self.mem_db.cache.block_hashes.insert(block_number.saturating_to(), block_hash);
        }
    }
}

impl<FEN: FoundryEvmNetwork> DatabaseRef for Backend<FEN> {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        if let Some(db) = self.active_fork_db() {
            db.basic_ref(address)
        } else {
            Ok(self.mem_db.basic_ref(address)?)
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if let Some(db) = self.active_fork_db() {
            db.code_by_hash_ref(code_hash)
        } else {
            Ok(self.mem_db.code_by_hash_ref(code_hash)?)
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        if let Some(db) = self.active_fork_db() {
            DatabaseRef::storage_ref(db, address, index)
        } else {
            Ok(DatabaseRef::storage_ref(&self.mem_db, address, index)?)
        }
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        if let Some(db) = self.active_fork_db() {
            db.block_hash_ref(number)
        } else {
            Ok(self.mem_db.block_hash_ref(number)?)
        }
    }
}

impl<FEN: FoundryEvmNetwork> DatabaseCommit for Backend<FEN> {
    fn commit(&mut self, changes: AddressMap<Account>) {
        if let Some(db) = self.active_fork_db_mut() {
            db.commit(changes)
        } else {
            self.mem_db.commit(changes)
        }
    }
}

impl<FEN: FoundryEvmNetwork> Database for Backend<FEN> {
    type Error = DatabaseError;
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        if let Some(db) = self.active_fork_db_mut() {
            Ok(db.basic(address)?)
        } else {
            Ok(self.mem_db.basic(address)?)
        }
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if let Some(db) = self.active_fork_db_mut() {
            Ok(db.code_by_hash(code_hash)?)
        } else {
            Ok(self.mem_db.code_by_hash(code_hash)?)
        }
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        if let Some(db) = self.active_fork_db_mut() {
            Ok(Database::storage(db, address, index)?)
        } else {
            Ok(Database::storage(&mut self.mem_db, address, index)?)
        }
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        if let Some(db) = self.active_fork_db_mut() {
            Ok(db.block_hash(number)?)
        } else {
            Ok(self.mem_db.block_hash(number)?)
        }
    }
}

/// Variants of a [revm::Database]
#[derive(Clone, Debug)]
pub enum BackendDatabaseSnapshot<N: Network, B: ForkBlockEnv = BlockEnv> {
    /// Simple in-memory [revm::Database]
    InMemory(FoundryEvmInMemoryDB),
    /// Contains the entire forking mode database
    Forked(LocalForkId, ForkId, ForkLookupIndex, Box<Fork<N, B>>),
}

/// Represents a fork
#[derive(Clone, Debug)]
pub struct Fork<N: Network, B: ForkBlockEnv = BlockEnv> {
    db: ForkDB<N, B>,
    journaled_state: JournaledState,
    source_chain_id: ChainId,
    position: ForkPosition,
}

impl<N: Network, B: ForkBlockEnv> Fork<N, B> {
    /// Returns a reference to the underlying [`SharedBackend`].
    pub const fn backend(&self) -> &SharedBackend<N, B> {
        &self.db.db
    }

    /// Returns true if the account is a contract
    pub fn is_contract(&self, acc: Address) -> bool {
        if let Ok(Some(acc)) = self.db.basic_ref(acc)
            && acc.code_hash != KECCAK_EMPTY
        {
            return true;
        }
        is_contract_in_state(&self.journaled_state.state, acc)
    }

    /// Refreshes the given journaled state and the fork's own journaled state from the
    /// database, preserving persistent accounts.
    fn refresh_journaled_states(
        &mut self,
        journaled_state: &mut JournaledState,
        persistent_accounts: &AddressSet,
    ) -> Result<(), BackendError> {
        update_state(&mut journaled_state.state, &mut self.db, Some(persistent_accounts))?;
        update_state(&mut self.journaled_state.state, &mut self.db, Some(persistent_accounts))?;
        Ok(())
    }
}

/// Container type for various Backend related data
pub struct BackendInner<FEN: FoundryEvmNetwork> {
    /// Stores the `ForkId` of the fork the `Backend` launched with from the start.
    ///
    /// In other words if [`Backend::spawn()`] was called with a `CreateFork` command, to launch
    /// directly in fork mode, this holds the corresponding fork identifier of this fork.
    pub launched_with_fork: Option<(ForkId, LocalForkId, ForkLookupIndex)>,
    /// This tracks numeric fork ids and the `ForkId` used by the handler.
    ///
    /// This is necessary, because there can be multiple `Backends` associated with a single
    /// `ForkId` which is only a pair of endpoint + block. Since an existing fork can be
    /// modified (e.g. `roll_fork`), but this should only affect the fork that's unique for the
    /// test and not the `ForkId`
    ///
    /// This ensures we can treat forks as unique from the context of a test, so rolling to another
    /// is basically creating(or reusing) another `ForkId` that's then mapped to the previous
    /// issued _local_ numeric identifier, that remains constant, even if the underlying fork
    /// backend changes.
    pub issued_local_fork_ids: HashMap<LocalForkId, ForkId>,
    /// tracks all the created forks
    /// Contains the index of the corresponding `ForkDB` in the `forks` vec
    pub created_forks: HashMap<ForkId, ForkLookupIndex>,
    /// Holds all created fork databases
    // Note: data is stored in an `Option` so we can remove it without reshuffling
    pub forks: Vec<Option<Fork<AnyNetwork, BlockEnvFor<FEN>>>>,
    /// Contains state snapshots made at a certain point
    #[allow(clippy::type_complexity)]
    pub state_snapshots: StateSnapshots<
        BackendStateSnapshot<
            BackendDatabaseSnapshot<AnyNetwork, BlockEnvFor<FEN>>,
            SpecFor<FEN>,
            BlockEnvFor<FEN>,
        >,
    >,
    /// Tracks whether there was a failure in a snapshot that was reverted
    ///
    /// The Test contract contains a bool variable that is set to true when an `assert` function
    /// failed. When a snapshot is reverted, it reverts the state of the evm, but we still want
    /// to know if there was an `assert` that failed after the snapshot was taken so that we can
    /// check if the test function passed all asserts even across snapshots. When a snapshot is
    /// reverted we get the _current_ `revm::JournaledState` which contains the state that we can
    /// check if the `_failed` variable is set,
    /// additionally
    pub has_state_snapshot_failure: bool,
    /// Tracks the caller of the test function
    pub caller: Option<Address>,
    /// Tracks numeric identifiers for forks
    pub next_fork_id: LocalForkId,
    /// All accounts that should be kept persistent when switching forks.
    /// This means all accounts stored here _don't_ use a separate storage section on each fork
    /// instead the use only one that's persistent across fork swaps.
    pub persistent_accounts: AddressSet,
    /// The configured spec id
    pub spec_id: SpecFor<FEN>,
    /// All accounts that are allowed to execute cheatcodes
    pub cheatcode_access_accounts: AddressSet,
}

impl<FEN: FoundryEvmNetwork> Clone for BackendInner<FEN> {
    fn clone(&self) -> Self {
        Self {
            launched_with_fork: self.launched_with_fork.clone(),
            issued_local_fork_ids: self.issued_local_fork_ids.clone(),
            created_forks: self.created_forks.clone(),
            forks: self.forks.clone(),
            state_snapshots: self.state_snapshots.clone(),
            has_state_snapshot_failure: self.has_state_snapshot_failure,
            caller: self.caller,
            next_fork_id: self.next_fork_id,
            persistent_accounts: self.persistent_accounts.clone(),
            spec_id: self.spec_id,
            cheatcode_access_accounts: self.cheatcode_access_accounts.clone(),
        }
    }
}

impl<FEN: FoundryEvmNetwork> Debug for BackendInner<FEN> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendInner")
            .field("launched_with_fork", &self.launched_with_fork)
            .field("issued_local_fork_ids", &self.issued_local_fork_ids)
            .field("created_forks", &self.created_forks)
            .field("forks", &self.forks)
            .field("state_snapshots", &self.state_snapshots)
            .field("has_state_snapshot_failure", &self.has_state_snapshot_failure)
            .field("caller", &self.caller)
            .field("next_fork_id", &self.next_fork_id)
            .field("persistent_accounts", &self.persistent_accounts)
            .field("spec_id", &self.spec_id)
            .field("cheatcode_access_accounts", &self.cheatcode_access_accounts)
            .finish()
    }
}

impl<FEN: FoundryEvmNetwork> BackendInner<FEN> {
    pub fn ensure_fork_id(&self, id: LocalForkId) -> eyre::Result<&ForkId> {
        self.issued_local_fork_ids
            .get(&id)
            .ok_or_else(|| eyre::eyre!("No matching fork found for {}", id))
    }

    pub fn ensure_fork_index(&self, id: &ForkId) -> eyre::Result<ForkLookupIndex> {
        self.created_forks
            .get(id)
            .copied()
            .ok_or_else(|| eyre::eyre!("No matching fork found for {}", id))
    }

    pub fn ensure_fork_index_by_local_id(&self, id: LocalForkId) -> eyre::Result<ForkLookupIndex> {
        self.ensure_fork_index(self.ensure_fork_id(id)?)
    }

    /// Returns the underlying fork mapped to the index
    #[track_caller]
    fn get_fork(&self, idx: ForkLookupIndex) -> &Fork<AnyNetwork, BlockEnvFor<FEN>> {
        debug_assert!(idx < self.forks.len(), "fork lookup index must exist");
        self.forks[idx].as_ref().unwrap()
    }

    /// Returns the underlying fork mapped to the index
    #[track_caller]
    fn get_fork_mut(&mut self, idx: ForkLookupIndex) -> &mut Fork<AnyNetwork, BlockEnvFor<FEN>> {
        debug_assert!(idx < self.forks.len(), "fork lookup index must exist");
        self.forks[idx].as_mut().unwrap()
    }

    /// Returns the underlying fork corresponding to the id
    #[track_caller]
    fn get_fork_by_id_mut(
        &mut self,
        id: LocalForkId,
    ) -> eyre::Result<&mut Fork<AnyNetwork, BlockEnvFor<FEN>>> {
        let idx = self.ensure_fork_index_by_local_id(id)?;
        Ok(self.get_fork_mut(idx))
    }

    /// Returns the underlying fork corresponding to the id
    #[track_caller]
    fn get_fork_by_id(&self, id: LocalForkId) -> eyre::Result<&Fork<AnyNetwork, BlockEnvFor<FEN>>> {
        let idx = self.ensure_fork_index_by_local_id(id)?;
        Ok(self.get_fork(idx))
    }

    /// Removes the fork
    fn take_fork(&mut self, idx: ForkLookupIndex) -> Fork<AnyNetwork, BlockEnvFor<FEN>> {
        debug_assert!(idx < self.forks.len(), "fork lookup index must exist");
        self.forks[idx].take().unwrap()
    }

    fn set_fork(&mut self, idx: ForkLookupIndex, fork: Fork<AnyNetwork, BlockEnvFor<FEN>>) {
        self.forks[idx] = Some(fork)
    }

    /// Returns an iterator over Forks
    pub fn forks_iter(
        &self,
    ) -> impl Iterator<Item = (LocalForkId, &Fork<AnyNetwork, BlockEnvFor<FEN>>)> + '_ {
        self.issued_local_fork_ids
            .iter()
            .map(|(id, fork_id)| (*id, self.get_fork(self.created_forks[fork_id])))
    }

    /// Returns a mutable iterator over all Forks
    pub fn forks_iter_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut Fork<AnyNetwork, BlockEnvFor<FEN>>> + '_ {
        self.forks.iter_mut().filter_map(|f| f.as_mut())
    }

    /// Reverts the entire fork database
    pub fn revert_state_snapshot(
        &mut self,
        id: LocalForkId,
        fork_id: ForkId,
        idx: ForkLookupIndex,
        fork: Fork<AnyNetwork, BlockEnvFor<FEN>>,
    ) {
        self.created_forks.insert(fork_id.clone(), idx);
        self.issued_local_fork_ids.insert(id, fork_id);
        self.set_fork(idx, fork)
    }

    /// Updates the fork and the local mapping and returns the new index for the `fork_db`
    pub fn update_fork_mapping(
        &mut self,
        id: LocalForkId,
        fork_id: ForkId,
        block: BlockNumHash,
        source_chain_id: ChainId,
        db: ForkDB<AnyNetwork, BlockEnvFor<FEN>>,
        journaled_state: JournaledState,
    ) -> ForkLookupIndex {
        let idx = self.forks.len();
        self.issued_local_fork_ids.insert(id, fork_id.clone());
        self.created_forks.insert(fork_id, idx);

        let fork = Fork {
            db,
            journaled_state,
            source_chain_id,
            position: ForkPosition::AfterBlock { block },
        };
        self.forks.push(Some(fork));
        idx
    }

    pub fn roll_fork(
        &mut self,
        id: LocalForkId,
        new_fork_id: ForkId,
        block: BlockNumHash,
        source_chain_id: ChainId,
        backend: SharedBackend<AnyNetwork, BlockEnvFor<FEN>>,
    ) -> eyre::Result<ForkLookupIndex> {
        let fork_id = self.ensure_fork_id(id)?;
        let idx = self.ensure_fork_index(fork_id)?;

        if let Some(active) = self.forks[idx].as_mut() {
            // Initialize a new `ForkDB` while retaining persistent account data.
            let mut new_db = ForkDB::new(backend);
            for addr in self.persistent_accounts.iter().copied() {
                merge_db_account_data(addr, &active.db, &mut new_db);
            }
            active.db = new_db;
            active.source_chain_id = source_chain_id;
            active.position = ForkPosition::AfterBlock { block };
        }
        self.issued_local_fork_ids.insert(id, new_fork_id.clone());
        self.created_forks.insert(new_fork_id, idx);
        Ok(idx)
    }

    /// Prepares a replacement for one fork without changing its local mapping or database.
    #[cfg(feature = "monad")]
    fn stage_fork_roll(
        &self,
        id: LocalForkId,
        new_fork_id: ForkId,
        block: BlockNumHash,
        source_chain_id: ChainId,
        backend: SharedBackend<AnyNetwork, BlockEnvFor<FEN>>,
        journaled_state: JournaledState,
    ) -> eyre::Result<StagedForkRoll<FEN>> {
        let fork_id = self.ensure_fork_id(id)?;
        let idx = self.ensure_fork_index(fork_id)?;
        let current = self.get_fork(idx);

        // Initialize a new `ForkDB` with persistent account data and the prepared journal. The
        // live fork remains untouched until publication.
        let mut new_db = ForkDB::new(backend);
        for addr in self.persistent_accounts.iter().copied() {
            merge_db_account_data(addr, &current.db, &mut new_db);
        }

        Ok(StagedForkRoll {
            local_id: id,
            fork_id: new_fork_id,
            fork_index: idx,
            fork: Fork {
                db: new_db,
                journaled_state,
                source_chain_id,
                position: ForkPosition::AfterBlock { block },
            },
        })
    }

    /// Atomically publishes a previously prepared fork replacement.
    #[cfg(feature = "monad")]
    fn publish_fork_roll(&mut self, staged: StagedForkRoll<FEN>) -> ForkLookupIndex {
        let StagedForkRoll { local_id, fork_id, fork_index, fork } = staged;
        self.set_fork(fork_index, fork);
        self.issued_local_fork_ids.insert(local_id, fork_id.clone());
        self.created_forks.insert(fork_id, fork_index);
        fork_index
    }

    /// Inserts a _new_ `ForkDB` and issues a new local fork identifier
    ///
    /// Also returns the index where the `ForDB` is stored
    pub fn insert_new_fork(
        &mut self,
        fork_id: ForkId,
        block: BlockNumHash,
        source_chain_id: ChainId,
        db: ForkDB<AnyNetwork, BlockEnvFor<FEN>>,
        journaled_state: JournaledState,
    ) -> (LocalForkId, ForkLookupIndex) {
        self.insert_fork(
            fork_id,
            Fork {
                db,
                journaled_state,
                source_chain_id,
                position: ForkPosition::AfterBlock { block },
            },
        )
    }

    /// Inserts an existing fork while preserving its exact chain position.
    fn insert_fork(
        &mut self,
        fork_id: ForkId,
        fork: Fork<AnyNetwork, BlockEnvFor<FEN>>,
    ) -> (LocalForkId, ForkLookupIndex) {
        let idx = self.forks.len();
        self.created_forks.insert(fork_id.clone(), idx);
        let id = self.next_id();
        self.issued_local_fork_ids.insert(id, fork_id);
        self.forks.push(Some(fork));
        (id, idx)
    }

    fn next_id(&mut self) -> U256 {
        let id = self.next_fork_id;
        self.next_fork_id += U256::from(1);
        id
    }

    /// Returns the number of issued ids
    pub fn len(&self) -> usize {
        self.issued_local_fork_ids.len()
    }

    /// Returns true if no forks are issued
    pub fn is_empty(&self) -> bool {
        self.issued_local_fork_ids.is_empty()
    }

    pub fn precompile_addresses(&self) -> AddressSet {
        let evm = FEN::EvmFactory::default().create_evm(
            EmptyDB::default(),
            EvmEnv::new(CfgEnv::new_with_spec(self.spec_id), Default::default()),
        );
        evm.precompiles().addresses().copied().collect()
    }

    /// Returns a new, empty, `JournaledState` with set precompiles
    pub fn new_journaled_state(&self) -> JournaledState {
        let mut journal = {
            let mut journal_inner = JournalInner::new();
            journal_inner.set_spec_id(self.spec_id.into());
            journal_inner
        };
        let precompile_addresses = self.precompile_addresses();
        journal.warm_addresses.set_precompile_addresses(&precompile_addresses);
        journal
    }
}

impl<FEN: FoundryEvmNetwork> Default for BackendInner<FEN> {
    fn default() -> Self {
        Self {
            launched_with_fork: None,
            issued_local_fork_ids: Default::default(),
            created_forks: Default::default(),
            forks: vec![],
            state_snapshots: Default::default(),
            has_state_snapshot_failure: false,
            caller: None,
            next_fork_id: Default::default(),
            persistent_accounts: Default::default(),
            spec_id: SpecFor::<FEN>::default(),
            // grant the cheatcode,default test and caller address access to execute cheatcodes
            // itself
            cheatcode_access_accounts: AddressSet::from_iter([
                CHEATCODE_ADDRESS,
                TEST_CONTRACT_ADDRESS,
                CALLER,
            ]),
        }
    }
}

/// Clones the data of the given `accounts` from the `active` database into the `fork_db`
/// This includes the data held in storage (`CacheDB`) and kept in the `JournaledState`.
pub(crate) fn merge_account_data<ExtDB: DatabaseRef, N: Network, B: ForkBlockEnv>(
    accounts: impl IntoIterator<Item = Address>,
    active: &CacheDB<ExtDB>,
    active_journaled_state: &mut JournaledState,
    target_fork: &mut Fork<N, B>,
) {
    for addr in accounts {
        merge_db_account_data(addr, active, &mut target_fork.db);
        merge_journaled_state_data(addr, active_journaled_state, &mut target_fork.journaled_state);
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
fn merge_db_account_data<ExtDB: DatabaseRef, N: Network, B: ForkBlockEnv>(
    addr: Address,
    active: &CacheDB<ExtDB>,
    fork_db: &mut ForkDB<N, B>,
) {
    trace!(?addr, "merging database data");

    let Some(acc) = active.cache.accounts.get(&addr) else { return };

    // port contract cache over
    if let Some(code) = active.cache.contracts.get(&acc.info.code_hash) {
        trace!("merging contract cache");
        fork_db.cache.contracts.insert(acc.info.code_hash, code.clone());
    }

    // port account storage over
    use std::collections::hash_map::Entry;
    match fork_db.cache.accounts.entry(addr) {
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

/// Returns true of the address is a contract
fn is_contract_in_state(evm_state: &EvmState, acc: Address) -> bool {
    evm_state.get(&acc).map(|acc| acc.info.code_hash != KECCAK_EMPTY).unwrap_or_default()
}

/// Updates the evm env's block with the block's data
fn update_env_block<N: Network, SPEC: Into<SpecId> + Copy, BLOCK: FoundryBlock>(
    evm_env: &mut EvmEnv<SPEC, BLOCK>,
    block: &N::BlockResponse,
    source_chain_id: ChainId,
    networks: NetworkConfigs,
) {
    let header = block.header();
    let block_env = &mut evm_env.block_env;
    block_env.set_timestamp(U256::from(header.timestamp()));
    block_env.set_beneficiary(header.beneficiary());
    block_env.set_difficulty(header.difficulty());
    block_env.set_prevrandao(header.mix_hash());
    block_env.set_basefee(header.base_fee_per_gas().unwrap_or_default());
    block_env.set_gas_limit(header.gas_limit());
    block_env.set_number(U256::from(header.number()));

    if let Some(excess_blob_gas) = header.excess_blob_gas() {
        evm_env.block_env.set_blob_excess_gas_and_price(
            excess_blob_gas,
            get_blob_base_fee_update_fraction(evm_env.cfg_env.chain_id, header.timestamp()),
        );
    }

    apply_chain_and_block_specific_env_changes_for_chain::<N, _, _>(
        evm_env,
        block,
        source_chain_id,
        networks,
    );
}

/// Executes the given transaction and commits state changes to the database _and_ the journaled
/// state, with an inspector.
fn commit_transaction<FEN: FoundryEvmNetwork>(
    transaction: TransactionInputs<FEN>,
    journaled_state: &mut JournaledState,
    fork: &mut Fork<AnyNetwork, BlockEnvFor<FEN>>,
    fork_id: &ForkId,
    networks: NetworkConfigs,
    persistent_accounts: &AddressSet,
    inspector: &mut dyn for<'db> FoundryInspectorExt<
        <FEN::EvmFactory as FoundryEvmFactory>::FoundryContext<'db>,
    >,
) -> eyre::Result<()> {
    let TransactionInputs { evm_env, tx_env, chain_context, rpc_block_number } = transaction;
    let now = Instant::now();
    let res = {
        let fork = fork.clone();
        let journaled_state = journaled_state.clone();
        let depth = journaled_state.depth;
        let mut db: Backend<FEN> =
            Backend::new_with_fork(fork_id, fork, journaled_state, networks)?;
        db.fork_block_number_override = Some(rpc_block_number);

        let mut evm = FEN::EvmFactory::default().create_foundry_nested_evm(
            &mut db,
            evm_env,
            chain_context,
            inspector,
        );
        evm.journal_inner_mut().depth = depth + 1;
        evm.transact_raw(tx_env).wrap_err("backend: failed committing transaction")?
    };
    trace!(elapsed = ?now.elapsed(), "transacted transaction");

    apply_state_changeset(res.state, journaled_state, fork, persistent_accounts)?;
    Ok(())
}

/// Helper method which updates data in the state with the data from the database.
/// Does not change state for persistent accounts (for roll fork to transaction and transact).
pub fn update_state<DB: Database>(
    state: &mut EvmState,
    db: &mut DB,
    persistent_accounts: Option<&AddressSet>,
) -> Result<(), DB::Error> {
    for (addr, acc) in state.iter_mut() {
        if persistent_accounts.is_none_or(|accounts| !accounts.contains(addr)) {
            acc.info = db.basic(*addr)?.unwrap_or_default();
            for (key, val) in &mut acc.storage {
                val.present_value = db.storage(*addr, *key)?;
            }
        }
    }

    Ok(())
}

/// Applies the changeset of a transaction to the active journaled state and also commits it in the
/// forked db
fn apply_state_changeset<N: Network, B: ForkBlockEnv>(
    state: EvmState,
    journaled_state: &mut JournaledState,
    fork: &mut Fork<N, B>,
    persistent_accounts: &AddressSet,
) -> Result<(), BackendError> {
    // Refresh cloned journals against a cloned database so a failed read cannot publish only part
    // of the transaction state.
    let mut staged_db = fork.db.clone();
    let mut staged_journaled_state = journaled_state.clone();
    let mut staged_fork_journaled_state = fork.journaled_state.clone();
    staged_db.commit(state);
    update_state(&mut staged_journaled_state.state, &mut staged_db, Some(persistent_accounts))?;
    update_state(
        &mut staged_fork_journaled_state.state,
        &mut staged_db,
        Some(persistent_accounts),
    )?;

    fork.db = staged_db;
    *journaled_state = staged_journaled_state;
    fork.journaled_state = staged_fork_journaled_state;
    Ok(())
}

fn inject_replay_precompiles(
    networks: NetworkConfigs,
    precompiles: &mut PrecompilesMap,
    chain_id: ChainId,
    timestamp: u64,
) {
    networks.inject_precompiles(precompiles);
    apply_bsc_p256_precompile(precompiles, chain_id, timestamp);
}

#[cfg(test)]
mod tests {
    use super::{
        Fork, ForkAccountField, apply_state_changeset, ensure_block_identity, update_env_block,
    };
    use crate::{
        backend::{Backend, DatabaseExt, ForkPosition},
        evm::EthEvmNetwork,
        fork::CreateFork,
        opts::EvmOpts,
    };
    use alloy_consensus::transaction::Recovered;
    use alloy_eips::BlockNumHash;
    use alloy_evm::EvmEnv;
    use alloy_network::{
        AnyHeader, AnyNetwork, AnyRpcBlock, AnyRpcHeader, AnyRpcTransaction, AnyTxEnvelope,
        AnyTxType, TransactionBuilder, UnknownTxEnvelope, UnknownTypedTransaction,
    };
    use alloy_primitives::{Address, B256, Bytes, U256, address, keccak256, map::AddressSet};
    use alloy_provider::{Provider, ProviderBuilder, mock::Asserter};
    use alloy_rpc_types::{
        Block, BlockTransactions, Transaction as RpcTransaction, TransactionRequest,
    };
    use alloy_serde::WithOtherFields;
    use alloy_sol_types::SolValue;
    use anvil::{NodeConfig, spawn};
    use foundry_common::{SYSTEM_TRANSACTION_TYPE, provider::get_http_provider};
    use foundry_config::{Config, NamedChain};
    use foundry_evm_networks::{NetworkConfigs, celo::transfer::CELO_TRANSFER_ADDRESS};
    use foundry_fork_db::{
        SharedBackend,
        cache::{BlockchainDb, BlockchainDbMeta},
    };
    use revm::{
        context::{BlockEnv, JournalInner, TxEnv},
        database::{AccountState, CacheDB, DatabaseRef, DbAccount},
        primitives::{KECCAK_EMPTY, hardfork::SpecId},
        state::{Account, AccountInfo, EvmState, EvmStorageSlot, TransactionId},
    };

    fn fork_with_closed_backend() -> Fork<AnyNetwork, BlockEnv> {
        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(Asserter::new());
        let db = BlockchainDb::new(
            BlockchainDbMeta::new(BlockEnv::default(), "http://localhost".to_string()),
            None,
        );
        let (backend, handler) = SharedBackend::new(provider, db, None);
        drop(handler);
        Fork {
            db: CacheDB::new(backend),
            journaled_state: JournalInner::new(),
            source_chain_id: 1,
            position: ForkPosition::AfterBlock { block: BlockNumHash::default() },
        }
    }

    fn rpc_block(number: u64, hash: B256, parent_hash: B256) -> AnyRpcBlock {
        let header = AnyHeader { number, parent_hash, ..Default::default() };
        AnyRpcBlock::new(
            Block::new(
                AnyRpcHeader::from_sealed(header.seal(hash)),
                BlockTransactions::Full(Vec::new()),
            )
            .into(),
        )
    }

    #[test]
    fn validates_block_identity() {
        let hash = B256::with_last_byte(2);
        let block = rpc_block(2, hash, B256::with_last_byte(1));
        assert!(ensure_block_identity(&block, BlockNumHash::new(2, hash), "parent").is_ok());

        let err =
            ensure_block_identity(&block, BlockNumHash::new(2, B256::with_last_byte(3)), "parent")
                .unwrap_err();
        assert!(err.to_string().contains("parent block changed"));

        let err =
            ensure_block_identity(&block, BlockNumHash::new(1, hash), "grandparent").unwrap_err();
        assert!(err.to_string().contains("grandparent block changed"));
    }

    #[test]
    fn failed_fork_state_refresh_does_not_publish_transaction_changes() {
        let mut fork = fork_with_closed_backend();
        let externally_loaded = Address::with_last_byte(1);
        let fork_loaded = Address::with_last_byte(2);
        let committed = Address::with_last_byte(3);
        let missing_slot = U256::from(1);

        let cached_external = AccountInfo { balance: U256::from(11), ..Default::default() };
        let cached_fork = AccountInfo { balance: U256::from(12), ..Default::default() };
        fork.db.insert_account_info(externally_loaded, cached_external);
        fork.db.insert_account_info(fork_loaded, cached_fork);

        let mut journaled_state = JournalInner::new();
        let external_account = Account::default()
            .with_info(AccountInfo { balance: U256::from(1), ..Default::default() });
        journaled_state.state.insert(externally_loaded, external_account);

        let mut fork_account = Account::default()
            .with_info(AccountInfo { balance: U256::from(2), ..Default::default() });
        fork_account
            .storage
            .insert(missing_slot, EvmStorageSlot::new(U256::ZERO, TransactionId::ZERO));
        fork.journaled_state.state.insert(fork_loaded, fork_account);

        let mut committed_account = Account::default()
            .with_info(AccountInfo { balance: U256::from(13), ..Default::default() });
        committed_account.mark_touch();
        let mut state = EvmState::default();
        state.insert(committed, committed_account);

        let result =
            apply_state_changeset(state, &mut journaled_state, &mut fork, &AddressSet::default());
        assert!(result.is_err());
        assert!(!fork.db.cache.accounts.contains_key(&committed));
        assert_eq!(journaled_state.state[&externally_loaded].info.balance, U256::from(1));
        assert_eq!(fork.journaled_state.state[&fork_loaded].info.balance, U256::from(2));
    }

    #[test]
    fn failed_fork_state_refresh_preserves_not_existing_account() {
        let mut fork = fork_with_closed_backend();
        let address = Address::with_last_byte(1);
        let missing_slot = U256::from(1);
        fork.db.cache.accounts.insert(address, DbAccount::new_not_existing());

        let mut journaled_state = JournalInner::new();
        let mut journaled_account = Account::default()
            .with_info(AccountInfo { balance: U256::from(1), ..Default::default() });
        journaled_account
            .storage
            .insert(missing_slot, EvmStorageSlot::new(U256::from(7), TransactionId::ZERO));
        journaled_state.state.insert(address, journaled_account);

        let mut touched_account = Account::default()
            .with_info(AccountInfo { balance: U256::from(13), ..Default::default() });
        touched_account.mark_touch();
        let mut state = EvmState::default();
        state.insert(address, touched_account);

        let result =
            apply_state_changeset(state, &mut journaled_state, &mut fork, &AddressSet::default());
        assert!(result.is_err());
        assert_eq!(fork.db.cache.accounts[&address].account_state, AccountState::NotExisting);
        assert_eq!(journaled_state.state[&address].info.balance, U256::from(1));
        assert_eq!(
            journaled_state.state[&address].storage[&missing_slot].present_value(),
            U256::from(7)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refresh_fork_account_updates_loaded_journals() {
        let (api, handle) = spawn(NodeConfig::test()).await;
        let target = address!("0x0000000000000000000000000000000000001331");
        let code =
            Bytes::from_static(&[0x60, 0x2a, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3]);

        let provider = handle.http_provider();
        let block_number = provider.get_block_number().await.unwrap();
        let mut evm_opts = Config::figment().extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some(handle.http_endpoint());
        evm_opts.fork_block_number = Some(block_number);
        let fork = evm_opts.get_fork(&Config::default(), 31_337, Some(block_number)).unwrap();
        let mut backend = Backend::<EthEvmNetwork>::spawn(Some(fork)).unwrap();

        let mut journaled_state = JournalInner::new();
        journaled_state.load_account(&mut backend, target).unwrap();
        journaled_state.state.get_mut(&target).unwrap().info.balance = U256::from(1);
        let fork = backend.active_fork_mut().unwrap();
        fork.journaled_state.load_account(&mut fork.db, target).unwrap();
        fork.journaled_state.state.get_mut(&target).unwrap().info.balance = U256::from(2);
        let cached = fork.db.cache.accounts.get_mut(&target).unwrap();
        cached.info.balance = U256::from(3);
        cached.account_state = AccountState::Touched;
        assert_eq!(journaled_state.state[&target].info.code_hash, KECCAK_EMPTY);
        assert_eq!(fork.journaled_state.state[&target].info.code_hash, KECCAK_EMPTY);

        api.anvil_set_code(target, code.clone()).await.unwrap();
        backend.refresh_fork_account(target, ForkAccountField::Code, &mut journaled_state).unwrap();

        let expected_hash = keccak256(&code);
        let refreshed = &journaled_state.state[&target].info;
        assert_eq!(refreshed.code_hash, expected_hash);
        assert_eq!(refreshed.code.as_ref().unwrap().original_bytes(), code);
        assert_eq!(refreshed.balance, U256::from(1));
        let refreshed = &backend.active_fork().unwrap().journaled_state.state[&target].info;
        assert_eq!(refreshed.code_hash, expected_hash);
        assert_eq!(refreshed.code.as_ref().unwrap().original_bytes(), code);
        assert_eq!(refreshed.balance, U256::from(2));
        let cached = &backend.active_fork().unwrap().db.cache.accounts[&target];
        assert_eq!(cached.info.code_hash, expected_hash);
        assert_eq!(cached.info.balance, U256::from(3));
    }

    #[test]
    fn ethereum_replay_skips_unknown_system_envelopes_before_conversion() {
        let transaction = |ty| {
            let unknown = AnyTxEnvelope::Unknown(UnknownTxEnvelope {
                hash: B256::ZERO,
                inner: UnknownTypedTransaction {
                    ty: AnyTxType(ty),
                    fields: Default::default(),
                    memo: Default::default(),
                },
            });
            AnyRpcTransaction::new(WithOtherFields::new(RpcTransaction {
                inner: Recovered::new_unchecked(unknown, Address::with_last_byte(0x42)),
                block_hash: None,
                block_number: None,
                transaction_index: None,
                effective_gas_price: None,
                block_timestamp: None,
            }))
        };

        assert!(
            Backend::<EthEvmNetwork>::replay_tx_env(&transaction(SYSTEM_TRANSACTION_TYPE))
                .unwrap()
                .is_none()
        );
        assert!(Backend::<EthEvmNetwork>::replay_tx_env(&transaction(0xff)).is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn celo_transaction_hash_fork_replays_transfer_precompile() {
        let networks = NetworkConfigs::with_celo();
        let (api, handle) = spawn(
            NodeConfig::test().with_chain_id(Some(NamedChain::Celo as u64)).with_networks(networks),
        )
        .await;
        let provider = handle.http_provider();
        let sender = provider.get_accounts().await.unwrap()[0];
        let recipient = Address::with_last_byte(0x99);
        let transfer_amount = U256::from(1_000);
        let target_amount = U256::from(1);
        let nonce = provider.get_transaction_count(sender).await.unwrap();
        let gas_price = provider.get_gas_price().await.unwrap();

        api.anvil_set_auto_mine(false).await.unwrap();
        api.send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(CELO_TRANSFER_ADDRESS)
                .with_nonce(nonce)
                .with_gas_limit(100_000)
                .with_gas_price(gas_price)
                .with_input(Bytes::from((sender, recipient, transfer_amount).abi_encode())),
        ))
        .await
        .unwrap();
        let target_hash = api
            .send_transaction(WithOtherFields::new(
                TransactionRequest::default()
                    .with_from(sender)
                    .with_to(recipient)
                    .with_nonce(nonce + 1)
                    .with_gas_limit(21_000)
                    .with_gas_price(gas_price)
                    .with_value(target_amount),
            ))
            .await
            .unwrap();
        api.mine_one().await.unwrap();

        assert_eq!(provider.get_balance(recipient).await.unwrap(), transfer_amount + target_amount);

        let endpoint = handle.http_endpoint();
        let fork_block_number = provider.get_block_number().await.unwrap();
        let evm_opts = EvmOpts {
            fork_url: Some(endpoint.clone()),
            fork_block_number: Some(fork_block_number),
            networks,
            ..Default::default()
        };
        let fork = CreateFork { url: endpoint, enable_caching: false, evm_opts, resolved: None };
        let mut backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        backend.set_networks(networks);

        let fork_id = backend.create_fork_at_transaction(fork, target_hash).unwrap();
        let fork = backend.inner.get_fork_by_id(fork_id).unwrap();
        assert!(matches!(
            fork.position,
            ForkPosition::BeforeTransaction { transaction_index: 1, .. }
        ));
        assert_eq!(
            fork.db.basic_ref(recipient).unwrap().unwrap_or_default().balance,
            transfer_amount
        );
    }

    #[test]
    fn fork_position_advances_from_exact_transaction_predecessor() {
        let parent_block = BlockNumHash::new(10, B256::with_last_byte(10));
        let block = BlockNumHash::new(11, B256::with_last_byte(11));
        let parent = ForkPosition::AfterBlock { block: parent_block };
        assert_eq!(
            parent.after_transaction(block, parent_block.hash, 0, 2),
            Some(ForkPosition::BeforeTransaction { block, transaction_index: 1 })
        );
        assert_eq!(
            parent.after_transaction(block, parent_block.hash, 0, 1),
            Some(ForkPosition::AfterBlock { block })
        );

        let before_first = ForkPosition::BeforeTransaction { block, transaction_index: 0 };
        assert_eq!(
            before_first.after_transaction(block, parent_block.hash, 0, 2),
            Some(ForkPosition::BeforeTransaction { block, transaction_index: 1 })
        );

        let before_second = ForkPosition::BeforeTransaction { block, transaction_index: 1 };
        assert_eq!(
            before_second.after_transaction(block, parent_block.hash, 1, 3),
            Some(ForkPosition::BeforeTransaction { block, transaction_index: 2 })
        );
        assert_eq!(
            before_second.after_transaction(block, parent_block.hash, 1, 2),
            Some(ForkPosition::AfterBlock { block })
        );

        assert_eq!(parent.after_transaction(block, B256::ZERO, 0, 1), None);
        assert_eq!(parent.after_transaction(block, parent_block.hash, 1, 2), None);
        assert_eq!(before_second.after_transaction(block, parent_block.hash, 0, 3), None);
        assert_eq!(before_second.after_transaction(block, parent_block.hash, 2, 3), None);
        assert_eq!(before_second.after_transaction(block, parent_block.hash, 1, 1), None);
        assert_eq!(parent.after_transaction(block, parent_block.hash, 0, 0), None);
    }

    #[test]
    fn fork_replay_block_env_preserves_arbitrum_l1_number() {
        let header = AnyHeader { number: 75_219_831, ..Default::default() };
        let mut block = AnyRpcBlock::new(
            Block::new(
                AnyRpcHeader::from_sealed(header.seal(B256::ZERO)),
                BlockTransactions::Full(Vec::new()),
            )
            .into(),
        );
        block.other.insert("l1BlockNumber".to_string(), serde_json::json!("0x10276d3"));
        let mut evm_env =
            EvmEnv::new(revm::context::CfgEnv::<SpecId>::default(), BlockEnv::default());

        update_env_block::<AnyNetwork, _, _>(
            &mut evm_env,
            &block,
            NamedChain::Arbitrum as u64,
            NetworkConfigs::default(),
        );

        assert_eq!(evm_env.block_env.number, U256::from(16_938_707));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn temporary_backend_preserves_fork_position() {
        let (_api, handle) = spawn(NodeConfig::test()).await;
        let provider = handle.http_provider();
        let block_number = provider.get_block_number().await.unwrap();

        let mut evm_opts = Config::figment().extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some(handle.http_endpoint());
        evm_opts.fork_block_number = Some(block_number);
        let fork = evm_opts.get_fork(&Config::default(), 31_337, Some(block_number)).unwrap();
        let mut backend = Backend::<EthEvmNetwork>::spawn(Some(fork)).unwrap();
        let id = backend.active_fork_ids.unwrap().0;
        let fork_id = backend.inner.ensure_fork_id(id).unwrap().clone();

        for position in [
            ForkPosition::BeforeTransaction {
                block: BlockNumHash::new(block_number + 1, B256::with_last_byte(1)),
                transaction_index: 2,
            },
            ForkPosition::AfterBlock {
                block: BlockNumHash::new(block_number + 2, B256::with_last_byte(2)),
            },
        ] {
            backend.inner.get_fork_by_id_mut(id).unwrap().position = position;
            let fork = backend.active_fork().unwrap().clone();
            let journaled_state = fork.journaled_state.clone();
            let mut temporary = Backend::<EthEvmNetwork>::new_with_fork(
                &fork_id,
                fork,
                journaled_state,
                NetworkConfigs::default(),
            )
            .unwrap();

            assert_eq!(temporary.active_fork().unwrap().position, position);
            let expected = match position {
                ForkPosition::AfterBlock { block }
                | ForkPosition::BeforeTransaction { block, .. } => block.number,
            };
            assert_eq!(temporary.active_fork_block_number(), Some(expected));
            temporary.fork_block_number_override = Some(expected + 1);
            assert_eq!(temporary.active_fork_block_number(), Some(expected + 1));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg(feature = "monad")]
    async fn fork_factory_boundary_preserves_explicit_execution_overrides() {
        async fn pinned_opts(endpoint: String, networks: Option<NetworkConfigs>) -> EvmOpts {
            let mut opts = EvmOpts { fork_url: Some(endpoint), ..Default::default() };
            if let Some(networks) = networks {
                opts.networks = networks;
            }
            opts.infer_network_from_fork().await.unwrap();
            let identity = opts.fork_endpoint.clone().unwrap();
            let network_is_inferred = opts.fork_network_is_inferred;
            opts.expect_fork_endpoint(identity, network_is_inferred);
            opts.pin_fork_block().await.unwrap();
            opts
        }

        fn target_fork(opts: EvmOpts, url: String) -> crate::fork::CreateFork {
            crate::fork::CreateFork { url, enable_caching: false, evm_opts: opts, resolved: None }
        }

        let (ethereum_base_api, ethereum_base) = spawn(NodeConfig::test()).await;
        let (_ethereum_target_api, ethereum_target) = spawn(NodeConfig::test()).await;
        let (monad_base_api, monad_base) = spawn(NodeConfig::test_monad()).await;
        let (_monad_target_api, monad_target) = spawn(NodeConfig::test_monad()).await;
        ethereum_base_api.mine_one().await.unwrap();
        monad_base_api.mine_one().await.unwrap();

        let inferred_ethereum = pinned_opts(ethereum_base.http_endpoint(), None).await;
        assert!(inferred_ethereum.fork_network_is_inferred);
        let error = Backend::<EthEvmNetwork>::spawn(Some(target_fork(
            inferred_ethereum,
            monad_target.http_endpoint(),
        )))
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cannot create a `monad` fork with an EVM instantiated for `ethereum`"),
            "{error}"
        );

        let inferred_monad = pinned_opts(monad_base.http_endpoint(), None).await;
        assert!(inferred_monad.fork_network_is_inferred);
        let error = Backend::<crate::evm::MonadEvmNetwork>::spawn(Some(target_fork(
            inferred_monad,
            ethereum_target.http_endpoint(),
        )))
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cannot create a `ethereum` fork with an EVM instantiated for `monad`"),
            "{error}"
        );

        let explicit_ethereum =
            pinned_opts(ethereum_base.http_endpoint(), Some(NetworkConfigs::with_ethereum())).await;
        assert!(!explicit_ethereum.fork_network_is_inferred);
        let _backend = Backend::<EthEvmNetwork>::spawn(Some(target_fork(
            explicit_ethereum,
            monad_target.http_endpoint(),
        )))
        .unwrap();

        let explicit_monad =
            pinned_opts(monad_base.http_endpoint(), Some(NetworkConfigs::with_monad())).await;
        assert!(!explicit_monad.fork_network_is_inferred);
        let _backend = Backend::<crate::evm::MonadEvmNetwork>::spawn(Some(target_fork(
            explicit_monad,
            ethereum_target.http_endpoint(),
        )))
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn can_read_write_cache() {
        let endpoint = &*foundry_test_utils::rpc::next_http_rpc_endpoint();
        let provider = get_http_provider(endpoint);

        let block_num = provider.get_block_number().await.unwrap();

        let mut evm_opts = Config::figment().extract::<EvmOpts>().unwrap();
        evm_opts.fork_url = Some(endpoint.to_string());
        evm_opts.fork_block_number = Some(block_num);

        let (evm_env, _, resolved) =
            evm_opts.env_resolved::<SpecId, BlockEnv, TxEnv>().await.unwrap();

        let fork = evm_opts
            .get_fork_resolved(&Config::default(), evm_env.cfg_env.chain_id, resolved.as_ref())
            .unwrap();

        let resolved = resolved.unwrap();
        let fork_hash = resolved.hash();
        let source_id = resolved.source_id();
        let backend = Backend::<EthEvmNetwork>::spawn(Some(fork)).unwrap();

        // some rng contract from etherscan
        let address = address!("0x63091244180ae240c87d1f528f5f269134cb07b3");

        let num_slots = 5;
        let _account = backend.basic_ref(address);
        for idx in 0..num_slots {
            let _ = backend.storage_ref(address, U256::from(idx));
        }
        drop(backend);

        let meta = BlockchainDbMeta::new(evm_env.block_env, endpoint.to_string())
            .with_fork_identity(fork_hash, source_id);

        let db = BlockchainDb::new(
            meta,
            Some(Config::foundry_block_cache_dir(NamedChain::Mainnet, block_num).unwrap()),
        );
        assert!(db.accounts().read().contains_key(&address));
        assert!(db.storage().read().contains_key(&address));
        assert_eq!(db.storage().read().get(&address).unwrap().len(), num_slots as usize);
    }
}
