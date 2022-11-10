//! background service

use crate::{
    eth::{
        fees::FeeHistoryService,
        miner::Miner,
        pool::{transactions::PoolTransaction, Pool},
    },
    filter::Filters,
    mem::{storage::MinedBlockOutcome, Backend},
    NodeResult,
};
use futures::{FutureExt, Stream, StreamExt};
use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::time::Interval;
use tracing::trace;

/// The type that drives the blockchain's state
///
/// This service is basically an endless future that continuously polls the miner which returns
/// transactions for the next block, then those transactions are handed off to the
/// [backend](backend::mem::Backend) to construct a new block, if all transactions were successfully
/// included in a new block they get purged from the `Pool`.
pub struct NodeService {
    /// the pool that holds all transactions
    pool: Arc<Pool>,
    /// creates new blocks
    block_producer: BlockProducer,
    /// the miner responsible to select transactions from the `pool´
    miner: Miner,
    /// maintenance task for fee history related tasks
    fee_history: FeeHistoryService,
    /// Tracks all active filters
    filters: Filters,
    /// The interval at which to check for filters that need to be evicted
    filter_eviction_interval: Interval,
}

impl NodeService {
    pub fn new(
        pool: Arc<Pool>,
        backend: Arc<Backend>,
        miner: Miner,
        fee_history: FeeHistoryService,
        filters: Filters,
    ) -> Self {
        let start = tokio::time::Instant::now() + filters.keep_alive();
        let filter_eviction_interval = tokio::time::interval_at(start, filters.keep_alive());
        Self {
            pool,
            block_producer: BlockProducer::new(backend),
            miner,
            fee_history,
            filter_eviction_interval,
            filters,
        }
    }
}

impl Future for NodeService {
    type Output = NodeResult<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        // this drives block production and feeds new sets of ready transactions to the block
        // producer
        loop {
            while let Poll::Ready(Some(outcome)) = pin.block_producer.poll_next_unpin(cx) {
                trace!(target: "node", "mined block {}", outcome.block_number);
                // prune the transactions from the pool
                pin.pool.on_mined_block(outcome);
            }

            if let Poll::Ready(transactions) = pin.miner.poll(&pin.pool, cx) {
                // miner returned a set of transaction that we feed to the producer
                pin.block_producer.queued.push_back(transactions);
            } else {
                // no progress made
                break
            }
        }

        // poll the fee history task
        let _ = pin.fee_history.poll_unpin(cx);

        if pin.filter_eviction_interval.poll_tick(cx).is_ready() {
            let filters = pin.filters.clone();
            // evict filters that timed out
            tokio::task::spawn(async move { filters.evict().await });
        }

        Poll::Pending
    }
}

// The type of the future that mines a new block
type BlockMiningFuture =
    Pin<Box<dyn Future<Output = (MinedBlockOutcome, Arc<Backend>)> + Send + Sync>>;

/// A type that exclusively mines one block at a time
#[must_use = "BlockProducer does nothing unless polled"]
struct BlockProducer {
    /// Holds the backend if no block is being mined
    idle_backend: Option<Arc<Backend>>,
    /// Single active future that mines a new block
    block_mining: Option<BlockMiningFuture>,
    /// backlog of sets of transactions ready to be mined
    queued: VecDeque<Vec<Arc<PoolTransaction>>>,
}

// === impl BlockProducer ===

impl BlockProducer {
    fn new(backend: Arc<Backend>) -> Self {
        Self { idle_backend: Some(backend), block_mining: None, queued: Default::default() }
    }
}

impl Stream for BlockProducer {
    type Item = MinedBlockOutcome;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();

        if !pin.queued.is_empty() {
            if let Some(backend) = pin.idle_backend.take() {
                let transactions = pin.queued.pop_front().expect("not empty; qed");
                pin.block_mining = Some(Box::pin(async move {
                    trace!(target: "miner", "creating new block");
                    let block = backend.mine_block(transactions).await;
                    trace!(target: "miner", "created new block: {}", block.block_number);
                    (block, backend)
                }));
            }
        }

        if let Some(mut mining) = pin.block_mining.take() {
            if let Poll::Ready((outcome, backend)) = mining.poll_unpin(cx) {
                pin.idle_backend = Some(backend);
                return Poll::Ready(Some(outcome))
            } else {
                pin.block_mining = Some(mining)
            }
        }

        Poll::Pending
    }
}
