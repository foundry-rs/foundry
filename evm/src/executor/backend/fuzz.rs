use super::update_current_env_with_fork_env;
use crate::{
    abi::CHEATCODE_ADDRESS,
    executor::{
        backend::{snapshot::BackendSnapshot, Backend, BackendDatabase, BackendInner, DatabaseExt},
        fork::{CreateFork, ForkId},
    },
    Address,
};
use bytes::Bytes;
use ethers::prelude::{H160, H256, U256};
use hashbrown::HashMap as Map;
use revm::{
    db::{CacheDB, DatabaseRef},
    Account, AccountInfo, Database, Env, Inspector, Log, Return, SubRoutine, TransactOut,
    TransactTo,
};
use tracing::{trace, warn};

/// A wrapper around `Backend` that ensures only `revm::DatabaseRef` functions are called.
///
/// Any changes made during its existence that affect the caching layer of the underlying Database
/// will result in a clone of the initial Database. Therefor, this backend type is something akin to
/// a clone-on-write `Backend` type.
///
/// Main purpose for this type is for fuzzing. A test function fuzzer will repeatedly execute  the
/// function via immutable raw (no state changes) calls.
///
/// **N.B.**: we're assuming cheatcodes that alter the state (like multi fork swapping) are niche.
/// If they executed during fuzzing, it will require a clone of the initial input database. This way
/// we can support these cheatcodes in fuzzing cheaply without adding overhead for fuzz tests that
/// don't make use of them. Alternatively each test case would require its own `Backend` clone,
/// which would add significant overhead for large fuzz sets even if the Database is not big after
/// setup.
#[derive(Debug, Clone)]
pub struct FuzzBackendWrapper<'a> {
    /// The underlying immutable `Backend`
    ///
    /// No calls on the `FuzzBackendWrapper` will ever persistently modify the `backend`'s state.
    pub backend: &'a Backend,
    /// active database clone that holds the currently active db, like reverted snapshots, selected
    /// fork, etc.
    db_override: Option<CacheDB<BackendDatabase>>,
    /// holds additional Backend data
    inner: BackendInner,
}

// === impl FuzzBackendWrapper ===

impl<'a> FuzzBackendWrapper<'a> {
    pub fn new(inner: &'a Backend) -> Self {
        Self { backend: inner, db_override: None, inner: Default::default() }
    }

    /// Returns the currently active database
    fn active_db(&self) -> &CacheDB<BackendDatabase> {
        self.db_override.as_ref().unwrap_or(&self.backend.db)
    }

    /// Sets the database override
    fn set_active(&mut self, db: CacheDB<BackendDatabase>) {
        self.db_override = Some(db)
    }

    /// Sets the address of the `DSTest` contract that is being executed
    pub fn set_test_contract(&mut self, addr: Address) -> &mut Self {
        self.inner.test_contract_context = Some(addr);
        self
    }

    /// Returns the address of the set `DSTest` contract
    pub fn test_contract_address(&self) -> Option<Address> {
        self.inner.test_contract_context
    }

    /// Checks if the test contract associated with this backend failed, See
    /// [Self::is_failed_test_contract]
    pub fn is_failed(&self) -> bool {
        self.backend.is_failed() ||
            self.inner.has_failure_snapshot ||
            self.test_contract_address()
                .map(|addr| self.is_failed_test_contract(addr))
                .unwrap_or_default()
    }

    /// Checks if the given test function failed
    ///
    /// DSTest will not revert inside its `assertEq`-like functions which allows
    /// to test multiple assertions in 1 test function while also preserving logs.
    /// Instead, it stores whether an `assert` failed in a boolean variable that we can read
    pub fn is_failed_test_contract(&self, address: Address) -> bool {
        /*
         contract DSTest {
            bool public IS_TEST = true;
            // slot 0 offset 1 => second byte of slot0
            bool private _failed;
         }
        */
        let value = self.storage(address, U256::zero());

        value.byte(1) != 0
    }

    /// In addition to the `_failed` variable, `DSTest::fail()` stores a failure
    /// in "failed"
    /// See <https://github.com/dapphub/ds-test/blob/9310e879db8ba3ea6d5c6489a579118fd264a3f5/src/test.sol#L66-L72>
    pub fn is_global_failure(&self) -> bool {
        let index = U256::from(&b"failed"[..]);
        let value = self.storage(CHEATCODE_ADDRESS, index);
        value == U256::one()
    }

    /// Executes the configured transaction of the `env` without commiting state changes
    pub fn inspect_ref<INSP>(
        &mut self,
        mut env: Env,
        mut inspector: INSP,
    ) -> (Return, TransactOut, u64, Map<Address, Account>, Vec<Log>)
    where
        INSP: Inspector<Self>,
    {
        if let TransactTo::Call(to) = env.tx.transact_to {
            self.inner.test_contract_context = Some(to);
        }
        revm::evm_inner::<Self, true>(&mut env, self, &mut inspector).transact()
    }
}

impl<'a> DatabaseExt for FuzzBackendWrapper<'a> {
    fn snapshot(&mut self, subroutine: &SubRoutine, env: &Env) -> U256 {
        let id = self.inner.snapshots.insert(BackendSnapshot::new(
            self.active_db().clone(),
            subroutine.clone(),
            env.clone(),
        ));
        trace!(target: "backend::fuzz", "Created new snapshot {}", id);
        id
    }

