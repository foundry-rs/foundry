use super::config::EvmOpts;
use crate::Cheatcodes;
use alloy_primitives::{Address, B256, U256};
use itertools::Itertools;
use revm::{primitives::Env, Database, JournaledState};

mod error;
pub use error::{DatabaseError, DatabaseResult};

/// Represents a numeric `ForkId` valid only for the existence of the `Backend`.
/// The difference between `ForkId` and `LocalForkId` is that `ForkId` tracks pairs of `endpoint +
/// block` which can be reused by multiple tests, whereas the `LocalForkId` is unique within a test
pub type LocalForkId = U256;

/// Represents possible diagnostic cases on revert
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum RevertDiagnostic {
    /// The `contract` does not exist on the `active` fork but exist on other fork(s)
    ContractExistsOnOtherForks {
        contract: Address,
        active: LocalForkId,
        available_on: Vec<LocalForkId>,
    },
    ContractDoesNotExist {
        contract: Address,
        active: LocalForkId,
        persistent: bool,
    },
}

impl RevertDiagnostic {
    /// Converts the diagnostic to a readable error message
    pub fn to_error_msg(&self, cheats: &Cheatcodes) -> String {
        let get_label =
            |addr: &Address| cheats.labels.get(addr).cloned().unwrap_or_else(|| addr.to_string());

        match self {
            RevertDiagnostic::ContractExistsOnOtherForks { contract, active, available_on } => {
                let contract_label = get_label(contract);

                format!(
                    "Contract {contract_label} does not exist on active fork with id `{active}`
                     but exists on non active forks: `[{}]`",
                    available_on.iter().format(", ")
                )
            }
            RevertDiagnostic::ContractDoesNotExist { contract, persistent, .. } => {
                let contract_label = get_label(contract);
                if *persistent {
                    format!("Contract {contract_label} does not exist")
                } else {
                    format!(
                        "Contract {contract_label} does not exist and is not marked \
                         as persistent, see `makePersistent`"
                    )
                }
            }
        }
    }
}

/// Represents a _fork_ of a remote chain whose data is available only via the `url` endpoint.
#[derive(Debug, Clone)]
pub struct CreateFork {
    /// Whether to enable rpc storage caching for this fork.
    pub enable_caching: bool,
    /// The URL to a node for fetching remote state.
    pub url: String,
    /// The env to create this fork, main purpose is to provide some metadata for the fork.
    pub env: Env,
    /// All env settings as configured by the user
    pub evm_opts: EvmOpts,
}

/// Extend the [`revm::Database`] with cheatcodes specific functionality.
pub trait DatabaseExt: Database<Error = DatabaseError> {
    /// Creates a new snapshot at the current point of execution.
    ///
    /// A snapshot is associated with a new unique id that's created for the snapshot.
    /// Snapshots can be reverted: [DatabaseExt::revert], however a snapshot can only be reverted
    /// once. After a successful revert, the same snapshot id cannot be used again.
    fn snapshot(&mut self, journaled_state: &JournaledState, env: &Env) -> U256;

    /// Reverts the snapshot if it exists
    ///
    /// Returns `true` if the snapshot was successfully reverted, `false` if no snapshot for that id
    /// exists.
    ///
    /// **N.B.** While this reverts the state of the evm to the snapshot, it keeps new logs made
    /// since the snapshots was created. This way we can show logs that were emitted between
    /// snapshot and its revert.
    /// This will also revert any changes in the `Env` and replace it with the captured `Env` of
    /// `Self::snapshot`
    fn revert(
        &mut self,
        id: U256,
        journaled_state: &JournaledState,
        env: &mut Env,
    ) -> Option<JournaledState>;

