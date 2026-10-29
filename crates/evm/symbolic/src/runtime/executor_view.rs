use super::*;
use foundry_evm::core::backend::DatabaseError;

/// Network-independent, read-only view of a concrete Foundry executor.
///
/// The view is built by the caller of the public entry points so the symbolic executor does not
/// have to be monomorphized for every [`FoundryEvmNetwork`].
pub struct ExecutorView<'a> {
    pub(crate) backend: &'a dyn DatabaseRef<Error = DatabaseError>,
    pub(crate) spec_id: SpecId,
    pub(crate) chain_id: u64,
    pub(crate) memory_limit: u64,
    pub(crate) max_code_size: usize,
    pub(crate) active_fork_id: Option<U256>,
    /// Addresses cached by the in-memory database and the active fork database.
    pub(crate) account_addresses: Vec<Address>,
    pub(crate) block: ExecutorBlock,
    pub(crate) gas_price: u128,
    pub(crate) isolate: bool,
    /// Whether `vm.expectRevert` may be satisfied by an internal call below the next call.
    pub(crate) internal_expect_revert: bool,
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
            internal_expect_revert: cheats
                .is_some_and(|cheats| cheats.core.config.internal_expect_revert),
            blob_hashes: executor.tx_env().blob_versioned_hashes().to_vec(),
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
        }
    }
}