    fn revert(
        &mut self,
        id: U256,
        subroutine: &SubRoutine,
        current: &mut Env,
    ) -> Option<SubRoutine> {
        if let Some(mut snapshot) =
            self.inner.snapshots.remove(id).or_else(|| self.backend.snapshots().get(id).cloned())
        {
            // need to check whether DSTest's `failed` variable is set to `true` which means an
            // error occurred either during the snapshot or even before
            if self.is_failed() {
                self.inner.has_failure_snapshot = true;
            }
            // merge additional logs
            snapshot.merge(subroutine);
            let BackendSnapshot { db, subroutine, env } = snapshot;
            self.set_active(db);
            update_current_env_with_fork_env(current, env);

            trace!(target: "backend::fuzz", "Reverted snapshot {}", id);
            Some(subroutine)
        } else {
            warn!(target: "backend::fuzz", "No snapshot to revert for {}", id);
            None
        }
    }

    fn create_fork(&mut self, fork: CreateFork) -> eyre::Result<U256> {
        let (id, fork) = self.backend.forks.create_fork(fork)?;
        let id = self.inner.insert_new_fork(id, fork);
        Ok(id)
    }

    fn select_fork(&mut self, id: U256, env: &mut Env) -> eyre::Result<()> {
        let fork_id = self.ensure_fork_id(id).cloned()?;
        let fork_env = self
            .backend
            .forks
            .get_env(fork_id)?
            .ok_or_else(|| eyre::eyre!("Requested fork `{}` does not exit", id))?;
        let fork = self
            .inner
            .ensure_backend(id)
            .or_else(|_| self.backend.inner.ensure_backend(id))
            .cloned()?;

        if let Some(ref mut db) = self.db_override {
            db.db = BackendDatabase::Forked(fork, id);
        } else {
            let mut db = self.backend.db.clone();
            db.db = BackendDatabase::Forked(fork, id);
            self.set_active(db);
        }

        update_current_env_with_fork_env(env, fork_env);
        Ok(())
    }

    fn roll_fork(
        &mut self,
        env: &mut Env,
        block_number: U256,
        id: Option<U256>,
    ) -> eyre::Result<()> {
        let id = self.ensure_fork(id)?;
        let (fork_id, fork) = self
            .backend
            .forks
            .roll_fork(self.inner.ensure_fork_id(id).cloned()?, block_number.as_u64())?;
        // this will update the local mapping
        self.inner.update_fork_mapping(id, fork_id, fork);
        if self.active_fork() == Some(id) {
            // need to update the block number right away
            env.block.number = block_number;
        }
        Ok(())
    }

    fn active_fork(&self) -> Option<U256> {
        self.active_db().db.as_fork()
    }

    fn ensure_fork(&self, id: Option<U256>) -> eyre::Result<U256> {
        if let Some(id) = id {
            if self.inner.issued_local_fork_ids.contains_key(&id) ||
                self.backend.inner.issued_local_fork_ids.contains_key(&id)
            {
                return Ok(id)
            }
            eyre::bail!("Requested fork `{}` does not exit", id)
        }
        if let Some(id) = self.active_fork() {
            Ok(id)
        } else {
            eyre::bail!("No fork active")
        }
    }

    fn ensure_fork_id(&self, id: U256) -> eyre::Result<&ForkId> {
        self.inner.ensure_fork_id(id).or_else(|_| self.backend.ensure_fork_id(id))
    }
}

impl<'a> DatabaseRef for FuzzBackendWrapper<'a> {
    fn basic(&self, address: H160) -> AccountInfo {
        if let Some(ref db) = self.db_override {
            DatabaseRef::basic(db, address)
        } else {
            DatabaseRef::basic(self.backend, address)
        }
    }

    fn code_by_hash(&self, code_hash: H256) -> Bytes {
        if let Some(ref db) = self.db_override {
            DatabaseRef::code_by_hash(db, code_hash)
        } else {
            DatabaseRef::code_by_hash(self.backend, code_hash)
        }
    }

    fn storage(&self, address: H160, index: U256) -> U256 {
        if let Some(ref db) = self.db_override {
            DatabaseRef::storage(db, address, index)
        } else {
            DatabaseRef::storage(self.backend, address, index)
        }
    }

    fn block_hash(&self, number: U256) -> H256 {
        if let Some(ref db) = self.db_override {
            DatabaseRef::block_hash(db, number)
        } else {
            DatabaseRef::block_hash(self.backend, number)
        }
    }
}

impl<'a> Database for FuzzBackendWrapper<'a> {
    fn basic(&mut self, address: H160) -> AccountInfo {
        DatabaseRef::basic(self, address)
    }
    fn code_by_hash(&mut self, code_hash: H256) -> Bytes {
        DatabaseRef::code_by_hash(self, code_hash)
    }
    fn storage(&mut self, address: H160, index: U256) -> U256 {
        DatabaseRef::storage(self, address, index)
    }

    fn block_hash(&mut self, number: U256) -> H256 {
        DatabaseRef::block_hash(self, number)
    }
}
