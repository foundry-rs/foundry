use crate::substrate_node::service::{
    Backend,
    storage::{CodeInfo, ReviveAccountInfo, SystemAccountInfo, well_known_keys},
};
use alloy_primitives::{Address, Bytes};
use codec::{Decode, Encode};
use lru::LruCache;
use parking_lot::Mutex;
use polkadot_sdk::{
    parachains_common::{AccountId, Hash, opaque::Block},
    sc_client_api::{Backend as BackendT, StateBackend, TrieCacheContext},
    sc_client_db::BlockchainDb,
    sp_blockchain,
    sp_core::{H160, H256},
    sp_io::hashing::blake2_256,
    sp_state_machine::{StorageKey, StorageValue},
};
use std::{collections::HashMap, num::NonZeroUsize, sync::Arc};
use substrate_runtime::Balance;

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("Inner client error: {0}")]
    Client(#[from] sp_blockchain::Error),
    #[error("Could not find total issuance in the state")]
    MissingTotalIssuance,
    #[error("Could not find chain id in the state")]
    MissingChainId,
    #[error("Unable to decode total issuance {0}")]
    DecodeTotalIssuance(codec::Error),
    #[error("Unable to decode chain id {0}")]
    DecodeChainId(codec::Error),
    #[error("Unable to decode balance {0}")]
    DecodeBalance(codec::Error),
    #[error("Unable to decode revive account info {0}")]
    DecodeReviveAccountInfo(codec::Error),
    #[error("Unable to decode system account info {0}")]
    DecodeSystemAccountInfo(codec::Error),
    #[error("Unable to decode revive code info {0}")]
    DecodeCodeInfo(codec::Error),
}

type Result<T> = std::result::Result<T, BackendError>;

pub struct BackendWithOverlay {
    backend: Arc<Backend>,
    overrides: Arc<Mutex<StorageOverrides>>,
}

impl BackendWithOverlay {
    pub fn new(backend: Arc<Backend>, overrides: Arc<Mutex<StorageOverrides>>) -> Self {
        Self { backend, overrides }
    }

    pub fn blockchain(&self) -> &BlockchainDb<Block> {
        self.backend.blockchain()
    }

    pub fn read_chain_id(&self, hash: Hash) -> Result<u64> {
        let key = well_known_keys::CHAIN_ID;

        let value = self.read_top_state(hash, key.to_vec())?.ok_or(BackendError::MissingChainId)?;
        u64::decode(&mut &value[..]).map_err(BackendError::DecodeChainId)
    }

    pub fn read_total_issuance(&self, hash: Hash) -> Result<Balance> {
        let key = well_known_keys::TOTAL_ISSUANCE;

        let value =
            self.read_top_state(hash, key.to_vec())?.ok_or(BackendError::MissingTotalIssuance)?;
        Balance::decode(&mut &value[..]).map_err(BackendError::DecodeTotalIssuance)
    }

    pub fn read_system_account_info(
        &self,
        hash: Hash,
        account_id: AccountId,
    ) -> Result<Option<SystemAccountInfo>> {
        let key = well_known_keys::system_account_info(account_id);

        self.read_top_state(hash, key)?
            .map(|value| {
                SystemAccountInfo::decode(&mut &value[..])
                    .map_err(BackendError::DecodeSystemAccountInfo)
            })
            .transpose()
    }

    pub fn read_revive_account_info(
        &self,
        hash: Hash,
        address: Address,
    ) -> Result<Option<ReviveAccountInfo>> {
        let key = well_known_keys::revive_account_info(H160::from_slice(address.as_slice()));

        self.read_top_state(hash, key)?
            .map(|value| {
                ReviveAccountInfo::decode(&mut &value[..])
                    .map_err(BackendError::DecodeReviveAccountInfo)
            })
            .transpose()
    }

    pub fn read_code_info(&self, hash: Hash, code_hash: H256) -> Result<Option<CodeInfo>> {
        let key = well_known_keys::code_info(code_hash);

        self.read_top_state(hash, key)?
            .map(|value| CodeInfo::decode(&mut &value[..]).map_err(BackendError::DecodeCodeInfo))
            .transpose()
    }

    pub fn inject_system_account_info(
        &self,
        at: Hash,
        account_id: AccountId,
        value: SystemAccountInfo,
    ) {
        let mut overrides = self.overrides.lock();
        overrides.set_system_account_info(at, account_id, value);
    }

    pub fn inject_chain_id(&self, at: Hash, chain_id: u64) {
        let mut overrides = self.overrides.lock();
        overrides.set_chain_id(at, chain_id);
    }

    pub fn inject_total_issuance(&self, at: Hash, value: Balance) {
        let mut overrides = self.overrides.lock();
        overrides.set_total_issuance(at, value);
    }

    pub fn inject_revive_account_info(&self, at: Hash, address: Address, info: ReviveAccountInfo) {
        let mut overrides = self.overrides.lock();
        overrides.set_revive_account_info(at, address, info);
    }

    pub fn inject_pristine_code(&self, at: Hash, code_hash: H256, code: Option<Bytes>) {
        let mut overrides = self.overrides.lock();
        overrides.set_pristine_code(at, code_hash, code);
    }

