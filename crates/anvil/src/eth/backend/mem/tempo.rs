//! Tempo consensus owned by the canonical in-memory chain.
//!
//! Mining checks consensus metadata before committing execution and signs the final header in the
//! same storage write guard that publishes its block. Cache commits and state-root computation do
//! not hold the storage lock, so readers can still access the published chain.
//!
//! Imports that change canonical history start an unsigned signing segment. Database snapshots
//! remain usable, but reverting across that import boundary starts a new segment at the restored
//! height. Snapshots within the current segment restore consensus exactly.
//!
//! All live consensus mutations, including rotation RPCs, must acquire the database write
//! lock before the storage lock. Mining retains that database guard from preparation through
//! publication, preventing snapshots, rewinds, resets, imports, or rotations from changing the
//! prepared height or key schedule. Only initialization before the backend is shared and private
//! staged copies are exempt; publishing a staged copy still requires the database write lock.
//! Consensus readers acquire the database read lock before storage.
//!
//! Historical queries can use the trusted fork provider at or below its pinned height. They
//! snapshot the provider, root, and known canonical hash together with local history, then release
//! all guards before network I/O. Upstream blocks must match that canonical hash when known.
//! An overlapping reset can complete while the request returns the old snapshot;
//! upstream responses are never cached into the replacement chain. Upstream committees remain
//! independent of the local development identity. Height-bounded forwarding does not establish
//! an authenticated committee or ancestry chain from upstream history to locally signed blocks.
//! Startup and fork resets validate the pinned native root before committing a replacement. Its
//! context supplies the local successor's actual parent view; epoch transitions reset to view zero.
//! Non-genesis remote roots without context, including pre-T4 roots, are rejected. Native contexts
//! must agree with the configured height-based epoch schedule. Imported unsigned execution-only
//! checkpoints still use the local height-derived round convention.
//!
//! Locally mined fork headers seal the same partial account and storage trie used by proofs.
//! Its snapshots live in a separate bounded history cache, sharing the execution history's cache
//! settings but preserving local trie membership through disk round trips. Only a canonical local
//! certificate can select a snapshot. Rewinds remove orphaned proof states. Resets clear proof
//! history before a replacement signing history starts.

use super::{
    Backend,
    state::StateRootCache,
    storage::{BlockchainStorage, InMemoryBlockStates},
};
use crate::{
    NodeConfig,
    config::DEFAULT_TEMPO_EPOCH_LENGTH,
    eth::{
        backend::{
            db::MaybeFullDatabase,
            fork::ClientFork,
            tempo_consensus::{
                TempoConsensus, TempoConsensusError, TempoConsensusIdentity, TempoFinalization,
            },
        },
        error::BlockchainError,
        tempo_consensus::TempoCertifiedBlock,
    },
};
use alloy_consensus::{BlockHeader, Sealable, proofs::calculate_transaction_root};
use alloy_network::Network;
use alloy_primitives::{
    Address, B256, U256,
    map::{AddressMap, HashMap},
};
use alloy_provider::Provider;
use alloy_rpc_types::EIP1186AccountProofResponse as AccountProof;
use anvil_core::eth::block::{Block, canonical_block};
use anvil_rpc::error::{ErrorCode, RpcError};
use foundry_primitives::FoundryHeader;
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};
use revm::database::DbAccount;
use std::{fmt, sync::Arc};
use tempo_hardfork::TempoHardfork;
use tempo_primitives::TempoHeader;

/// Local certificates, key schedule, and consensus snapshots for the current signing history.
#[derive(Clone)]
pub(crate) struct TempoChain {
    consensus: TempoConsensus,
    latest_block: Option<Block>,
    snapshots: HashMap<U256, TempoSnapshot>,
    /// Immutable local tries sealed by locally mined fork blocks, with bounded retention.
    pub(super) proof_states: Arc<RwLock<InMemoryBlockStates>>,
    /// Incremental trie of the latest sealed account set, updated by diffing the next seal.
    pub(super) proof_root: Arc<Mutex<(StateRootCache, AddressMap<DbAccount>)>>,
}

/// Consensus and the sealed head trie retained by an explicit EVM snapshot.
#[derive(Clone)]
struct TempoSnapshot {
    consensus: TempoConsensus,
    latest_block: Option<Block>,
    proof_state: Option<Arc<AddressMap<DbAccount>>>,
}

impl TempoChain {
    pub(super) fn new(consensus: TempoConsensus) -> Self {
        Self {
            consensus,
            latest_block: None,
            snapshots: Default::default(),
            proof_states: Default::default(),
            proof_root: Default::default(),
        }
    }

    /// Resets local trust at an unsigned chain head and clears saved consensus histories.
    pub(super) fn reanchor(&mut self, consensus: TempoConsensus) {
        self.consensus = consensus;
        self.latest_block = None;
        self.proof_states.write().clear();
        *self.proof_root.lock() = Default::default();
        self.snapshots.clear();
    }

    pub(super) fn snapshot(&mut self, id: U256, proof_state: Option<Arc<AddressMap<DbAccount>>>) {
        self.snapshots.insert(
            id,
            TempoSnapshot {
                consensus: self.consensus.clone(),
                latest_block: self.latest_block.clone(),
                proof_state,
            },
        );
    }

    /// Restores saved consensus, or starts unsigned if an import discarded that history.
    ///
    /// The caller prepares the saved database anchor, which may differ from the imported head,
    /// and restores the returned proof state after releasing the chain storage lock.
    pub(super) fn restore(
        &mut self,
        id: U256,
        anchor: TempoConsensus,
    ) -> Option<Arc<AddressMap<DbAccount>>> {
        let TempoSnapshot { consensus, latest_block, proof_state } =
            self.snapshots.remove(&id).unwrap_or_else(|| TempoSnapshot {
                consensus: anchor,
                latest_block: None,
                proof_state: None,
            });
        self.consensus = consensus;
        self.latest_block = latest_block;
        self.snapshots.retain(|snapshot_id, _| *snapshot_id < id);
        proof_state
    }

    /// Rewinds signed history or starts a new unsigned segment below the imported anchor.
    pub(super) fn unwind(&mut self, block: &Block) -> Result<(), BlockchainError> {
        let height = block.header.number();
        let is_current =
            self.consensus.latest().is_some_and(|(current_height, _)| current_height == height);
        if height < self.consensus.anchor_height() {
            // Exact snapshots retain their own controller and anchor across ordinary rollbacks.
            self.consensus = self.prepare_anchor(&block.header)?;
        } else {
            self.consensus
                .unwind(height)
                .map_err(|error| BlockchainError::Internal(error.to_string()))?;
        }
        if !is_current {
            self.latest_block = self.consensus.latest().map(|_| block.clone());
        }
        Ok(())
    }

