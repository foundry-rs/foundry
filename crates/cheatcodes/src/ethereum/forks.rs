//! Fork-owned state and persistent accounts for native Ethereum execution.

use super::{ApplyError, Cheatcodes, snapshot::SnapshotFrame};
use crate::{CheatsConfig, Error};
use alloy_consensus::BlockHeader;
use alloy_primitives::{Address, B256, Bytes, U256, map::AddressSet};
use evm2::{
    Evm, ExecutionConfig, Precompiles, SpecId, Version,
    env::BlockEnvExt,
    ethereum::ethereum_tx_registry,
    evm::{
        BEACON_ROOTS_ADDRESS, DbResult, HISTORY_STORAGE_ADDRESS, State, StateSnapshot, SystemTx,
    },
};
use foundry_common::block_on;
use foundry_evm_core::{
    constants::{CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER, TEST_CONTRACT_ADDRESS},
    ethereum::{EthereumFork, FoundryContext, FoundryEvmTypes, RpcDatabase, SavedState},
    opts::EvmOpts,
};
use std::{collections::BTreeMap, fmt, mem, sync::Arc};

#[derive(Clone)]
pub(super) struct Fork {
    pub(super) database: RpcDatabase,
    pub(super) incarnation: u64,
    pub(super) state: Option<Arc<StateSnapshot>>,
    pub(super) needs_finalization: bool,
    block: BlockEnvExt,
    context: FoundryContext,
    pub(super) opts: EvmOpts,
}

impl fmt::Debug for Fork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fork").field("block", &self.block).finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(super) struct Forks {
    pub(super) forks: BTreeMap<U256, Fork>,
    pub(super) active: Option<U256>,
    pub(super) revision: u64,
    next_incarnation: u64,
    pub(super) persistent: AddressSet,
    pub(super) cheatcode_access: AddressSet,
}

impl Default for Forks {
    fn default() -> Self {
        Self {
            forks: BTreeMap::new(),
            active: None,
            revision: 0,
            next_incarnation: 1,
            cheatcode_access: AddressSet::from_iter([
                CALLER,
                CHEATCODE_ADDRESS,
                TEST_CONTRACT_ADDRESS,
            ]),
            persistent: AddressSet::from_iter([
                CALLER,
                CHEATCODE_ADDRESS,
                DEFAULT_CREATE2_DEPLOYER,
            ]),
        }
    }
}

impl fmt::Debug for Forks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Forks")
            .field("active", &self.active)
            .field("count", &self.forks.len())
            .field("persistent", &self.persistent)
            .finish()
    }
}

impl Cheatcodes {
    /// Registers the source fork before test-contract deployment and setup.
    pub fn initialize_fork(&mut self, fork: &EthereumFork) {
        self.forks.forks.insert(
            U256::ZERO,
            Fork {
                database: fork.database.clone(),
                incarnation: 0,
                state: None,
                needs_finalization: false,
                block: fork.env.block,
                context: FoundryContext {
                    chain_id_override: Some(fork.env.version.chain_id),
                    basefee_override: Some(fork.env.block.basefee),
                    gas_price_override: Some(fork.env.gas_price),
                    ..Default::default()
                },
                opts: self.config.evm_opts.clone(),
            },
        );
        self.forks.active = Some(U256::ZERO);
    }
}

impl Forks {
    pub(super) fn identity(&self) -> Option<(U256, u64)> {
        self.active.map(|id| (id, self.forks.get(&id).expect("active fork").incarnation))
    }

    pub(super) fn active_url(&self) -> Option<String> {
        self.active.and_then(|id| self.forks.get(&id)).and_then(|fork| fork.opts.fork_url.clone())
    }

    pub(super) fn snapshot(&self) -> Option<(U256, Fork)> {
        self.active.map(|id| (id, self.forks.get(&id).expect("active fork").clone()))
    }

    pub(super) fn restore(&mut self, snapshot: Option<(U256, Fork)>) -> Option<RpcDatabase> {
        if let Some((id, fork)) = snapshot {
            let database = fork.database.clone();
            self.forks.insert(id, fork);
            self.active = Some(id);
            Some(database)
        } else {
            self.active = None;
            None
        }
    }

    pub(super) fn create(
        &mut self,
        config: &CheatsConfig,
        url: &str,
        block: Option<u64>,
    ) -> Result<U256, ApplyError> {
        let endpoint = config.rpc_endpoint(url)?;
        let mut opts = config.evm_opts.clone();
        opts.fork_url = Some(endpoint.url().map_err(|error| Error::from(error.to_string()))?);
        opts.fork_block_number = block;
        opts.expected_fork_endpoint = None;
        opts.fork_endpoint = None;
        opts.fork_block_number_is_inferred = false;
        // Resolve source metadata afresh; execution still uses the caller's configured Ethereum
        // spec.
        let fork = block_on(EthereumFork::open(&foundry_config::Config::default(), &opts, None))
            .map_err(|error| Error::from(error.to_string()))?;
        let id = U256::from(self.forks.len());
        let incarnation = self.next_incarnation;
        self.next_incarnation = self.next_incarnation.wrapping_add(1);
        self.forks.insert(
            id,
            Fork {
                database: fork.database,
                incarnation,
                state: None,
                needs_finalization: false,
                block: fork.env.block,
                context: FoundryContext {
                    chain_id_override: Some(fork.env.version.chain_id),
                    basefee_override: Some(fork.env.block.basefee),
                    gas_price_override: Some(fork.env.gas_price),
                    ..Default::default()
                },
                opts,
            },
        );
        Ok(id)
    }

