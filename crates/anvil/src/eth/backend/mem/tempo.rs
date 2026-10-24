//! Tempo consensus owned by the canonical in-memory chain.
//!
//! Mining checks consensus metadata before committing execution and signs the final header in the
//! same storage write guard that publishes its block. Cache commits and state-root computation do
//! not hold the storage lock, so readers can still access the published chain.
//!
//! All live consensus mutations, including future rotation RPCs, must acquire the database write
//! lock before the storage lock. Mining retains that database guard from preparation through
//! publication, preventing snapshots, rewinds, resets, imports, or rotations from changing the
//! prepared height or key schedule. Only initialization before the backend is shared and private
//! staged copies are exempt; publishing a staged copy still requires the database write lock.

use crate::eth::{backend::tempo_consensus::TempoConsensus, error::BlockchainError};
use alloy_consensus::BlockHeader;
use alloy_primitives::{B256, U256, map::HashMap};
use anvil_core::eth::block::Block;
use std::{fmt, num::NonZeroU64};
use tempo_hardfork::TempoHardfork;
use tempo_primitives::TempoHeader;

/// Local certificates, key schedule, and exact development snapshots for one canonical chain.
#[derive(Clone)]
pub(crate) struct TempoChain {
    seed: B256,
    epoch_length: NonZeroU64,
    consensus: TempoConsensus,
    snapshots: HashMap<U256, TempoConsensus>,
}

impl TempoChain {
    pub(super) fn new(seed: B256, epoch_length: NonZeroU64, height: u64) -> Self {
        Self {
            seed,
            epoch_length,
            consensus: TempoConsensus::at_height(seed, epoch_length, height),
            snapshots: Default::default(),
        }
    }

    /// Resets local trust at an unsigned imported or replacement chain head.
    pub(super) fn reanchor(&mut self, height: u64) {
        self.consensus = TempoConsensus::at_height(self.seed, self.epoch_length, height);
        self.snapshots.clear();
    }

    pub(super) fn snapshot(&mut self, id: U256) {
        self.snapshots.insert(id, self.consensus.clone());
    }

    pub(super) fn restore(&mut self, id: U256) {
        let snapshot =
            self.snapshots.remove(&id).expect("the database snapshot has consensus state");
        self.consensus = snapshot;
        self.snapshots.retain(|snapshot_id, _| *snapshot_id < id);
    }

    /// Rewinds signed history or starts a new unsigned segment below the imported anchor.
    pub(super) fn unwind(&mut self, block: &Block) -> Result<(), BlockchainError> {
        let height = block.header.number();
        if height < self.consensus.anchor_height() {
            // Exact snapshots retain their own controller and anchor across ordinary rollbacks.
            self.consensus = TempoConsensus::at_height(self.seed, self.epoch_length, height);
        } else {
            self.consensus
                .unwind(height)
                .map_err(|error| BlockchainError::Internal(error.to_string()))?;
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
}

impl fmt::Debug for TempoChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TempoChain")
            .field("identity", &self.consensus.identity())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        NodeConfig,
        eth::{
            api::EthApi,
            backend::{db::SerializableState, mem::MiningCommitHook},
        },
        spawn,
    };
    use alloy_primitives::Address;
    use alloy_rpc_types::{BlockNumberOrTag, TransactionRequest, anvil::Forking};
    use alloy_serde::WithOtherFields;
    use anvil_core::types::ReorgOptions;
    use foundry_evm::{
        backend::RevertStateSnapshotAction,
        hardfork::{EthereumHardfork, FoundryHardfork},
    };
    use foundry_primitives::{FoundryHeader, FoundryNetwork};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
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

