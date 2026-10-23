//! Tempo consensus owned by the canonical in-memory chain.
//!
//! Consensus readers acquire the database lock before storage, matching block publication. Mining
//! prepares all fallible consensus metadata before committing execution, and holds an exclusive
//! borrow of the prepared header and signer until the final state root is installed.

use super::{Backend, storage::BlockchainStorage};
use crate::eth::{
    backend::tempo_consensus::{TempoConsensus, TempoConsensusIdentity, TempoFinalization},
    error::BlockchainError,
};
use alloy_consensus::{BlockHeader, Sealable, proofs::calculate_transaction_root};
use alloy_network::Network;
use alloy_primitives::{B256, U256, map::HashMap};
use anvil_core::eth::block::{Block, canonical_block};
use anvil_rpc::error::{ErrorCode, RpcError};
use std::{fmt, num::NonZeroU64};
use tempo_hardfork::TempoHardfork;
use tempo_primitives::TempoHeader;

/// Local certificates, key schedule, and exact development snapshots for one canonical chain.
#[derive(Clone)]
pub(crate) struct TempoChain {
    seed: B256,
    epoch_length: NonZeroU64,
    consensus: TempoConsensus,
    latest_block: Option<Block>,
    snapshots: HashMap<U256, TempoSnapshot>,
}

impl TempoChain {
    pub(super) fn new(seed: B256, epoch_length: NonZeroU64, height: u64) -> Self {
        Self {
            seed,
            epoch_length,
            consensus: TempoConsensus::at_height(seed, epoch_length, height),
            latest_block: None,
            snapshots: Default::default(),
        }
    }

    /// Resets local trust at an unsigned imported or replacement chain head.
    pub(super) fn reanchor(&mut self, height: u64) {
        self.consensus = TempoConsensus::at_height(self.seed, self.epoch_length, height);
        self.latest_block = None;
        self.snapshots.clear();
    }

    pub(super) fn snapshot(&mut self, id: U256) {
        self.snapshots.insert(
            id,
            TempoSnapshot {
                consensus: self.consensus.clone(),
                latest_block: self.latest_block.clone(),
            },
        );
    }

    pub(super) fn restore(&mut self, id: U256) {
        let snapshot =
            self.snapshots.remove(&id).expect("the database snapshot has consensus state");
        self.consensus = snapshot.consensus;
        self.latest_block = snapshot.latest_block;
        self.snapshots.retain(|snapshot_id, _| *snapshot_id < id);
    }

    pub(super) fn unwind(&mut self, block: &Block) -> Result<(), BlockchainError> {
        let is_current =
            self.consensus.latest().is_some_and(|(height, _)| height == block.header.number());
        self.consensus
            .unwind(block.header.number())
            .map_err(|error| BlockchainError::Internal(error.to_string()))?;
        if !is_current {
            self.latest_block = self.consensus.latest().map(|_| block.clone());
        }
        Ok(())
    }

    /// Checks every fallible consensus input before execution is committed.
    pub(super) fn prepare<'a>(
        &'a mut self,
        header: &'a mut TempoHeader,
        hardfork: TempoHardfork,
    ) -> Result<PreparedTempoHeader<'a>, BlockchainError> {
        self.consensus
            .prepare_header(header, hardfork)
            .map_err(|error| BlockchainError::Internal(error.to_string()))?;
        Ok(PreparedTempoHeader { consensus: &mut self.consensus, header, hardfork })
    }

    /// Retains the latest complete body independently of historical transaction pruning.
    pub(super) fn publish(&mut self, block: &Block) {
        self.latest_block = Some(block.clone());
    }
}

impl fmt::Debug for TempoChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TempoChain")
            .field("identity", &self.consensus.identity())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct TempoSnapshot {
    consensus: TempoConsensus,
    latest_block: Option<Block>,
}

/// Exclusive access prevents the prepared metadata or committee from changing before completion.
pub(super) struct PreparedTempoHeader<'a> {
    consensus: &'a mut TempoConsensus,
    header: &'a mut TempoHeader,
    hardfork: TempoHardfork,
}