    pub(super) fn roll(
        &mut self,
        host: &mut Evm<'_, FoundryEvmTypes>,
        id: Option<U256>,
        block: u64,
        caller: Address,
        frames: &mut [SnapshotFrame],
    ) -> Result<(), ApplyError> {
        let id = id.or(self.active).ok_or_else(|| Error::from("no active fork"))?;
        let source = self
            .forks
            .get(&id)
            .ok_or_else(|| Error::from(format!("Requested fork with id `{id}` does not exist")))?;
        let mut opts = source.opts.clone();
        opts.fork_block_number = Some(block);
        opts.expected_fork_endpoint = None;
        opts.fork_endpoint = None;
        opts.fork_block_number_is_inferred = false;
        let fork = block_on(EthereumFork::open(&foundry_config::Config::default(), &opts, None))
            .map_err(|error| Error::from(error.to_string()))?;
        if self.active == Some(id) {
            SavedState::materialize_initial(host.state_mut());
            SnapshotFrame::capture_source(frames, host, self.identity());
        }
        let incarnation = self.next_incarnation;
        self.next_incarnation = self.next_incarnation.wrapping_add(1);
        self.forks.insert(
            id,
            Fork {
                database: fork.database,
                incarnation,
                state: None,
                needs_finalization: false,
                block: fork.env.block,
                context: FoundryContext {
                    chain_id_override: Some(fork.env.version.chain_id),
                    basefee_override: Some(fork.env.block.basefee),
                    gas_price_override: Some(fork.env.gas_price),
                    ..Default::default()
                },
                opts,
            },
        );
        self.persistent.insert(caller);
        if self.active == Some(id) {
            // Rolling discards this fork's writes, retaining only persistent accounts.
            self.active = None;
            self.select(host, id, caller, frames)?;
        }
        Ok(())
    }

    pub(super) fn select(
        &mut self,
        host: &mut Evm<'_, FoundryEvmTypes>,
        id: U256,
        caller: Address,
        frames: &mut [SnapshotFrame],
    ) -> Result<(), ApplyError> {
        if !self.forks.contains_key(&id) {
            return Err(Error::from(format!("Requested fork with id `{id}` does not exist")).into());
        }
        self.persistent.insert(caller);
        if self.active == Some(id) {
            return Ok(());
        }
        SavedState::materialize_initial(host.state_mut());
        SnapshotFrame::capture_source(frames, host, self.identity());
        if let Some(active) = self.active {
            let source = self.forks.get_mut(&active).expect("active fork");
            source.state = Some(Arc::new(host.state().snapshot()));
            source.needs_finalization = true;
            source.block = *host.block();
            source.context = *host.ext();
        }
        let prewarm = host.state().prewarm_set().clone();
        let target = self.forks.get(&id).expect("validated fork");
        let database = target.database.clone().with_spec(host.spec_id());
        let mut next = match &target.state {
            Some(state) => state.as_ref().clone().into_state(database),
            None => State::new(database),
        };
        self.merge_persistent(host.state(), &mut next);
        let origin = host.ext().transaction_origin;
        let override_origin = host.ext().origin_override;
        let mut block = target.block;
        block.gas_limit = host.block().gas_limit;
        host.set_block(block);
        *host.ext_mut() = target.context;
        host.ext_mut().transaction_origin = origin;
        host.ext_mut().origin_override = override_origin;
        // Receipt logs belong to the executing transaction, not to a fork's suspended state.
        // Resume its current ordered log stream rather than replaying the target's saved logs.
        *next.logs_mut() = mem::take(host.state_mut().logs_mut());
        next.set_prewarm_set(prewarm);
        if host.feature(evm2::EvmFeatures::EIP3651) {
            next.prewarm(&block.beneficiary);
        }
        *host.state_mut() = next;
        self.active = Some(id);
        self.revision = self.revision.wrapping_add(1);
        SnapshotFrame::capture_entry(frames, host, self);
        Ok(())
    }

    /// Finalizes suspended forks once, after the enclosing transaction's rollback scopes end.
    pub(super) fn finalize_inactive(&mut self, spec: SpecId, version: &Version) -> DbResult<()> {
        for (id, fork) in &mut self.forks {
            if Some(*id) != self.active && fork.needs_finalization {
                let saved = fork.state.take().expect("suspended fork state");
                let mut state =
                    Arc::unwrap_or_clone(saved).into_state(fork.database.clone().with_spec(spec));
                state.finalize_transaction(version)?;
                state.commit_transaction();
                state.clear_transaction_state();
                fork.state = Some(Arc::new(state.snapshot()));
                fork.needs_finalization = false;
            }
        }
        Ok(())
    }

