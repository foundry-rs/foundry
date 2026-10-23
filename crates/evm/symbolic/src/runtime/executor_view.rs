use super::*;
use foundry_evm::core::backend::DatabaseError;

/// Network-independent, read-only view of a concrete Foundry executor.
///
/// The view is built once at the public entry points so the symbolic executor does not have to be
/// monomorphized for every [`FoundryEvmNetwork`].
pub struct ExecutorView<'a> {
    backend: &'a dyn DatabaseRef<Error = DatabaseError>,
    spec_id: SpecId,
    chain_id: u64,
    memory_limit: u64,
    max_code_size: usize,
    active_fork_id: Option<U256>,
    account_addresses: Vec<Address>,
    pub(crate) block: ExecutorBlock,
    pub(crate) gas_price: u128,
    pub(crate) isolate: bool,
    pub(crate) blob_hashes: Vec<B256>,
    pub(crate) arbitrary_storage_targets: Vec<(Address, bool)>,
    pub(crate) arbitrary_storage_copies: Vec<(Address, Address)>,
    pub(crate) storage_load_hooks: Vec<(Address, SymbolicStorageHook)>,
    pub(crate) storage_store_hooks: Vec<(Address, SymbolicStorageHook)>,
    pub(crate) mapping_storage_store_hooks: Vec<((Address, U256), SymbolicStorageHook)>,
}

/// Block environment values exposed to symbolic execution.
pub(crate) struct ExecutorBlock {
    pub(crate) coinbase: Address,
    pub(crate) timestamp: U256,
    pub(crate) number: U256,
    pub(crate) difficulty: U256,
    pub(crate) gas_limit: u64,
    pub(crate) basefee: u64,
    pub(crate) blob_basefee: u128,
}

impl<'a> ExecutorView<'a> {
    /// Snapshots the environment, backend, and cheatcode configuration of `executor`.
    pub fn new<FEN: FoundryEvmNetwork>(executor: &'a Executor<FEN>) -> Self {
        let backend = executor.backend();
        let evm_env = executor.evm_env();
        let cheats = executor.inspector().cheatcodes.as_deref();
        let block = cheats.and_then(|cheats| cheats.block.as_ref()).unwrap_or(evm_env.block_env());
        let difficulty = block.prevrandao().map(Into::into).unwrap_or_else(|| block.difficulty());

        let mut account_addresses =
            backend.mem_db().cache.accounts.keys().copied().collect::<Vec<_>>();
        if let Some(db) = backend.active_fork_db() {
            account_addresses.extend(db.cache.accounts.keys().copied());
        }

        Self {
            backend,
            spec_id: executor.spec_id().into(),
            chain_id: evm_env.cfg_env.chain_id,
            memory_limit: evm_env.cfg_env.memory_limit(),
            max_code_size: evm_env.cfg_env.max_code_size(),
            active_fork_id: backend.active_fork_id(),
            account_addresses,
            block: ExecutorBlock {
                coinbase: block.beneficiary(),
                timestamp: block.timestamp(),
                number: block.number(),
                difficulty,
                gas_limit: block.gas_limit(),
                basefee: block.basefee(),
                blob_basefee: block.blob_gasprice().unwrap_or_default(),
            },
            gas_price: cheats
                .and_then(|cheats| cheats.gas_price)
                .unwrap_or_else(|| executor.tx_env().gas_price()),
            isolate: cheats.is_some_and(|cheats| cheats.core.config.isolate),
            blob_hashes: executor.tx_env().blob_versioned_hashes().to_vec(),
            arbitrary_storage_targets: cheats
                .map(|cheats| cheats.arbitrary_storage_target_overwrite_modes().collect())
                .unwrap_or_default(),
            arbitrary_storage_copies: cheats
                .map(|cheats| cheats.arbitrary_storage_copied_target_sources().collect())
                .unwrap_or_default(),
            storage_load_hooks: cheats
                .map(|cheats| {
                    cheats
                        .storage_load_hooks()
                        .map(|(target, h)| {
                            (
                                target,
                                SymbolicStorageHook::new(h.callback_target, h.callback_selector),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            storage_store_hooks: cheats
                .map(|cheats| {
                    cheats
                        .storage_store_hooks()
                        .map(|(target, h)| {
                            (
                                target,
                                SymbolicStorageHook::new(h.callback_target, h.callback_selector),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            mapping_storage_store_hooks: cheats
                .map(|cheats| {
                    cheats
                        .mapping_storage_store_hooks()
                        .map(|(target, root, h)| {
                            (
                                (target, root.into()),
                                SymbolicStorageHook::new(h.callback_target, h.callback_selector),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub(crate) fn backend(&self) -> &dyn DatabaseRef<Error = DatabaseError> {
        self.backend
    }

    pub(crate) const fn spec_id(&self) -> SpecId {
        self.spec_id
    }

    pub(crate) const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub(crate) const fn memory_limit(&self) -> u64 {
        self.memory_limit
    }

    pub(crate) const fn max_code_size(&self) -> usize {
        self.max_code_size
    }

    pub(crate) const fn active_fork_id(&self) -> Option<U256> {
        self.active_fork_id
    }

    pub(crate) fn is_active_fork(&self, id: U256) -> bool {
        self.active_fork_id == Some(id)
    }

    /// Addresses cached by the in-memory database and the active fork database.
    pub(crate) fn account_addresses(&self) -> &[Address] {
        &self.account_addresses
    }
}