impl PreparedTempoHeader<'_> {
    /// Installs the committed execution root and signs the complete native header.
    pub(super) fn finalize(self, state_root: B256) {
        self.header.inner.state_root = state_root;
        self.consensus.finalize(self.header, self.hardfork).expect(
            "prepared height, metadata, hardfork and committee remain exclusively borrowed",
        );
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

    /// Schedules a development identity change without racing canonical mining or rewinds.
    pub async fn tempo_rotate_consensus_identity(
        &self,
    ) -> Result<TempoConsensusIdentity, BlockchainError> {
        let _mining = self.mining.lock().await;
        let _db = self.db.read().await;
        let mut storage = self.blockchain.storage.write();
        storage
            .tempo
            .as_mut()
            .ok_or_else(tempo_unavailable)?
            .consensus
            .rotate()
            .map_err(|error| BlockchainError::Internal(error.to_string()))
    }

    /// Returns a certificate and its complete canonical block from the same chain snapshot.
    pub async fn tempo_finalization(
        &self,
        height: Option<u64>,
    ) -> Result<(TempoFinalization, Block), BlockchainError> {
        let _db = self.db.read().await;
        let storage = self.blockchain.storage.read();
        let tempo = tempo_chain(&storage)?;
        match height {
            Some(height) => {
                let finalization = tempo
                    .consensus
                    .finalization(height)
                    .ok_or_else(|| consensus_unavailable("certificate"))?;
                let block = storage
                    .hashes
                    .get(&height)
                    .and_then(|hash| storage.blocks.get(hash))
                    .ok_or_else(|| consensus_unavailable("block"))?;
                certified_block(finalization, block)
            }
            None => {
                latest_finalization(tempo)?.ok_or_else(|| consensus_unavailable("certifications"))
            }
        }
    }

    /// Returns no finalized block until this local signer has produced its first certificate.
    pub async fn tempo_latest_finalization(
        &self,
    ) -> Result<Option<(TempoFinalization, Block)>, BlockchainError> {
        let _db = self.db.read().await;
        let storage = self.blockchain.storage.read();
        latest_finalization(tempo_chain(&storage)?)
    }

    /// Resolves only the pinned, unsigned Tempo fork anchor for local chain rollback.
    pub(crate) async fn tempo_rollback_block(
        &self,
        number: u64,
    ) -> Result<Option<Block>, BlockchainError> {
        if self.blockchain.storage.read().tempo.is_none() {
            return Ok(None);
        }
        let Some(fork) = self.get_fork().filter(|fork| fork.block_number() == number) else {
            return Ok(None);
        };
        let hash = fork.block_hash();
        let Some(block) = fork.block_by_hash(hash).await? else { return Ok(None) };
        // AnyRpcBlock retains native header fields at the block level. Deserialize the complete
        // RPC object through Tempo's codec rather than rebuilding its Ethereum-only inner header.
        let header = serde_json::to_value(&block)
            .and_then(serde_json::from_value::<TempoHeader>)
            .map_err(|error| BlockchainError::Internal(error.to_string()))?;
        if block.header.hash != hash || header.number() != number || header.hash_slow() != hash {
            return Err(BlockchainError::Internal(
                "Tempo fork anchor header does not match its pinned hash".into(),
            ));
        }
        Ok(Some(Block { header: header.into(), body: Default::default() }))
    }
}

fn tempo_chain<N: Network>(storage: &BlockchainStorage<N>) -> Result<&TempoChain, BlockchainError> {
    storage.tempo.as_ref().ok_or_else(tempo_unavailable)
}