    #[tokio::test]
    async fn mined_tempo_header_contains_consensus_context() {
        let (api, _handle) = spawn(
            NodeConfig::test_tempo()
                .with_no_mining(true)
                .with_hardfork(Some(TempoHardfork::T4.into())),
        )
        .await;
        api.mine_one().await.unwrap();
        let block = api.backend.get_block(1).unwrap();
        assert!(block.header.as_tempo().unwrap().consensus_context.is_some());
        assert_certified_head(&api);
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
        hook.resume.notify_one();
        let snapshot = timeout(Duration::from_secs(10), async {
            mining.await.unwrap().unwrap();
            snapshot.await.unwrap()
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
        for _ in 0..2 {
            api.mine_one().await.unwrap();
        }
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
        assert_certified_head(&api);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejected_consensus_preparation_does_not_commit_execution() {
        let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        let sender = handle.dev_wallets().next().unwrap().address();
        api.send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(sender).to(Address::repeat_byte(0x11)),
        ))
        .await
        .unwrap();
        let previous_hash = api.backend.best_hash();
        let previous_root = api.backend.db.read().await.maybe_state_root();
        {
            let _db = api.backend.db.write().await;
            api.backend.blockchain.storage.write().tempo.as_mut().unwrap().reanchor(5);
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
            api.backend.blockchain.storage.write().tempo.as_mut().unwrap().reanchor(0);
        }
        api.mine_one().await.unwrap();
        assert_eq!(api.backend.get_block(1).unwrap().body.transactions.len(), 1);
        assert_certified_head(&api);
    }

    #[tokio::test]
    async fn tempo_snapshot_restores_pending_committee_and_counter() {
        let (api, _handle) = spawn(
            NodeConfig::test_tempo()
                .with_no_mining(true)
                .with_tempo_epoch_length(NonZeroU64::new(3).unwrap()),
        )
        .await;
        api.mine_one().await.unwrap();
        let first = consensus(&api).finalization(1).unwrap().digest;
        let scheduled = {
            let _db = api.backend.db.write().await;
            api.backend
                .blockchain
                .storage
                .write()
                .tempo
                .as_mut()
                .unwrap()
                .consensus
                .rotate()
                .unwrap()
        };
        let snapshot = api.evm_snapshot().await.unwrap();
        let replaced = {
            let _db = api.backend.db.write().await;
            api.backend
                .blockchain
                .storage
                .write()
                .tempo
                .as_mut()
                .unwrap()
                .consensus
                .rotate()
                .unwrap()
        };
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
    async fn tempo_rollback_reorg_and_reset_preserve_lifecycle() {
        let (api, _handle) = spawn(
            NodeConfig::test_tempo()
                .with_no_mining(true)
                .with_tempo_epoch_length(NonZeroU64::new(3).unwrap()),
        )
        .await;
        let initial = consensus(&api).identity();
        for _ in 0..4 {
            api.mine_one().await.unwrap();
        }
        let first = consensus(&api).finalization(1).unwrap().digest;
        api.anvil_rollback(Some(3)).await.unwrap();
        assert_eq!(consensus(&api).finalization(1).unwrap().digest, first);
        assert!(consensus(&api).finalization(2).is_none());
        assert_certified_head(&api);
        api.anvil_rollback(Some(0)).await.unwrap();
        assert_certified_head(&api);
        for _ in 0..2 {
            api.mine_one().await.unwrap();
        }
        let previous = consensus(&api).finalization(3).unwrap().digest;
        api.evm_set_next_block_timestamp(2_000_000_000).unwrap();
        api.anvil_reorg(ReorgOptions { depth: 2, tx_block_pairs: Vec::new() }).await.unwrap();
        assert_eq!(consensus(&api).finalization(1).unwrap().digest, first);
        assert_ne!(consensus(&api).finalization(3).unwrap().digest, previous);
        assert_certified_head(&api);
        api.anvil_reset(None).await.unwrap();
        assert!(consensus(&api).latest().is_none());
        assert_eq!(consensus(&api).identity(), initial);
        api.mine_one().await.unwrap();
        assert_certified_head(&api);
    }

    #[tokio::test]
    async fn tempo_load_state_reanchors_and_invalidates_snapshots() {
        let (api, _handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        api.mine_one().await.unwrap();
        let state = api.backend.serialized_state(false).await.unwrap();
        api.mine_one().await.unwrap();
        let old_snapshot = api.evm_snapshot().await.unwrap();
        api.backend.load_state(state).await.unwrap();
        assert_eq!(api.backend.best_number(), 1);
        assert!(consensus(&api).latest().is_none());
        assert!(api.backend.list_state_snapshots().is_empty());
        assert!(!api.evm_revert(old_snapshot).await.unwrap());
        assert!(
            !api.backend
                .db
                .write()
                .await
                .revert_state(old_snapshot, RevertStateSnapshotAction::RevertRemove)
        );
        let snapshot = api.evm_snapshot().await.unwrap();
        api.mine_one().await.unwrap();
        assert_certified_head(&api);
        assert!(api.evm_revert(snapshot).await.unwrap());
        assert_eq!(api.backend.best_number(), 1);
        assert!(consensus(&api).latest().is_none());
    }

    #[tokio::test]
    async fn tempo_rejected_import_preserves_certificate_and_snapshot() {
        let (api, _handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        api.mine_one().await.unwrap();
        let first = consensus(&api).finalization(1).unwrap().digest;
        let snapshot = api.evm_snapshot().await.unwrap();
        let mut invalid = api.backend.serialized_state(false).await.unwrap();
        invalid.best_block_number = Some(99);
        let error = api.backend.load_state(invalid).await.unwrap_err();
        assert!(matches!(error, BlockchainError::RpcError(error)
            if error.message == "Best hash not found for best number 99"));
        assert_certified_head(&api);
        assert_eq!(consensus(&api).finalization(1).unwrap().digest, first);
        api.mine_one().await.unwrap();
        assert!(api.evm_revert(snapshot).await.unwrap());
        assert_certified_head(&api);
        assert_eq!(consensus(&api).finalization(1).unwrap().digest, first);
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
            let by_number = api.block_by_number(1.into()).await.unwrap().unwrap();
            let projected: TempoHeader =
                serde_json::from_value(serde_json::to_value(by_number).unwrap()).unwrap();
            assert_eq!(&projected, native);
            let by_hash =
                api.backend.block_by_hash(block.header.hash_slow()).await.unwrap().unwrap();
            let projected: TempoHeader =
                serde_json::from_value(serde_json::to_value(by_hash).unwrap()).unwrap();
            assert_eq!(&projected, native);
        }
        let (api, _handle) = spawn(NodeConfig::test().with_no_mining(true)).await;
        api.mine_one().await.unwrap();
        assert!(api.backend.blockchain.storage.read().tempo.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tempo_fork_startup_and_reset_sign_only_local_successors() {
        let config = NodeConfig::test_tempo()
            .with_no_mining(true)
            .with_tempo_epoch_length(NonZeroU64::new(3).unwrap());
        let (source_api, source) = spawn(config.clone()).await;
        for _ in 0..4 {
            source_api.mine_one().await.unwrap();
        }
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
        api.anvil_reset(Some(Forking {
            json_rpc_url: Some(source.http_endpoint()),
            block_number: Some(4),
        }))
        .await
        .unwrap();
        assert!(consensus(&api).latest().is_none());
        assert_eq!(consensus(&api).identity().from_epoch, 1);
        api.mine_one().await.unwrap();
        assert_certified_head(&api);
        assert_eq!(api.backend.best_number(), 5);

        let sender = source.dev_wallets().next().unwrap().address();
        let hash = source_api
            .send_transaction(WithOtherFields::new(
                TransactionRequest::default().from(sender).to(Address::repeat_byte(0x11)),
            ))
            .await
            .unwrap();
        source_api.mine_one().await.unwrap();
        let (replay, _replay_handle) = spawn(
            NodeConfig::test_tempo()
                .with_no_mining(true)
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
        let (api, _handle) = spawn(
            NodeConfig::test_tempo()
                .with_no_mining(true)
                .with_tempo_epoch_length(NonZeroU64::new(3).unwrap()),
        )
        .await;
        for _ in 0..4 {
            api.mine_one().await.unwrap();
        }
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
        let retained = consensus(&api).finalization(3).unwrap().clone();
        api.mine_one().await.unwrap();
        api.anvil_rollback(Some(1)).await.unwrap();
        assert_eq!(consensus(&api).finalization(3), Some(&retained));
        assert!(consensus(&api).finalization(4).is_none());
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
    fn tempo_cross_anchor_rollback_preserves_exact_snapshot_anchor() {
        let mut chain = TempoChain::new(B256::ZERO, NonZeroU64::new(3).unwrap(), 5);
        chain.consensus.rotate().unwrap();
        let next_rotation = chain.consensus.clone().rotate().unwrap();
        chain.unwind(&unsigned_tempo_block(5)).unwrap();
        assert_eq!(chain.consensus.clone().rotate().unwrap(), next_rotation);
        chain.snapshot(U256::ONE);
        let mut expected = chain.consensus.clone();

        chain.unwind(&unsigned_tempo_block(3)).unwrap();
        assert!(chain.consensus.latest().is_none());
        chain.restore(U256::ONE);
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
        let mut chain = TempoChain::new(B256::ZERO, NonZeroU64::new(3).unwrap(), 5);
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
            let (api, _handle) = spawn(
                NodeConfig::test_tempo()
                    .with_no_mining(true)
                    .with_tempo_epoch_length(NonZeroU64::new(4).unwrap()),
            )
            .await;
            for _ in 0..2 {
                api.mine_one().await.unwrap();
            }
            let first = consensus(&api).finalization(1).unwrap().clone();
            let head = consensus(&api).finalization(2).unwrap().clone();
            let account = Address::repeat_byte(0x22);
            api.anvil_set_balance(account, U256::ONE).await.unwrap();
            let scheduled = {
                let _db = api.backend.db.write().await;
                api.backend
                    .blockchain
                    .storage
                    .write()
                    .tempo
                    .as_mut()
                    .unwrap()
                    .consensus
                    .rotate()
                    .unwrap()
            };
            let snapshot = api.evm_snapshot().await.unwrap();
            let next_rotation = consensus(&api).rotate().unwrap();
            let mut state = api.backend.serialized_state(false).await.unwrap();
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
            for _ in 0..2 {
                api.mine_one().await.unwrap();
            }
            assert_eq!(consensus(&api).identity(), scheduled);
            assert_certified_head(&api);

            assert!(api.evm_revert(snapshot).await.unwrap());
            assert_eq!(api.balance(account, None).await.unwrap(), U256::ONE);
            assert_eq!(consensus(&api).finalization(2), Some(&head));
            assert_eq!(consensus(&api).rotate().unwrap(), next_rotation);
            assert_certified_head(&api);
            api.anvil_rollback(Some(1)).await.unwrap();
            assert_eq!(api.backend.best_number(), 1);
            assert_eq!(consensus(&api).finalization(1), Some(&first));
            assert_certified_head(&api);
        }
    }

    #[tokio::test]
    async fn tempo_same_head_import_with_changed_ancestor_invalidates_snapshots() {
        let (api, _handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        for _ in 0..2 {
            api.mine_one().await.unwrap();
        }
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
        assert!(api.backend.list_state_snapshots().is_empty());
        assert!(!api.evm_revert(snapshot).await.unwrap());
        assert!(
            !api.backend
                .db
                .write()
                .await
                .revert_state(snapshot, RevertStateSnapshotAction::RevertRemove)
        );
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

    fn unsigned_tempo_block(height: u64) -> Block {
        let mut header = TempoHeader::default();
        header.inner.number = height;
        Block::new(header.into(), Default::default())
    }
}