    pub fn inject_code_info(&self, at: Hash, code_hash: H256, code_info: Option<CodeInfo>) {
        let mut overrides = self.overrides.lock();
        overrides.set_code_info(at, code_hash, code_info);
    }

    pub fn inject_child_storage(
        &self,
        at: Hash,
        child_key: StorageKey,
        key: StorageKey,
        value: StorageValue,
    ) {
        let mut overrides = self.overrides.lock();
        overrides.set_child_storage(at, child_key, key, value);
    }

    fn read_top_state(&self, hash: Hash, key: StorageKey) -> Result<Option<StorageValue>> {
        let maybe_overridden_val = {
            let mut guard = self.overrides.lock();

            guard.per_block.get(&hash).and_then(|overrides| overrides.top.get(&key).cloned())
        };

        if let Some(overridden_val) = maybe_overridden_val {
            return Ok(overridden_val);
        }

        let state = self.backend.state_at(hash, TrieCacheContext::Trusted)?;
        Ok(state
            .storage(key.as_slice())
            .map_err(|e| sp_blockchain::Error::from_state(Box::new(e)))?)
    }
}

pub type Storage = HashMap<StorageKey, Option<StorageValue>>;

#[derive(Default, Clone)]
pub struct BlockOverrides {
    pub top: Storage,
    pub children: HashMap<StorageKey, Storage>,
}

pub struct StorageOverrides {
    // We keep N most recently used block state overrides because we may later get RPC calls which
    // query the state of past blocks. When state is mutated by the `set_*` RPCs, it gets committed
    // to the state DB only in the next block.
    per_block: LruCache<Hash, BlockOverrides>,
}

impl Default for StorageOverrides {
    fn default() -> Self {
        Self { per_block: LruCache::new(NonZeroUsize::new(10).expect("10 is greater than 0")) }
    }
}

impl StorageOverrides {
    pub fn get(&mut self, block: &Hash) -> Option<BlockOverrides> {
        self.per_block.get(block).cloned()
    }

    fn set_chain_id(&mut self, latest_block: Hash, id: u64) {
        let mut changeset = BlockOverrides::default();
        changeset.top.insert(well_known_keys::CHAIN_ID.to_vec(), Some(id.encode()));

        self.add(latest_block, changeset);
    }

    #[allow(unused)]
    fn set_timestamp(&mut self, latest_block: Hash, timestamp: u64) {
        let mut changeset = BlockOverrides::default();
        changeset.top.insert(well_known_keys::TIMESTAMP.to_vec(), Some(timestamp.encode()));

        self.add(latest_block, changeset);
    }

    fn set_system_account_info(
        &mut self,
        latest_block: Hash,
        account_id: AccountId,
        info: SystemAccountInfo,
    ) {
        let mut changeset = BlockOverrides::default();
        changeset.top.insert(well_known_keys::system_account_info(account_id), Some(info.encode()));

        self.add(latest_block, changeset);
    }

    fn set_total_issuance(&mut self, latest_block: Hash, value: Balance) {
        let mut changeset = BlockOverrides::default();
        changeset.top.insert(well_known_keys::TOTAL_ISSUANCE.to_vec(), Some(value.encode()));

        self.add(latest_block, changeset);
    }

    fn set_revive_account_info(
        &mut self,
        latest_block: Hash,
        address: Address,
        info: ReviveAccountInfo,
    ) {
        let mut changeset = BlockOverrides::default();
        changeset.top.insert(
            well_known_keys::revive_account_info(H160::from_slice(address.as_slice())),
            Some(info.encode()),
        );

        self.add(latest_block, changeset);
    }

    fn set_pristine_code(&mut self, latest_block: Hash, code_hash: H256, code: Option<Bytes>) {
        let mut changeset = BlockOverrides::default();

        changeset
            .top
            .insert(well_known_keys::pristine_code(code_hash), code.map(|code| code.0.encode()));

        self.add(latest_block, changeset);
    }

    fn set_code_info(&mut self, latest_block: Hash, code_hash: H256, code_info: Option<CodeInfo>) {
        let mut changeset = BlockOverrides::default();

        changeset.top.insert(
            well_known_keys::code_info(code_hash),
            code_info.map(|code_info| code_info.encode()),
        );

        self.add(latest_block, changeset);
    }

    fn set_child_storage(
        &mut self,
        latest_block: Hash,
        child_key: StorageKey,
        key: StorageKey,
        value: StorageValue,
    ) {
        let mut changeset = BlockOverrides::default();

        let mut child_map = Storage::with_capacity(1);
        child_map.insert(blake2_256(key.as_slice()).to_vec(), Some(value));

        changeset.children.insert(child_key, child_map);

        self.add(latest_block, changeset);
    }

    fn add(&mut self, latest_block: Hash, changeset: BlockOverrides) {
        if let Some(per_block) = self.per_block.get_mut(&latest_block) {
            per_block.top.extend(changeset.top);

            for (child_key, child_map) in changeset.children {
                per_block.children.entry(child_key).or_default().extend(child_map);
            }
        } else {
            self.per_block.put(latest_block, changeset);
        }
    }
}