    /// Checks every fallible consensus input before execution is committed.
    pub(super) fn prepare(
        &self,
        header: &mut TempoHeader,
        hardfork: TempoHardfork,
    ) -> Result<(), BlockchainError> {
        self.consensus
            .prepare_header(header, hardfork)
            .map_err(|error| BlockchainError::Internal(error.to_string()))
    }

    /// Signs a prepared header while its block is published under the same storage write guard.
    ///
    /// The caller must retain the database write guard from preparation through publication and
    /// only install the execution state root in the prepared header, using the same hardfork.
    pub(super) fn finalize(&mut self, header: &TempoHeader, hardfork: TempoHardfork) {
        self.consensus.finalize(header, hardfork).expect(
            "the database write lock preserves the prepared height and committee; metadata and hardfork are unchanged",
        );
    }

    /// Retains the latest complete body independently of historical transaction pruning.
    pub(super) fn publish(&mut self, block: &Block) {
        self.latest_block = Some(block.clone());
    }

    /// Prepares an unsigned segment from a native head before any canonical state is changed.
    pub(super) fn prepare_anchor(
        &self,
        header: &FoundryHeader,
    ) -> Result<TempoConsensus, BlockchainError> {
        // Legacy execution-only dumps are unsigned checkpoints, with no inherited round.
        let native;
        let header = if let Some(header) = header.as_tempo() {
            header
        } else {
            native = FoundryHeader::tempo(header.inner().clone());
            native.as_tempo().expect("the unsigned checkpoint is a Tempo header")
        };
        self.consensus
            .reanchored(header)
            .map_err(|error| BlockchainError::Internal(error.to_string()))
    }

    /// Fetches and validates the pinned fork round before acquiring canonical commit locks.
    pub(super) async fn fork_consensus(
        fork: &ClientFork,
        config: &NodeConfig,
    ) -> Result<TempoConsensus, BlockchainError> {
        let header = fork.root_header().await?.ok_or(BlockchainError::BlockNotFound)?;
        // Execution-only genesis fixtures share the unsigned genesis convention. Nonzero
        // contextless roots are still rejected by the controller.
        let header = match header {
            FoundryHeader::Tempo(_) => header,
            FoundryHeader::Ethereum(inner) => FoundryHeader::tempo(inner),
        };
        TempoConsensus::at_fork_header(
            config.tempo_consensus_seed(),
            config
                .tempo_epoch_length
                .or(fork.config.read().endpoint_identity.tempo_epoch_length)
                .unwrap_or(DEFAULT_TEMPO_EPOCH_LENGTH),
            header.as_tempo().expect("the fork anchor is a Tempo header"),
        )
        .map_err(|error| {
            let message = if matches!(error, TempoConsensusError::InvalidAnchorEpoch { .. }) {
                format!(
                    "{error}; set --tempo.epoch-length to match the fork source \
                     (default: 21600 when source metadata is unavailable)"
                )
            } else {
                error.to_string()
            };
            BlockchainError::Internal(message)
        })
    }

    /// Keeps the known parent round when a replay prefix starts an unsigned local segment.
    pub(super) fn prepare_unsigned_successor(&self) -> Result<TempoConsensus, BlockchainError> {
        self.consensus
            .unsigned_successor()
            .map_err(|error| BlockchainError::Internal(error.to_string()))
    }
}

impl fmt::Debug for TempoChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TempoChain")
            .field("identity", &self.consensus.identity())
            .finish_non_exhaustive()
    }
}

impl<N: Network> Backend<N> {
    /// Returns the currently active public development committee.
    pub async fn tempo_consensus_identity(
        &self,
    ) -> Result<TempoConsensusIdentity, BlockchainError> {
        let _db = self.db.read().await;
        let storage = self.blockchain.storage.read();
        Ok(tempo_chain(&storage)?.consensus.identity())
    }

    /// Returns a local certificate, or pinned fork history when no local certificate exists.
    pub async fn tempo_finalization(
        &self,
        height: Option<u64>,
    ) -> Result<TempoCertifiedBlock, BlockchainError> {
        self.tempo_certified_block(height).await?.ok_or_else(|| {
            consensus_unavailable(if height.is_some() { "certificate" } else { "certifications" })
        })
    }

    /// Returns local latest, falling back to the pinned upstream height before local mining.
    ///
    /// Standalone unsigned history returns `None`; upstream errors remain errors.
    pub async fn tempo_latest_finalization(
        &self,
    ) -> Result<Option<TempoCertifiedBlock>, BlockchainError> {
        self.tempo_certified_block(None).await
    }

    /// Schedules a development identity change without racing canonical mining or rewinds.
    pub async fn tempo_rotate_consensus_identity(
        &self,
    ) -> Result<TempoConsensusIdentity, BlockchainError> {
        let _mining = self.mining.lock().await;
        let _db = self.db.write().await;
        let mut storage = self.blockchain.storage.write();
        storage
            .tempo
            .as_mut()
            .ok_or_else(tempo_unavailable)?
            .consensus
            .rotate()
            .map_err(|error| BlockchainError::Internal(error.to_string()))
    }

    async fn tempo_certified_block(
        &self,
        height: Option<u64>,
    ) -> Result<Option<TempoCertifiedBlock>, BlockchainError> {
        let (provider, height, root, hash, canonical_hash) = {
            let _db = self.db.read().await;
            let storage = self.blockchain.storage.read();
            let tempo = tempo_chain(&storage)?;
            let local = match height {
                Some(height) => tempo
                    .consensus
                    .finalization(height)
                    .map(|finalization| {
                        let block = storage
                            .hashes
                            .get(&height)
                            .and_then(|hash| storage.blocks.get(hash))
                            .ok_or_else(|| consensus_unavailable("block"))?;
                        certified_block(finalization, block)
                    })
                    .transpose()?,
                None => latest_finalization(tempo)?,
            };
            if let Some(local) = local {
                return local.try_into().map(Some);
            }
            let Some(fork) = self.get_fork() else { return Ok(None) };
            let config = fork.config.read();
            let height = height.unwrap_or(config.block_number);
            if height > config.block_number {
                return Ok(None);
            }
            // Capture one fork snapshot while reset is excluded; release guards before network I/O.
            (
                config.provider.clone(),
                height,
                config.block_number,
                config.block_hash,
                storage.hashes.get(&height).copied(),
            )
        };
        let certified = provider
            .raw_request::<_, TempoCertifiedBlock>(
                "consensus_getFinalization".into(),
                (serde_json::json!({ "height": height }),),
            )
            .await?;
        if certified.block.header.number() != height
            || certified.block.header.hash_slow() != certified.finalization.digest
            || (height == root && certified.finalization.digest != hash)
            || canonical_hash.is_some_and(|hash| certified.finalization.digest != hash)
            || tempo_evm::consensus::validate_body_against_header(
                &certified.block.body,
                &certified.block.header,
            )
            .is_err()
        {
            return Err(consensus_unavailable(
                "upstream finalization does not match the pinned fork history",
            ));
        }
        Ok(Some(certified))
    }

