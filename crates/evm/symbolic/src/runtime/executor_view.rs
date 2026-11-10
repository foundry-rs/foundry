use super::*;
use evm2::evm::{AccountInfo, DbResult};
use foundry_common::contracts::ContractsByArtifact;
use foundry_evm::{
    core::backend::Backend,
    fuzz::strategies::LiteralsDictionary,
    inspectors::cheatcodes::{CheatcodesCore, Prank, RecordAccess},
};

/// Reads of a backend's accepted state, independent of its network.
pub(crate) trait AcceptedState {
    /// Returns an account, with its code.
    fn basic(&self, address: Address) -> DbResult<Option<AccountInfo>>;

    /// Returns a storage slot.
    fn storage(&self, address: Address, slot: U256) -> DbResult<U256>;

    /// Returns the cached storage slots of an account, with their values, in slot order.
    fn storage_slots(&self, address: Address) -> Vec<(U256, U256)>;

    /// Returns a block hash.
    fn block_hash(&self, number: U256) -> DbResult<B256>;
}

impl<FEN: FoundryEvmNetwork> AcceptedState for Backend<FEN> {
    fn basic(&self, address: Address) -> DbResult<Option<AccountInfo>> {
        Self::basic(self, address)
    }

    fn storage(&self, address: Address, slot: U256) -> DbResult<U256> {
        Self::storage(self, address, slot)
    }

    fn storage_slots(&self, address: Address) -> Vec<(U256, U256)> {
        let mut slots = self
            .lifecycle()
            .active()
            .cache
            .as_ref()
            .and_then(|cache| cache.storage.get(&address))
            .map(|storage| {
                storage.slots.iter().map(|(slot, value)| (*slot, *value)).collect::<Vec<_>>()
            })
            .unwrap_or_default();
        slots.sort_unstable();
        slots
    }

    fn block_hash(&self, number: U256) -> DbResult<B256> {
        Self::block_hash(self, number)
    }
}

/// Network-independent, read-only view of a concrete Foundry executor.
///
/// The view is built by the caller of the public entry points so the symbolic executor does not
/// have to be monomorphized for every [`FoundryEvmNetwork`].
pub struct ExecutorView<'a> {
    pub(crate) backend: &'a dyn AcceptedState,
    pub(crate) spec_id: SpecId,
    pub(crate) chain_id: u64,
    pub(crate) memory_limit: u64,
    /// Gas limit of the calls and deployments the executor runs.
    pub(crate) gas_limit: u64,
    pub(crate) max_code_size: usize,
    pub(crate) active_fork_id: Option<U256>,
    /// Addresses cached by the active context, including committed-session reads.
    pub(crate) account_addresses: Vec<Address>,
    pub(crate) block: ExecutorBlock,
    pub(crate) gas_price: u128,
    pub(crate) isolate: bool,
    /// Whether `vm.expectRevert` may be satisfied by an internal call below the next call.
    pub(crate) internal_expect_revert: bool,
    /// Whether a failed assertion cheatcode reverts its call instead of setting the fail slot.
    pub(crate) assertions_revert: bool,
    pub(crate) blob_hashes: Vec<B256>,
    pub(crate) arbitrary_storage_targets: Vec<(Address, bool)>,
    pub(crate) arbitrary_storage_copies: Vec<(Address, Address)>,
    pub(crate) storage_load_hooks: Vec<(Address, SymbolicStorageHook)>,
    pub(crate) storage_store_hooks: Vec<(Address, SymbolicStorageHook)>,
    pub(crate) mapping_storage_store_hooks: Vec<((Address, U256), SymbolicStorageHook)>,
    /// Cheatcode state that `setUp` left behind, which Forge keeps for the test.
    pub(crate) cheatcodes: Option<&'a CheatcodesCore>,
    pub(crate) pranks: Vec<Prank>,
    pub(crate) accesses: Option<&'a RecordAccess>,
    pub(crate) recording_accesses: bool,
    /// Project artifacts, used to find the storage layouts of deployed contracts.
    pub(crate) known_contracts: Option<&'a ContractsByArtifact>,
    /// Source literals, used as candidate keys of constant-folded mapping slots.
    pub(crate) literals: Option<&'a LiteralsDictionary>,
    pub(crate) expects_safe_memory: bool,
    /// Accounts the backend marks persistent across fork switches, which `vm.isPersistent` sees.
    pub(crate) persistent_accounts: Vec<Address>,
}

/// Block environment values exposed to symbolic execution.
pub(crate) struct ExecutorBlock {
    pub(crate) coinbase: Address,
    pub(crate) timestamp: U256,
    pub(crate) number: U256,
    pub(crate) difficulty: U256,
    pub(crate) gas_limit: U256,
    pub(crate) basefee: U256,
    pub(crate) blob_basefee: U256,
    pub(crate) slot_num: U256,
}

