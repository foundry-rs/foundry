use crate::{block_env::BlockEnvSnapshot, state::AnvilState, time::TimeSnapshot};
use alloy_consensus::{BlockHeader, Header};
use alloy_primitives::{B256, Sealable, U256};
use parking_lot::RwLock;
use reth_ethereum::primitives::SealedHeader;
use std::{collections::BTreeMap, fmt::Debug, sync::Arc};

/// Everything a snapshot restores besides the chain head.
#[derive(Clone, Debug)]
pub struct Snapshot<H = Header> {
    /// The chain head at the time of the snapshot.
    pub header: SealedHeader<H>,
    /// The anvil state writes at the time of the snapshot.
    pub state: AnvilState,
    /// The time manager settings at the time of the snapshot.
    pub time: TimeSnapshot,
    /// The block environment overrides at the time of the snapshot.
    pub block_env: BlockEnvSnapshot,
}

/// Tracks snapshot ids and the state they restore.
#[derive(Debug)]
pub struct SnapshotManager<H = Header> {
    inner: Arc<RwLock<Snapshots<H>>>,
}

impl<H> Clone for SnapshotManager<H> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<H> Default for SnapshotManager<H> {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Snapshots {
                next_id: U256::ZERO,
                snapshots: BTreeMap::new(),
            })),
        }
    }
}

#[derive(Debug)]
struct Snapshots<H> {
    next_id: U256,
    snapshots: BTreeMap<U256, Snapshot<H>>,
}

impl<H: BlockHeader + Sealable + Clone + Debug> SnapshotManager<H> {
    /// Stores a snapshot and returns its id.
    pub fn insert(&self, snapshot: Snapshot<H>) -> U256 {
        let mut inner = self.inner.write();
        let id = inner.next_id;
        inner.next_id += U256::ONE;
        inner.snapshots.insert(id, snapshot);
        id
    }

    /// Removes the snapshot with the given id and every later snapshot, and returns it.
    pub fn take(&self, id: U256) -> Option<Snapshot<H>> {
        let mut inner = self.inner.write();
        let snapshot = inner.snapshots.remove(&id)?;
        inner.snapshots.retain(|snapshot_id, _| *snapshot_id < id);
        Some(snapshot)
    }

    /// Removes every snapshot.
    pub fn clear(&self) {
        self.inner.write().snapshots.clear();
    }

    /// Returns the block number and hash of every snapshot.
    pub fn metadata(&self) -> BTreeMap<U256, (u64, B256)> {
        self.inner
            .read()
            .snapshots
            .iter()
            .map(|(id, snapshot)| (*id, (snapshot.header.number(), snapshot.header.hash())))
            .collect()
    }
}