    /// Creates and also selects a new fork
    ///
    /// This is basically `create_fork` + `select_fork`
    fn create_select_fork(
        &mut self,
        fork: CreateFork,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<LocalForkId> {
        let id = self.create_fork(fork)?;
        self.select_fork(id, env, journaled_state)?;
        Ok(id)
    }

    /// Creates and also selects a new fork
    ///
    /// This is basically `create_fork` + `select_fork`
    fn create_select_fork_at_transaction(
        &mut self,
        fork: CreateFork,
        env: &mut Env,
        journaled_state: &mut JournaledState,
        transaction: B256,
    ) -> eyre::Result<LocalForkId> {
        let id = self.create_fork_at_transaction(fork, transaction)?;
        self.select_fork(id, env, journaled_state)?;
        Ok(id)
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
    /// This will also modify the current `Env`.
    ///
    /// **Note**: this does not change the local state, but swaps the remote state
    ///
    /// # Errors
    ///
    /// Returns an error if no fork with the given `id` exists
    fn select_fork(
        &mut self,
        id: LocalForkId,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()>;

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
        block_number: U256,
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()>;

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
        env: &mut Env,
        journaled_state: &mut JournaledState,
    ) -> eyre::Result<()>;

    /// Fetches the given transaction for the fork and executes it, committing the state in the DB
    fn transact(
        &mut self,
        id: Option<LocalForkId>,
        transaction: B256,
        env: &mut Env,
        journaled_state: &mut JournaledState,
        cheatcodes_inspector: Option<&mut Cheatcodes>,
    ) -> eyre::Result<()>;

    /// Returns the `ForkId` that's currently used in the database, if fork mode is on
    fn active_fork_id(&self) -> Option<LocalForkId>;

    /// Returns the Fork url that's currently used in the database, if fork mode is on
    fn active_fork_url(&self) -> Option<String>;

    /// Whether the database is currently in forked
    fn is_forked_mode(&self) -> bool {
        self.active_fork_id().is_some()
    }

    /// Ensures that an appropriate fork exits
    ///
    /// If `id` contains a requested `Fork` this will ensure it exits.
    /// Otherwise, this returns the currently active fork.
    ///
    /// # Errors
    ///
    /// Returns an error if the given `id` does not match any forks
    ///
    /// Returns an error if no fork exits
    fn ensure_fork(&self, id: Option<LocalForkId>) -> eyre::Result<LocalForkId>;

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
    fn diagnose_revert(
        &self,
        callee: Address,
        journaled_state: &JournaledState,
    ) -> Option<RevertDiagnostic>;

    /// Returns true if the given account is currently marked as persistent.
    fn is_persistent(&self, acc: &Address) -> bool;

    /// Revokes persistent status from the given account.
    fn remove_persistent_account(&mut self, account: &Address) -> bool;

    /// Marks the given account as persistent.
    fn add_persistent_account(&mut self, account: Address) -> bool;

    /// Removes persistent status from all given accounts
    fn remove_persistent_accounts(&mut self, accounts: impl IntoIterator<Item = Address>) {
        for acc in accounts {
            self.remove_persistent_account(&acc);
        }
    }

    /// Extends the persistent accounts with the accounts the iterator yields.
    fn extend_persistent_accounts(&mut self, accounts: impl IntoIterator<Item = Address>) {
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
    fn revoke_cheatcode_access(&mut self, account: Address) -> bool;

    /// Returns `true` if the given account is allowed to execute cheatcodes
    fn has_cheatcode_access(&self, account: Address) -> bool;

    /// Ensures that `account` is allowed to execute cheatcodes
    ///
    /// Returns an error if [`Self::has_cheatcode_access`] returns `false`
    fn ensure_cheatcode_access(&self, account: Address) -> Result<(), DatabaseError> {
        if !self.has_cheatcode_access(account) {
            return Err(DatabaseError::NoCheats(account))
        }
        Ok(())
    }

    /// Same as [`Self::ensure_cheatcode_access()`] but only enforces it if the backend is currently
    /// in forking mode
    fn ensure_cheatcode_access_forking_mode(&self, account: Address) -> Result<(), DatabaseError> {
        if self.is_forked_mode() {
            return self.ensure_cheatcode_access(account)
        }
        Ok(())
    }
}