    pub(super) fn create_at_transaction(
        &mut self,
        config: &CheatsConfig,
        url: &str,
        hash: B256,
        host: &Evm<'_, FoundryEvmTypes>,
    ) -> Result<U256, ApplyError> {
        let endpoint = config.rpc_endpoint(url)?;
        let mut opts = config.evm_opts.clone();
        opts.fork_url = Some(endpoint.url().map_err(|error| Error::from(error.to_string()))?);
        opts.fork_block_number = None;
        opts.expected_fork_endpoint = None;
        opts.fork_endpoint = None;
        opts.fork_block_number_is_inferred = false;
        let (mut fork, prefix, _) = block_on(EthereumFork::transaction_position(
            &foundry_config::Config::default(),
            &opts,
            hash,
        ))
        .map_err(|error| Error::from(error.to_string()))?;
        let spec = host.spec_id();
        fork.database = fork.database.with_spec(spec);
        let mut version = *host.version();
        version.chain_id = fork.env.version.chain_id;
        let mut replay = Evm::<FoundryEvmTypes>::new_with_execution_config_and_ext(
            ExecutionConfig::for_spec_and_version(spec, version),
            spec,
            fork.env.block,
            ethereum_tx_registry(spec),
            fork.database.clone(),
            Precompiles::base(spec),
            FoundryContext::default(),
        );
        let header = &fork.source_block().header;
        if spec >= SpecId::CANCUN
            && let Some(root) = header.parent_beacon_block_root()
        {
            let _ = replay
                .system_call(SystemTx::new(
                    BEACON_ROOTS_ADDRESS,
                    Bytes::copy_from_slice(root.as_slice()),
                ))
                .map_err(|error| Error::from(error.to_string()))?
                .commit();
        }
        if spec >= SpecId::PRAGUE {
            let _ = replay
                .system_call(SystemTx::new(
                    HISTORY_STORAGE_ADDRESS,
                    Bytes::copy_from_slice(header.parent_hash().as_slice()),
                ))
                .map_err(|error| Error::from(error.to_string()))?
                .commit();
        }
        for tx in prefix {
            replay.ext_mut().transaction_origin = Some(tx.signer());
            let _ = replay.transact(&tx).map_err(|error| Error::from(error.to_string()))?.commit();
        }
        let id = U256::from(self.forks.len());
        let incarnation = self.next_incarnation;
        self.next_incarnation = self.next_incarnation.wrapping_add(1);
        self.forks.insert(
            id,
            Fork {
                database: fork.database,
                incarnation,
                state: Some(Arc::new(replay.state().snapshot())),
                needs_finalization: false,
                block: fork.env.block,
                context: FoundryContext {
                    chain_id_override: Some(fork.env.version.chain_id),
                    basefee_override: Some(fork.env.block.basefee),
                    gas_price_override: Some(fork.env.gas_price),
                    ..Default::default()
                },
                opts,
            },
        );
        Ok(id)
    }

    pub(super) fn roll_to_transaction(
        &mut self,
        config: &CheatsConfig,
        host: &mut Evm<'_, FoundryEvmTypes>,
        id: Option<U256>,
        hash: B256,
        caller: Address,
        frames: &mut [SnapshotFrame],
    ) -> Result<(), ApplyError> {
        let id = id.or(self.active).ok_or_else(|| Error::from("no active fork"))?;
        let source = self
            .forks
            .get(&id)
            .ok_or_else(|| Error::from(format!("Requested fork with id `{id}` does not exist")))?;
        let url = source.opts.fork_url.clone().expect("fork source URL");
        let temporary = self.create_at_transaction(config, &url, hash, host)?;
        let fork = self.forks.remove(&temporary).expect("prepared transaction fork");
        if self.active == Some(id) {
            SavedState::materialize_initial(host.state_mut());
            SnapshotFrame::capture_source(frames, host, self.identity());
        }
        self.forks.insert(id, fork);
        self.persistent.insert(caller);
        if self.active == Some(id) {
            self.active = None;
            self.select(host, id, caller, frames)?;
        }
        Ok(())
    }
}

impl Forks {
    pub(super) fn merge_persistent(&self, source_state: &State<'_>, target_state: &mut State<'_>) {
        for address in &self.persistent {
            let source = &source_state.overlay_db().cache;
            let target = &mut target_state.overlay_db_mut().cache;
            if let Some(account) = source.accounts.get(address) {
                target.accounts.insert(*address, account.clone());
                if let Some(info) = account
                    && let Some(code) = source.contracts.get(&info.code_hash)
                {
                    target.contracts.insert(info.code_hash, code.clone());
                }
            }
            if let Some(storage) = source.storage.get(address) {
                let target = target.storage.entry(*address).or_default();
                if storage.wiped {
                    target.wipe();
                }
                target.wiped = storage.wiped;
                target.slots.extend(storage.slots.iter().map(|(key, value)| (*key, *value)));
            }
            target_state.merge_transaction_account_from(address, source_state);
        }
    }
}