    /// Proves against the immutable local trie committed by a locally certified fork block.
    pub(super) async fn prove_tempo_account_at(
        &self,
        address: Address,
        keys: &[B256],
        height: u64,
    ) -> Result<Option<AccountProof>, BlockchainError> {
        let db = self.db.read().await;
        if self.get_fork().is_none() {
            return Ok(None);
        }
        let (proof_states, hash) = {
            let storage = self.blockchain.storage.read();
            let Some(tempo) = &storage.tempo else { return Ok(None) };
            let Some(finalization) = tempo.consensus.finalization(height) else { return Ok(None) };
            let hash =
                storage.hashes.get(&height).copied().ok_or(BlockchainError::DataUnavailable)?;
            if finalization.digest != hash {
                return Err(BlockchainError::DataUnavailable);
            }
            (Arc::clone(&tempo.proof_states), hash)
        };
        drop(db);

        // Canonical selection uses the database guard; release it before loading and hashing a
        // historical trie so mining and other database writers are not blocked by proof work.
        let states = proof_states.upgradable_read();
        if let Some(state) = states.get_state(&hash) {
            let accounts = state.maybe_as_full_db().ok_or(BlockchainError::DataUnavailable)?;
            return Ok(Some(super::prove_account(accounts, address, keys)));
        }

        let mut states = RwLockUpgradableReadGuard::upgrade(states);
        // Loading a disk state mutates the cache, so the write guard is required only for the
        // load. Clone the loaded account map once, then release the exclusive guard before the
        // potentially expensive trie construction.
        let accounts = states
            .get_on_disk_state(&hash)
            .and_then(|state| state.maybe_as_full_db())
            .ok_or(BlockchainError::DataUnavailable)?
            .clone();
        drop(states);
        Ok(Some(super::prove_account(&accounts, address, keys)))
    }
}

fn tempo_chain<N: Network>(storage: &BlockchainStorage<N>) -> Result<&TempoChain, BlockchainError> {
    storage.tempo.as_ref().ok_or_else(tempo_unavailable)
}

fn latest_finalization(
    tempo: &TempoChain,
) -> Result<Option<(TempoFinalization, Block)>, BlockchainError> {
    let Some((_, finalization)) = tempo.consensus.latest() else { return Ok(None) };
    let block = tempo.latest_block.as_ref().ok_or_else(|| consensus_unavailable("block"))?;
    certified_block(finalization, block).map(Some)
}

fn certified_block(
    finalization: &TempoFinalization,
    block: &Block,
) -> Result<(TempoFinalization, Block), BlockchainError> {
    let block = canonical_block(block.clone());
    if block.header.hash_slow() != finalization.digest
        || calculate_transaction_root(&block.body.transactions) != block.header.transactions_root()
        || block.body.calculate_ommers_root() != block.header.ommers_hash()
        || block.body.calculate_withdrawals_root() != block.header.withdrawals_root()
    {
        return Err(consensus_unavailable("block"));
    }
    Ok((finalization.clone(), block))
}

fn consensus_unavailable(content: &str) -> BlockchainError {
    RpcError {
        code: ErrorCode::from(503),
        message: "the consensus subservice was not available, but the request can be retried later"
            .into(),
        data: Some(serde_json::Value::String(content.to_owned())),
    }
    .into()
}