impl<'a> ExecutorView<'a> {
    /// Snapshots the environment, backend, and cheatcode configuration of `executor`.
    pub fn new<FEN: FoundryEvmNetwork>(executor: &'a Executor<FEN>) -> Self {
        let backend = executor.backend();
        let evm_env = executor.evm_env();
        let cheats = executor.inspector().cheatcodes.as_deref();
        let block = cheats.and_then(|cheats| cheats.block.as_ref()).unwrap_or(&evm_env.block);
        let difficulty = if evm_env.version.feature(EvmFeatures::EIP4399) {
            block.prevrandao
        } else {
            block.difficulty
        };
        // Environment cheatcodes the running transaction cannot apply override their opcodes.
        let overrides =
            cheats.and_then(|cheats| cheats.env_overrides.get(backend.active_fork_id()));
        let tx = executor.tx();

        let account_addresses = backend
            .lifecycle()
            .active()
            .cache
            .as_ref()
            .map(|cache| cache.accounts.keys().copied().collect())
            .unwrap_or_default();

        Self {
            backend,
            spec_id: executor.spec_id().into(),
            chain_id: overrides
                .and_then(|overrides| overrides.chain_id)
                .unwrap_or(evm_env.chain_id()),
            memory_limit: evm_env.version.memory_limit,
            gas_limit: executor.gas_limit(),
            max_code_size: evm_env.version.max_code_size,
            active_fork_id: backend.active_fork_id(),
            account_addresses,
            block: ExecutorBlock {
                coinbase: block.beneficiary,
                timestamp: block.timestamp,
                number: block.number,
                difficulty,
                gas_limit: block.gas_limit,
                basefee: overrides
                    .and_then(|overrides| overrides.basefee_override())
                    .map_or(block.basefee, U256::from),
                blob_basefee: block.blob_basefee,
                slot_num: block.slot_num,
            },
            gas_price: overrides
                .and_then(|overrides| overrides.gas_price_override())
                .or_else(|| cheats.and_then(|cheats| cheats.gas_price))
                .unwrap_or_else(|| {
                    tx.gas_price().or_else(|| tx.max_fee_per_gas()).unwrap_or_default()
                }),
            isolate: cheats.is_some_and(|cheats| cheats.core.config.isolate),
            internal_expect_revert: cheats
                .is_some_and(|cheats| cheats.core.config.internal_expect_revert),
            assertions_revert: cheats.is_none_or(|cheats| cheats.core.config.assertions_revert),
            blob_hashes: overrides
                .and_then(|overrides| overrides.blob_hashes.clone())
                .unwrap_or_else(|| tx.blob_versioned_hashes().unwrap_or_default().to_vec()),
            arbitrary_storage_targets: cheats
                .into_iter()
                .flat_map(|cheats| cheats.arbitrary_storage_target_overwrite_modes())
                .collect(),
            arbitrary_storage_copies: cheats
                .into_iter()
                .flat_map(|cheats| cheats.arbitrary_storage_copied_target_sources())
                .collect(),
            storage_load_hooks: cheats
                .into_iter()
                .flat_map(|cheats| cheats.storage_load_hooks())
                .map(|(target, hook)| {
                    (
                        target,
                        SymbolicStorageHook {
                            callback_target: hook.callback_target,
                            callback_selector: hook.callback_selector,
                        },
                    )
                })
                .collect(),
            storage_store_hooks: cheats
                .into_iter()
                .flat_map(|cheats| cheats.storage_store_hooks())
                .map(|(target, hook)| {
                    (
                        target,
                        SymbolicStorageHook {
                            callback_target: hook.callback_target,
                            callback_selector: hook.callback_selector,
                        },
                    )
                })
                .collect(),
            mapping_storage_store_hooks: cheats
                .into_iter()
                .flat_map(|cheats| cheats.mapping_storage_store_hooks())
                .map(|(target, root, hook)| {
                    (
                        (target, root.into()),
                        SymbolicStorageHook {
                            callback_target: hook.callback_target,
                            callback_selector: hook.callback_selector,
                        },
                    )
                })
                .collect(),
            cheatcodes: cheats.map(|cheats| &cheats.core),
            pranks: cheats.into_iter().flat_map(|cheats| cheats.pranks.values().copied()).collect(),
            accesses: cheats.map(|cheats| &cheats.accesses),
            recording_accesses: cheats.is_some_and(|cheats| cheats.recording_accesses),
            known_contracts: None,
            literals: None,
            expects_safe_memory: cheats.is_some_and(|cheats| !cheats.allowed_mem_writes.is_empty()),
            persistent_accounts: backend.lifecycle().forks().persistent_accounts().collect(),
        }
    }

    /// Attaches the project artifacts and source literals that register the constant-folded
    /// mapping slots of deployed contracts before a run.
    pub const fn with_project(
        mut self,
        known_contracts: &'a ContractsByArtifact,
        literals: &'a LiteralsDictionary,
    ) -> Self {
        self.known_contracts = Some(known_contracts);
        self.literals = Some(literals);
        self
    }
}