fn tempo_unavailable() -> BlockchainError {
    RpcError::invalid_params("Tempo consensus is only available on the Tempo network").into()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeConfig, eth::backend::mem::MiningCommitHook, spawn};
    use alloy_primitives::Address;
    use alloy_rpc_types::TransactionRequest;
    use alloy_serde::WithOtherFields;
    use foundry_evm::backend::RevertStateSnapshotAction;
    use std::{sync::Arc, time::Duration};
    use tokio::{sync::Notify, time::timeout};

    #[tokio::test(flavor = "multi_thread")]
    async fn consensus_reads_wait_for_complete_block_publication() {
        let (api, _handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        let hook =
            MiningCommitHook { reached: Arc::new(Notify::new()), resume: Arc::new(Notify::new()) };
        *api.backend.mining_commit_hook.lock() = Some(hook.clone());
        let mining_api = api.clone();
        let mining = tokio::spawn(async move { mining_api.mine_one().await });
        timeout(Duration::from_secs(10), hook.reached.notified())
            .await
            .expect("mining reaches the publication barrier");

        assert_eq!(api.backend.best_number(), 0);
        let mut certificate = Box::pin(api.backend.tempo_finalization(None));
        let mut identity = Box::pin(api.backend.tempo_consensus_identity());
        let mut rotation = Box::pin(api.backend.tempo_rotate_consensus_identity());
        assert!(futures::poll!(certificate.as_mut()).is_pending());
        assert!(futures::poll!(identity.as_mut()).is_pending());
        assert!(futures::poll!(rotation.as_mut()).is_pending());

        hook.resume.notify_one();
        timeout(Duration::from_secs(10), async {
            mining.await.unwrap().unwrap();
            let (certificate, block) = certificate.await.unwrap();
            assert_eq!(block.header.number(), 1);
            assert_eq!(block.header.hash_slow(), certificate.digest);
            assert_eq!(api.backend.best_hash(), certificate.digest);
            identity.await.unwrap();
            rotation.await.unwrap();
        })
        .await
        .expect("consensus readers complete after publication");
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
        api.backend.blockchain.storage.write().tempo.as_mut().unwrap().reanchor(5);

        let error = api.mine_one().await.unwrap_err();
        assert!(matches!(
            error,
            BlockchainError::Internal(message) if message == "expected Tempo consensus height 6, got 1"
        ));
        assert_eq!(api.backend.best_number(), 0);
        assert_eq!(api.backend.best_hash(), previous_hash);
        assert_eq!(api.backend.db.read().await.maybe_state_root(), previous_root);
        assert!(api.backend.tempo_latest_finalization().await.unwrap().is_none());

        api.backend.blockchain.storage.write().tempo.as_mut().unwrap().reanchor(0);
        api.mine_one().await.unwrap();
        let (_, block) = api.backend.tempo_finalization(None).await.unwrap();
        assert_eq!(block.body.transactions.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_rollback_rejects_mismatched_native_anchor() {
        let config = NodeConfig::test_tempo().with_no_mining(true);
        let (source_api, source) = spawn(config.clone()).await;
        source_api.mine_one().await.unwrap();
        let (api, _handle) = spawn(
            config
                .with_eth_rpc_url(Some(source.http_endpoint()))
                .with_fork_block_number(Some(1u64)),
        )
        .await;
        let anchor = api.backend.tempo_rollback_block(1).await.unwrap().unwrap();
        assert_eq!(anchor.header.hash_slow(), source_api.backend.best_hash());
        assert!(api.backend.tempo_rollback_block(0).await.unwrap().is_none());

        let fork = api.backend.get_fork().unwrap();
        fork.storage
            .write()
            .blocks
            .get_mut(&fork.block_hash())
            .unwrap()
            .other
            .insert("timestampMillisPart".into(), serde_json::Value::String("0x1".into()));
        let error = api.backend.tempo_rollback_block(1).await.unwrap_err();
        assert!(matches!(
            error,
            BlockchainError::Internal(message)
                if message == "Tempo fork anchor header does not match its pinned hash"
        ));
        assert_eq!(api.backend.best_number(), 1);
        assert_eq!(api.backend.best_hash(), anchor.header.hash_slow());
        assert!(api.backend.tempo_latest_finalization().await.unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loaded_state_invalidates_tempo_database_snapshots() {
        let (api, _handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        let snapshot = api.evm_snapshot().await.unwrap();
        api.mine_one().await.unwrap();
        let state = api.backend.serialized_state(false).await.unwrap();

        api.backend.load_state(state).await.unwrap();

        assert!(api.backend.list_state_snapshots().is_empty());
        assert!(!api.evm_revert(snapshot).await.unwrap());
        assert!(api.backend.tempo_latest_finalization().await.unwrap().is_none());
        assert!(
            !api.backend
                .db
                .write()
                .await
                .revert_state(snapshot, RevertStateSnapshotAction::RevertRemove,)
        );
    }
}