fn tempo_unavailable() -> BlockchainError {
    RpcError::invalid_params("Tempo consensus is only available on the Tempo network").into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        eth::{
            api::EthApi,
            backend::{
                db::{SerializableState, StateDb},
                mem::MiningCommitHook,
            },
        },
        spawn,
    };
    use alloy_primitives::keccak256;
    use alloy_rpc_types::{BlockId, BlockNumberOrTag, TransactionRequest, anvil::Forking};
    use alloy_serde::WithOtherFields;
    use alloy_trie::{Nibbles, TrieAccount, proof::verify_proof};
    use anvil_core::types::ReorgOptions;
    use foundry_evm::{
        backend::MemDb,
        hardfork::{EthereumHardfork, FoundryHardfork},
    };
    use foundry_primitives::FoundryNetwork;
    use revm::database::AccountState;
    use std::{
        num::NonZeroU64,
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };
    use tokio::{sync::Notify, time::timeout};

    fn consensus(api: &EthApi<FoundryNetwork>) -> TempoConsensus {
        api.backend.blockchain.storage.read().tempo.as_ref().unwrap().consensus.clone()
    }

    fn assert_certified_head(api: &EthApi<FoundryNetwork>) {
        let consensus = consensus(api);
        let (height, certificate) = consensus.latest().unwrap();
        let block = api.backend.get_block(height).unwrap();
        assert_eq!(height, api.backend.best_number());
        assert_eq!(certificate.digest, block.header.hash_slow());
        assert_eq!(certificate.digest, api.backend.best_hash());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tempo_snapshot_waits_for_complete_block_publication() {
        let (api, _handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        let hook =
            MiningCommitHook { reached: Arc::new(Notify::new()), resume: Arc::new(Notify::new()) };
        *api.backend.mining_commit_hook.lock() = Some(hook.clone());
        let mining_api = api.clone();
        let mining = tokio::spawn(async move { mining_api.mine_one().await });
        timeout(Duration::from_secs(10), hook.reached.notified()).await.unwrap();
        assert_eq!(api.backend.best_number(), 0);
        assert!(consensus(&api).latest().is_none());
        assert!(api.backend.get_block(1).is_none());
        let mut snapshot = Box::pin(api.evm_snapshot());
        assert!(futures::poll!(snapshot.as_mut()).is_pending());
        let mut certificate = Box::pin(api.backend.tempo_finalization(None));
        let mut identity = Box::pin(api.backend.tempo_consensus_identity());
        let mut latest = Box::pin(api.backend.tempo_latest_finalization());
        let mut rotation = Box::pin(api.backend.tempo_rotate_consensus_identity());
        assert!(futures::poll!(certificate.as_mut()).is_pending());
        assert!(futures::poll!(identity.as_mut()).is_pending());
        assert!(futures::poll!(latest.as_mut()).is_pending());
        assert!(futures::poll!(rotation.as_mut()).is_pending());
        hook.resume.notify_one();
        let snapshot = timeout(Duration::from_secs(10), async {
            mining.await.unwrap().unwrap();
            let (snapshot, certificate, identity, latest, rotation) =
                tokio::join!(snapshot, certificate, identity, latest, rotation);
            let certificate = certificate.unwrap();
            assert_eq!(certificate.block.header.number(), 1);
            assert_eq!(certificate.block.header.hash_slow(), certificate.finalization.digest);
            assert_eq!(api.backend.best_hash(), certificate.finalization.digest);
            assert_eq!(latest.unwrap().unwrap().finalization, certificate.finalization);
            identity.unwrap();
            rotation.unwrap();
            snapshot.unwrap()
        })
        .await
        .unwrap();
        *api.backend.mining_commit_hook.lock() = None;
        assert_certified_head(&api);
        api.mine_one().await.unwrap();
        assert!(api.evm_revert(snapshot).await.unwrap());
        assert_eq!(api.backend.best_number(), 1);
        assert_certified_head(&api);
    }

    #[tokio::test]
    async fn tempo_state_root_keeps_published_storage_readable() {
        let (api, _handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        api.mine_one().await.unwrap();
        let storage = Arc::clone(&api.backend.blockchain.storage);
        let reached = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&reached);
        *api.backend.mining_state_root_hook.lock() = Some(Box::new(move || {
            let storage = storage.try_read().expect("state-root computation must not lock storage");
            let (height, certificate) = storage.tempo.as_ref().unwrap().consensus.latest().unwrap();
            assert_eq!(storage.best_number, 1);
            assert_eq!(height, storage.best_number);
            assert_eq!(certificate.digest, storage.best_hash);
            assert!(storage.blocks.contains_key(&certificate.digest));
            observed.store(true, Ordering::Relaxed);
        }));
        api.mine_one().await.unwrap();
        assert!(reached.load(Ordering::Relaxed));
        assert_certified_head(&api);
    }

    #[tokio::test]
    async fn tempo_rollback_publishes_consensus_and_head_together() {
        let (api, _handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        api.anvil_mine(Some(U256::from(2)), None).await.unwrap();
        let storage = Arc::clone(&api.backend.blockchain.storage);
        let reached = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&reached);
        *api.backend.rollback_unwind_hook.lock() = Some(Box::new(move || {
            assert!(storage.try_read().is_none(), "a partial rollback must not be visible");
            observed.store(true, Ordering::Relaxed);
        }));
        api.anvil_rollback(Some(1)).await.unwrap();
        assert!(reached.load(Ordering::Relaxed));
        assert_eq!(api.backend.best_number(), 1);
        assert!(consensus(&api).finalization(2).is_none());
        api.anvil_rollback(Some(0)).await.unwrap();
        assert_certified_head(&api);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejected_consensus_preparation_does_not_commit_execution() {
        let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        let sender = handle.dev_wallets().next().unwrap().address();
        queue_transfer(&api, sender).await;
        let previous_hash = api.backend.best_hash();
        let previous_root = api.backend.db.read().await.maybe_state_root();
        {
            let _db = api.backend.db.write().await;
            let mut storage = api.backend.blockchain.storage.write();
            let tempo = storage.tempo.as_mut().unwrap();
            let anchor = tempo.prepare_anchor(&unsigned_tempo_block(5).header).unwrap();
            tempo.reanchor(anchor);
        }
        let error = api.mine_one().await.unwrap_err();
        assert!(matches!(error, BlockchainError::Internal(message)
            if message == "expected Tempo consensus height 6, got 1"));
        assert_eq!(api.backend.best_number(), 0);
        assert_eq!(api.backend.best_hash(), previous_hash);
        assert_eq!(api.backend.db.read().await.maybe_state_root(), previous_root);
        assert!(consensus(&api).latest().is_none());
        {
            let _db = api.backend.db.write().await;
            let mut storage = api.backend.blockchain.storage.write();
            let tempo = storage.tempo.as_mut().unwrap();
            let anchor = tempo.prepare_anchor(&unsigned_tempo_block(0).header).unwrap();
            tempo.reanchor(anchor);
        }
        api.mine_one().await.unwrap();
        assert_eq!(api.backend.get_block(1).unwrap().body.transactions.len(), 1);
        assert_certified_head(&api);
    }

    #[tokio::test]
    async fn tempo_snapshot_restores_pending_committee_and_counter() {
        let (api, _handle) = spawn(tempo_config(3)).await;
        api.mine_one().await.unwrap();
        let first = consensus(&api).finalization(1).unwrap().digest;
        let scheduled = rotate_consensus(&api).await;
        let snapshot = api.evm_snapshot().await.unwrap();
        let replaced = rotate_consensus(&api).await;
        assert_ne!(scheduled, replaced);
        assert!(api.evm_revert(snapshot).await.unwrap());
        assert_eq!(consensus(&api).finalization(1).unwrap().digest, first);
        // The exact snapshot retains the pending key and derivation counter.
        let mut restored = consensus(&api);
        assert_eq!(restored.rotate().unwrap(), replaced);
        api.mine_one().await.unwrap();
        api.mine_one().await.unwrap();
        assert_eq!(consensus(&api).identity(), scheduled);
        assert_certified_head(&api);
    }

    #[tokio::test]
    async fn tempo_reorg_and_reset_preserve_lifecycle() {
        let (api, _handle) = spawn(tempo_config(3)).await;
        let initial = consensus(&api).identity();
        api.anvil_mine(Some(U256::from(3)), None).await.unwrap();
        let first = consensus(&api).finalization(1).unwrap().digest;
        let previous = consensus(&api).finalization(3).unwrap().digest;
        api.anvil_set_coinbase(Address::repeat_byte(0x33)).await.unwrap();
        api.anvil_reorg(ReorgOptions { depth: 2, tx_block_pairs: Vec::new() }).await.unwrap();
        assert_eq!(consensus(&api).finalization(1).unwrap().digest, first);
        assert_ne!(consensus(&api).finalization(3).unwrap().digest, previous);
        assert_certified_head(&api);
        let stale = api.evm_snapshot().await.unwrap();
        api.anvil_reset(None).await.unwrap();
        assert!(api.backend.list_state_snapshots().is_empty());
        assert!(!api.evm_revert(stale).await.unwrap());
        let fresh = api.evm_snapshot().await.unwrap();
        assert_eq!(fresh, stale);
        assert!(consensus(&api).latest().is_none());
        assert_eq!(consensus(&api).identity(), initial);
        api.mine_one().await.unwrap();
        assert_certified_head(&api);
        assert!(api.evm_revert(fresh).await.unwrap());
        assert_eq!(api.backend.best_number(), 0);
        assert!(consensus(&api).latest().is_none());
        api.mine_one().await.unwrap();
        assert_certified_head(&api);
    }

    #[tokio::test]
    async fn cross_import_snapshot_restores_state_and_resumes_mining() {
        for tempo in [false, true] {
            for (saved_height, imported_height) in [(2, 1), (1, 2)] {
                let config = if tempo { NodeConfig::test_tempo() } else { NodeConfig::test() };
                let (api, handle) = spawn(config.with_no_mining(true)).await;
                let account = Address::repeat_byte(0x22);
                let sender = handle.dev_wallets().next().unwrap().address();
                api.anvil_set_balance(account, U256::from(2)).await.unwrap();
                let mut state = None;
                let mut saved = None;
                for height in 1..=2 {
                    api.mine_one().await.unwrap();
                    if height == imported_height {
                        state = Some(api.anvil_dump_state(None).await.unwrap());
                    }
                    if height == saved_height {
                        api.anvil_set_balance(account, U256::ONE).await.unwrap();
                        let transaction = queue_transfer(&api, sender).await;
                        saved = Some((
                            api.evm_snapshot().await.unwrap(),
                            api.backend.best_hash(),
                            transaction,
                        ));
                        api.anvil_set_balance(account, U256::from(2)).await.unwrap();
                        api.anvil_drop_all_transactions().await.unwrap();
                    }
                }
                api.mine_one().await.unwrap();
                let (snapshot, saved_hash, transaction) = saved.unwrap();

                assert!(api.anvil_load_state(state.unwrap()).await.unwrap());
                assert_eq!(api.backend.best_number(), imported_height);
                assert_eq!(api.balance(account, None).await.unwrap(), U256::from(2));
                assert!(api.transaction_by_hash(transaction).await.unwrap().is_none());
                assert!(api.evm_revert(snapshot).await.unwrap());
                assert_eq!(api.backend.best_number(), saved_height);
                assert_eq!(api.backend.best_hash(), saved_hash);
                assert_eq!(api.balance(account, None).await.unwrap(), U256::ONE);
                assert!(api.transaction_by_hash(transaction).await.unwrap().is_some());
                if tempo {
                    assert!(consensus(&api).latest().is_none());
                }
                api.mine_one().await.unwrap();
                assert_eq!(api.backend.best_number(), saved_height + 1);
                let block = api.backend.get_block(saved_height + 1).unwrap();
                assert_eq!(block.header.parent_hash(), saved_hash);
                assert_eq!(block.body.transactions.len(), 1);
                if tempo {
                    assert_certified_head(&api);
                }
            }
        }
    }

    #[tokio::test]
    async fn tempo_pending_and_native_projection_follow_executed_hardfork() {
        for hardfork in [
            TempoHardfork::T3.into(),
            TempoHardfork::T4.into(),
            FoundryHardfork::Ethereum(EthereumHardfork::Prague),
        ] {
            let (api, _handle) =
                spawn(NodeConfig::test_tempo().with_no_mining(true).with_hardfork(Some(hardfork)))
                    .await;
            for _ in 0..2 {
                api.block_by_number(BlockNumberOrTag::Pending).await.unwrap().unwrap();
                api.header_by_number(BlockNumberOrTag::Pending).await.unwrap().unwrap();
            }
            assert!(consensus(&api).latest().is_none());
            api.mine_one().await.unwrap();
            assert_certified_head(&api);
            let block = api.backend.get_block(1).unwrap();
            let native = block.header.as_tempo().unwrap();
            assert_eq!(native.consensus_context.is_some(), api.backend.tempo_hardfork().is_t4());
            for projected in [
                api.block_by_number(1.into()).await.unwrap().unwrap(),
                api.backend.block_by_hash(block.header.hash_slow()).await.unwrap().unwrap(),
            ] {
                let projected: TempoHeader =
                    serde_json::from_value(serde_json::to_value(projected).unwrap()).unwrap();
                assert_eq!(&projected, native);
            }
        }
        let (api, _handle) = spawn(NodeConfig::test().with_no_mining(true)).await;
        api.mine_one().await.unwrap();
        assert!(api.backend.blockchain.storage.read().tempo.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tempo_fork_startup_and_reset_sign_only_local_successors() {
        let config = tempo_config(3);
        let (source_api, source) = spawn(config.clone()).await;
        source_api.anvil_mine(Some(U256::from(4)), None).await.unwrap();
        let (api, _handle) = spawn(
            config
                .with_eth_rpc_url(Some(source.http_endpoint()))
                .with_fork_block_number(Some(2u64))
                .with_tempo_consensus_seed(Some(B256::repeat_byte(0x42))),
        )
        .await;
        assert!(consensus(&api).latest().is_none());
        api.mine_one().await.unwrap();
        assert_certified_head(&api);
        assert_eq!(
            api.backend.get_block(3).unwrap().header.parent_hash(),
            source_api.backend.get_block(2).unwrap().header.hash_slow()
        );
        assert_eq!(consensus(&api).latest().unwrap().1.epoch, 1);
        let stale = api.evm_snapshot().await.unwrap();
        api.anvil_reset(Some(Forking {
            json_rpc_url: Some(source.http_endpoint()),
            block_number: Some(4),
        }))
        .await
        .unwrap();
        assert!(api.backend.list_state_snapshots().is_empty());
        assert!(!api.evm_revert(stale).await.unwrap());
        let fresh = api.evm_snapshot().await.unwrap();
        assert_eq!(fresh, stale);
        assert!(consensus(&api).latest().is_none());
        assert_eq!(consensus(&api).identity().from_epoch, 1);
        api.mine_one().await.unwrap();
        assert_certified_head(&api);
        assert_eq!(api.backend.best_number(), 5);

        let sender = source.dev_wallets().next().unwrap().address();
        let hash = queue_transfer(&source_api, sender).await;
        source_api.mine_one().await.unwrap();
        let (replay, _replay_handle) = spawn(
            tempo_config(3)
                .with_eth_rpc_url(Some(source.http_endpoint()))
                .with_fork_transaction_hash(Some(hash)),
        )
        .await;
        assert_eq!(replay.backend.best_number(), 5);
        assert!(consensus(&replay).latest().is_none());
        replay.mine_one().await.unwrap();
        assert_eq!(replay.backend.best_number(), 6);
        assert_certified_head(&replay);
    }

    #[tokio::test]
    async fn tempo_changed_head_import_can_rollback_and_reorg_below_anchor() {
        let (api, _handle) = spawn(tempo_config(3)).await;
        api.anvil_mine(Some(U256::from(4)), None).await.unwrap();
        let imported_head = api.backend.best_hash();
        let ancestor = api.backend.get_block(2).unwrap().header.hash_slow();
        let state = api.backend.serialized_state(false).await.unwrap();
        assert!(state.historical_states.is_none());
        api.mine_one().await.unwrap();
        assert_ne!(api.backend.best_hash(), imported_head);
        api.backend.load_state(state.clone()).await.unwrap();
        assert_eq!(api.backend.best_hash(), imported_head);
        assert!(consensus(&api).latest().is_none());

        // The same process retains execution history even when the dump omits it.
        api.anvil_rollback(Some(2)).await.unwrap();
        assert_eq!(api.backend.best_number(), 2);
        assert_eq!(api.backend.best_hash(), ancestor);
        assert!(consensus(&api).latest().is_none());
        api.mine_one().await.unwrap();
        assert_certified_head(&api);

        // Importing the saved head again creates another unsigned signing segment.
        api.backend.load_state(state).await.unwrap();
        assert_eq!(api.backend.best_hash(), imported_head);
        assert!(consensus(&api).latest().is_none());
        api.anvil_reorg(ReorgOptions { depth: 2, tx_block_pairs: Vec::new() }).await.unwrap();
        assert_eq!(api.backend.best_number(), 4);
        assert!(consensus(&api).finalization(2).is_none());
        let replacement = consensus(&api).finalization(3).unwrap().clone();
        assert_eq!(replacement.digest, api.backend.get_block(3).unwrap().header.hash_slow());
        assert_certified_head(&api);
    }

    #[test]
    fn tempo_reanchor_clears_proof_states() {
        let mut chain =
            TempoChain::new(TempoConsensus::at_height(B256::ZERO, NonZeroU64::new(3).unwrap(), 0));
        let hash = B256::repeat_byte(0x42);
        chain.proof_states.write().insert(hash, StateDb::new(MemDb::default()));
        assert!(chain.proof_states.read().get_state(&hash).is_some());

        chain.reanchor(chain.prepare_anchor(&unsigned_tempo_block(3).header).unwrap());

        assert!(chain.proof_states.read().get_state(&hash).is_none());
    }

    #[test]
    fn tempo_cross_anchor_rollback_preserves_exact_snapshot_anchor() {
        let mut chain =
            TempoChain::new(TempoConsensus::at_height(B256::ZERO, NonZeroU64::new(3).unwrap(), 5));
        chain.consensus.rotate().unwrap();
        let next_rotation = chain.consensus.clone().rotate().unwrap();
        chain.unwind(&unsigned_tempo_block(5)).unwrap();
        assert_eq!(chain.consensus.clone().rotate().unwrap(), next_rotation);
        chain.snapshot(U256::ONE, None);
        let mut expected = chain.consensus.clone();

        chain.unwind(&unsigned_tempo_block(3)).unwrap();
        assert!(chain.consensus.latest().is_none());
        let _ = chain
            .restore(U256::ONE, chain.prepare_anchor(&unsigned_tempo_block(5).header).unwrap());
        assert_eq!(chain.consensus.rotate().unwrap(), expected.rotate().unwrap());

        // Restoring the snapshot restores anchor 5, so height 4 is an unsigned ancestor again.
        chain.unwind(&unsigned_tempo_block(4)).unwrap();
        assert!(chain.consensus.latest().is_none());
        let mut header = TempoHeader::default();
        header.inner.number = 5;
        chain.prepare(&mut header, TempoHardfork::T4).unwrap();
        chain.finalize(&header, TempoHardfork::T4);
        assert_eq!(chain.consensus.latest().unwrap().0, 5);
    }

    #[test]
    fn tempo_rollback_above_anchor_preserves_pruning_error() {
        let mut chain =
            TempoChain::new(TempoConsensus::at_height(B256::ZERO, NonZeroU64::new(3).unwrap(), 5));
        for height in 6..=7 {
            let mut header = TempoHeader::default();
            header.inner.number = height;
            chain.prepare(&mut header, TempoHardfork::T4).unwrap();
            chain.finalize(&header, TempoHardfork::T4);
        }
        chain.consensus.prune(7).unwrap();
        let retained = chain.consensus.finalization(7).unwrap().clone();
        let identity = chain.consensus.identity();

        let error = chain.unwind(&unsigned_tempo_block(6)).unwrap_err();
        assert!(matches!(error, BlockchainError::Internal(message)
            if message == "cannot rewind Tempo consensus from 7 to 6 with earliest retained height 7"));
        assert_eq!(chain.consensus.latest(), Some((7, &retained)));
        assert_eq!(chain.consensus.identity(), identity);
        chain.unwind(&unsigned_tempo_block(7)).unwrap();
        assert_eq!(chain.consensus.latest(), Some((7, &retained)));
    }

    #[tokio::test]
    async fn tempo_unchanged_history_import_preserves_consensus_and_snapshots() {
        for include_history in [false, true] {
            let (api, _handle) = spawn(tempo_config(4)).await;
            api.anvil_mine(Some(U256::from(2)), None).await.unwrap();
            let first = consensus(&api).finalization(1).unwrap().clone();
            let head = consensus(&api).finalization(2).unwrap().clone();
            let account = Address::repeat_byte(0x22);
            api.anvil_set_balance(account, U256::ONE).await.unwrap();
            let scheduled = rotate_consensus(&api).await;
            let snapshot = api.evm_snapshot().await.unwrap();
            let next_rotation = consensus(&api).rotate().unwrap();
            let mut state = api.backend.serialized_state(false).await.unwrap();
            let mut invalid = state.clone();
            invalid.best_block_number = Some(99);
            let error = api.backend.load_state(invalid).await.unwrap_err();
            assert!(matches!(error, BlockchainError::RpcError(error)
                if error.message == "Best hash not found for best number 99"));
            assert_eq!(consensus(&api).finalization(2), Some(&head));
            state.accounts.get_mut(&account).unwrap().balance = U256::from(2);
            if !include_history {
                state = SerializableState { accounts: state.accounts, ..Default::default() };
            }

            api.backend.load_state(state).await.unwrap();
            assert_eq!(consensus(&api).finalization(1), Some(&first));
            assert_eq!(consensus(&api).finalization(2), Some(&head));
            assert_eq!(consensus(&api).rotate().unwrap(), next_rotation);
            assert_eq!(api.balance(account, None).await.unwrap(), U256::from(2));
            assert_certified_head(&api);
            api.anvil_mine(Some(U256::from(2)), None).await.unwrap();
            assert_eq!(consensus(&api).identity(), scheduled);
            assert!(api.evm_revert(snapshot).await.unwrap());
            assert_eq!(api.balance(account, None).await.unwrap(), U256::ONE);
            assert_eq!(consensus(&api).finalization(1), Some(&first));
            assert_eq!(consensus(&api).finalization(2), Some(&head));
            assert_eq!(consensus(&api).rotate().unwrap(), next_rotation);
            assert_certified_head(&api);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tempo_fork_unchanged_history_import_preserves_certified_proofs() {
        let config = NodeConfig::test_tempo().with_no_mining(true);
        let (source_api, source) = spawn(config.clone()).await;
        source_api.mine_one().await.unwrap();
        for include_history in [false, true] {
            let (api, _handle) = spawn(tempo_fork_config(config.clone(), &source)).await;
            let account = Address::repeat_byte(0x23);
            let keys = vec![B256::ZERO];
            api.anvil_set_balance(account, U256::ONE).await.unwrap();
            api.anvil_set_storage_at(account, U256::ZERO, B256::with_last_byte(1)).await.unwrap();
            api.mine_one().await.unwrap();

            let first = consensus(&api).finalization(2).unwrap().clone();
            let original =
                api.get_proof(account, keys.clone(), Some(BlockId::number(2))).await.unwrap();
            assert_eq!(original.balance, U256::ONE);
            assert_eq!(original.storage_proof[0].value, U256::ONE);
            api.mine_one().await.unwrap();
            let head = consensus(&api).finalization(3).unwrap().clone();
            let head_proof = api.get_proof(account, keys.clone(), None).await.unwrap();
            let snapshot = api.evm_snapshot().await.unwrap();
            let mut state = api.backend.serialized_state(false).await.unwrap();
            let imported = state.accounts.get_mut(&account).unwrap();
            imported.balance = U256::from(2);
            imported.storage.insert(B256::ZERO, B256::with_last_byte(2));
            if !include_history {
                state = SerializableState { accounts: state.accounts, ..Default::default() };
            }

            api.backend.load_state(state).await.unwrap();
            assert_eq!(consensus(&api).finalization(2), Some(&first));
            assert_eq!(consensus(&api).finalization(3), Some(&head));
            assert_eq!(api.get_proof(account, keys.clone(), None).await.unwrap(), head_proof);
            assert_eq!(
                api.get_proof(account, keys.clone(), Some(BlockId::number(2))).await.unwrap(),
                original
            );

            // Imported state belongs to pending execution and the next certified block.
            assert_eq!(
                api.balance(account, Some(BlockId::pending())).await.unwrap(),
                U256::from(2)
            );
            assert_eq!(
                api.storage_at(account, U256::ZERO, Some(BlockId::pending())).await.unwrap(),
                B256::with_last_byte(2)
            );
            api.mine_one().await.unwrap();
            let updated = api.get_proof(account, keys.clone(), None).await.unwrap();
            assert_eq!(updated.balance, U256::from(2));
            assert_eq!(updated.storage_proof[0].value, U256::from(2));
            assert_certified_head(&api);

            assert!(api.evm_revert(snapshot).await.unwrap());
            assert_eq!(api.balance(account, None).await.unwrap(), U256::ONE);
            assert_eq!(consensus(&api).finalization(2), Some(&first));
            assert_eq!(consensus(&api).finalization(3), Some(&head));
            assert_eq!(api.get_proof(account, keys.clone(), None).await.unwrap(), head_proof);
            assert_eq!(
                api.get_proof(account, keys, Some(BlockId::number(2))).await.unwrap(),
                original
            );
        }
    }

    #[tokio::test]
    async fn tempo_cross_import_snapshot_does_not_restore_changed_ancestor_certificate() {
        let (api, _handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        api.anvil_mine(Some(U256::from(2)), None).await.unwrap();
        let head = api.backend.best_hash();
        let signed_ancestor = consensus(&api).finalization(1).unwrap().digest;
        let snapshot = api.evm_snapshot().await.unwrap();
        let mut state = api.backend.serialized_state(false).await.unwrap();
        let ancestor = state.blocks.iter_mut().find(|block| block.header.number() == 1).unwrap();
        let FoundryHeader::Tempo(header) = &mut ancestor.header else {
            panic!("expected Tempo header");
        };
        header.inner.timestamp += 1;
        let replacement = ancestor.header.hash_slow();
        assert_ne!(replacement, signed_ancestor);

        api.backend.load_state(state).await.unwrap();
        assert_eq!(api.backend.best_hash(), head);
        assert_eq!(api.backend.get_block(1).unwrap().header.hash_slow(), replacement);
        assert!(consensus(&api).latest().is_none());
        assert!(api.evm_revert(snapshot).await.unwrap());
        assert_eq!(api.backend.best_hash(), head);
        assert_eq!(api.backend.get_block(1).unwrap().header.hash_slow(), replacement);
        assert!(consensus(&api).latest().is_none());
        assert!(consensus(&api).finalization(1).is_none());
        api.mine_one().await.unwrap();
        assert_certified_head(&api);
    }

    #[tokio::test]
    async fn tempo_import_preserves_backend_snapshot_created_after_staging() {
        let (api, _handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        api.mine_one().await.unwrap();
        let db = api.backend.db.write().await;

        // Raw backend snapshots use the database guard; RPC snapshots also hold the mining guard.
        // Queue the snapshot first, then stage the import before either can mutate the database.
        let mut snapshot = Box::pin(api.backend.create_state_snapshot());
        assert!(futures::poll!(snapshot.as_mut()).is_pending());
        let mut import = Box::pin(api.backend.load_state(SerializableState::default()));
        assert!(futures::poll!(import.as_mut()).is_pending());
        drop(db);

        let snapshot = timeout(Duration::from_secs(10), snapshot).await.unwrap();
        assert!(timeout(Duration::from_secs(10), import).await.unwrap().unwrap());
        assert!(
            timeout(Duration::from_secs(10), api.backend.revert_state_snapshot(snapshot))
                .await
                .unwrap()
                .unwrap()
        );
        assert_certified_head(&api);
    }

    #[test]
    fn tempo_cross_import_snapshots_preserve_revert_order() {
        let mut chain =
            TempoChain::new(TempoConsensus::at_height(B256::ZERO, NonZeroU64::new(3).unwrap(), 0));
        chain.snapshot(U256::ONE, None);
        chain.reanchor(chain.prepare_anchor(&unsigned_tempo_block(3).header).unwrap());
        assert!(chain.snapshots.is_empty());
        chain.snapshot(U256::from(2), None);
        chain.snapshot(U256::from(3), None);
        let expected = chain.consensus.clone().rotate().unwrap();
        chain.consensus.rotate().unwrap();
        chain.snapshot(U256::from(4), None);
        let _ = chain
            .restore(U256::from(3), chain.prepare_anchor(&unsigned_tempo_block(3).header).unwrap());
        assert_eq!(chain.consensus.rotate().unwrap(), expected);
        assert_eq!(chain.snapshots.keys().copied().collect::<Vec<_>>(), vec![U256::from(2)]);

        // Restoring a pre-import snapshot uses its database height, not the current anchor.
        let _ = chain
            .restore(U256::ONE, chain.prepare_anchor(&unsigned_tempo_block(1).header).unwrap());
        assert!(chain.snapshots.is_empty());
        assert_eq!(chain.consensus.anchor_height(), 1);
        assert!(chain.consensus.latest().is_none());
    }

    fn tempo_config(epoch_length: u64) -> NodeConfig {
        NodeConfig::test_tempo()
            .with_no_mining(true)
            .with_tempo_epoch_length(NonZeroU64::new(epoch_length).unwrap())
    }

    async fn queue_transfer(api: &EthApi<FoundryNetwork>, sender: Address) -> B256 {
        api.send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(sender).to(Address::repeat_byte(0x11)),
        ))
        .await
        .unwrap()
    }

    async fn rotate_consensus(api: &EthApi<FoundryNetwork>) -> TempoConsensusIdentity {
        api.anvil_tempo_rotate_consensus_identity().await.unwrap()
    }

    fn unsigned_tempo_block(height: u64) -> Block {
        let mut header = TempoHeader::default();
        header.inner.number = height;
        Block::new(header.into(), Default::default())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tempo_fork_proof_state_survives_disk_roundtrip() {
        let config = NodeConfig::test_tempo()
            .with_no_mining(true)
            .with_no_storage_caching(true)
            .with_max_persisted_states(Some(4usize));
        let (source_api, source) = spawn(config.clone()).await;
        source_api.mine_one().await.unwrap();
        let (api, _handle) = spawn(tempo_fork_config(config, &source)).await;
        let proof_states = api.backend.tempo_proof_states().unwrap();
        let address = Address::repeat_byte(0x31);
        let absent = Address::repeat_byte(0x32);
        let keys = [B256::ZERO, B256::with_last_byte(1)];
        api.anvil_set_balance(address, U256::from(42)).await.unwrap();
        api.anvil_set_code(address, alloy_primitives::bytes!("60005460005260206000f3"))
            .await
            .unwrap();
        api.anvil_set_storage_at(address, U256::ZERO, B256::with_last_byte(7)).await.unwrap();
        api.backend
            .db
            .write()
            .await
            .maybe_as_full_db_mut()
            .unwrap()
            .insert(absent, DbAccount::new_not_existing());
        api.mine_one().await.unwrap();
        let certified = api.backend.tempo_finalization(Some(2)).await.unwrap();
        let root = certified.block.header.state_root();
        let hash = certified.finalization.digest;
        let original =
            api.backend.prove_tempo_account_at(address, &keys, 2).await.unwrap().unwrap();
        let excluded = api.backend.prove_tempo_account_at(absent, &keys, 2).await.unwrap().unwrap();
        assert_eq!(original.balance, U256::from(42));
        assert_eq!(original.storage_proof[0].value, U256::from(7));
        assert_eq!(original.storage_proof[1].value, U256::ZERO);

        for spilled in [false, true] {
            if spilled {
                api.anvil_set_storage_at(address, U256::ZERO, B256::with_last_byte(9))
                    .await
                    .unwrap();
                api.mine_one().await.unwrap();
                let mut states = proof_states.write();
                states.set_cache_limit(1);
                states.insert(B256::repeat_byte(0x99), StateDb::new(MemDb::default()));
                assert!(states.get_state(&hash).is_none());
                let restored = states.get_on_disk_state(&hash).unwrap();
                assert_eq!(
                    restored.maybe_as_full_db().unwrap()[&absent].account_state,
                    AccountState::NotExisting,
                );
            }
            let proof =
                api.backend.prove_tempo_account_at(address, &keys, 2).await.unwrap().unwrap();
            assert_eq!(proof, original);
            assert_eq!(keccak256(&proof.account_proof[0]), root);
            let account =
                TrieAccount::new(proof.nonce, proof.balance, proof.storage_hash, proof.code_hash);
            verify_proof(
                root,
                Nibbles::unpack(keccak256(address)),
                Some(alloy_rlp::encode(account)),
                &proof.account_proof,
            )
            .unwrap();
            for storage in &proof.storage_proof {
                verify_proof(
                    proof.storage_hash,
                    Nibbles::unpack(keccak256(storage.key.as_b256())),
                    (!storage.value.is_zero()).then(|| alloy_rlp::encode(storage.value)),
                    &storage.proof,
                )
                .unwrap();
            }
            let proof =
                api.backend.prove_tempo_account_at(absent, &keys, 2).await.unwrap().unwrap();
            assert_eq!(proof, excluded);
            verify_proof(root, Nibbles::unpack(keccak256(absent)), None, &proof.account_proof)
                .unwrap();
            let retained = api.backend.tempo_finalization(Some(2)).await.unwrap();
            assert_eq!(retained.finalization, certified.finalization);
            assert_eq!(retained.block.header.state_root(), root);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tempo_fork_snapshot_restores_evicted_proof_state() {
        let config = NodeConfig::test_tempo().with_no_mining(true).set_pruned_history(Some(None));
        let (source_api, source) = spawn(config.clone()).await;
        source_api.mine_one().await.unwrap();
        let (api, handle) =
            spawn(tempo_fork_config(config.with_no_storage_caching(true), &source)).await;
        let provider = handle.http_provider();
        let address = Address::repeat_byte(0x33);
        api.anvil_set_storage_at(address, U256::ZERO, B256::with_last_byte(1)).await.unwrap();
        api.mine_one().await.unwrap();
        let certified = api.backend.tempo_finalization(Some(2)).await.unwrap();
        let original = provider.get_proof(address, vec![B256::ZERO]).await.unwrap();
        assert_eq!(original.storage_proof[0].value, U256::ONE);

        // The execution snapshot includes an unmined edit, while the sealed trie does not.
        api.anvil_set_storage_at(address, U256::ZERO, B256::with_last_byte(9)).await.unwrap();
        let snapshot = api.evm_snapshot().await.unwrap();
        api.mine_one().await.unwrap();
        assert!(api.evm_revert(snapshot).await.unwrap());
        assert_eq!(
            api.backend.tempo_finalization(Some(2)).await.unwrap().finalization,
            certified.finalization
        );
        assert_eq!(provider.get_proof(address, vec![B256::ZERO]).await.unwrap(), original);
        assert_eq!(provider.get_storage_at(address, U256::ZERO).await.unwrap(), U256::from(9));
    }

    fn tempo_fork_config(config: NodeConfig, source: &crate::NodeHandle) -> NodeConfig {
        config.with_eth_rpc_url(Some(source.http_endpoint())).with_fork_block_number(Some(1u64))
    }
}
