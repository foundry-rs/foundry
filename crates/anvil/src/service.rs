//! background service

use crate::{
    NodeResult,
    eth::{
        backend::validate::TransactionValidator, error::BlockchainError, fees::FeeHistoryService,
        miner::Miner, pool::Pool,
    },
    filter::Filters,
    mem::{Backend, storage::MinedBlockOutcome},
};
use alloy_consensus::TxReceipt;
use alloy_network::Network;
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxEnvelope};
use futures::{FutureExt, Stream, StreamExt};
use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{task::JoinHandle, time::Interval};

/// The type that drives the blockchain's state
///
/// This service is basically an endless future that continuously polls the miner which returns
/// transactions for the next block, then those transactions are handed off to the backend to
/// construct a new block, if all transactions were successfully included in a new block they get
/// purged from the `Pool`.
pub struct NodeService<N: Network>
where
    N::ReceiptEnvelope: TxReceipt<Log = alloy_primitives::Log>,
{
    /// The pool that holds all transactions.
    pool: Arc<Pool<N::TxEnvelope>>,
    /// Creates new blocks.
    block_producer: BlockProducer<N>,
    /// The miner responsible to select transactions from the `pool`.
    miner: Miner<N::TxEnvelope>,
    /// Maintenance task for fee history related tasks.
    fee_history: FeeHistoryService<N>,
    /// Tracks all active filters
    filters: Filters<N>,
    /// The interval at which to check for filters that need to be evicted
    filter_eviction_interval: Interval,
}

impl<N: Network> NodeService<N>
where
    Backend<N>: TransactionValidator<N::TxEnvelope>,
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
{
    pub fn new(
        pool: Arc<Pool<N::TxEnvelope>>,
        backend: Arc<Backend<N>>,
        miner: Miner<N::TxEnvelope>,
        fee_history: FeeHistoryService<N>,
        filters: Filters<N>,
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

impl<N: Network> Future for NodeService<N>
where
    Backend<N>: TransactionValidator<N::TxEnvelope>,
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
{
    type Output = NodeResult<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        // this drives block production and feeds new sets of ready transactions to the block
        // producer
        loop {
            // advance block production until pending
            while let Poll::Ready(Some(result)) = pin.block_producer.poll_next_unpin(cx) {
                match result {
                    BlockProduction::Mined(outcome) => {
                        trace!(target: "node", "mined block {}", outcome.block_number);
                        pin.pool.on_mined_block(outcome);
                    }
                    BlockProduction::Failed(generation) => {
                        pin.miner.handle_failed_candidate(generation);
                        break;
                    }
                }
            }

            // Do not select snapshots while another candidate is in flight. This leaves newer
            // ready notifications in the miner so a failed candidate cannot discard their work.
            if pin.block_producer.is_idle()
                && let Poll::Ready(work) = pin.miner.poll(&pin.pool, cx)
            {
                // miner returned a set of transaction that we feed to the producer
                pin.block_producer.queued.push_back(work);
            } else {
                // no progress made
                break;
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

type MiningResult<N> =
    (Result<MinedBlockOutcome<<N as Network>::TxEnvelope>, BlockchainError>, Arc<Backend<N>>, u64);

enum BlockProduction<T> {
    Mined(MinedBlockOutcome<T>),
    Failed(u64),
}

/// A type that exclusively mines one block at a time
#[must_use = "streams do nothing unless polled"]
struct BlockProducer<N: Network> {
    /// Holds the backend if no block is being mined
    idle_backend: Option<Arc<Backend<N>>>,
    /// Single active future that mines a new block
    block_mining: Option<JoinHandle<MiningResult<N>>>,
    /// backlog of sets of transactions ready to be mined
    queued: VecDeque<crate::eth::miner::MiningWork<N::TxEnvelope>>,
}

impl<N: Network> BlockProducer<N>
where
    Backend<N>: TransactionValidator<N::TxEnvelope>,
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
{
    fn new(backend: Arc<Backend<N>>) -> Self {
        Self { idle_backend: Some(backend), block_mining: None, queued: Default::default() }
    }

    fn is_idle(&self) -> bool {
        self.idle_backend.is_some() && self.block_mining.is_none() && self.queued.is_empty()
    }
}

impl<N: Network> Stream for BlockProducer<N>
where
    Backend<N>: TransactionValidator<N::TxEnvelope> + Send + Sync + 'static,
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope> + 'static,
{
    type Item = BlockProduction<N::TxEnvelope>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();

        if !pin.queued.is_empty() {
            // only spawn a building task if there's none in progress already
            if let Some(backend) = pin.idle_backend.take() {
                let work = pin.queued.pop_front().expect("not empty; qed");
                let generation = work.generation;

                // we spawn this on as blocking task because this can be blocking for a while in
                // forking mode, because of all the rpc calls to fetch the required state
                let handle = tokio::runtime::Handle::current();
                let mining = tokio::task::spawn_blocking(move || {
                    handle.block_on(async move {
                        trace!(target: "miner", "creating new block");
                        let block = backend.mine_block(work.transactions).await;
                        if let Ok(block) = &block {
                            trace!(target: "miner", "created new block: {}", block.block_number);
                        }
                        (block, backend, generation)
                    })
                });
                pin.block_mining = Some(mining);
            }
        }

        if let Some(mut mining) = pin.block_mining.take() {
            if let Poll::Ready(res) = mining.poll_unpin(cx) {
                return match res {
                    Ok((Ok(outcome), backend, _)) => {
                        pin.idle_backend = Some(backend);
                        Poll::Ready(Some(BlockProduction::Mined(outcome)))
                    }
                    Ok((Err(error), backend, generation)) => {
                        pin.idle_backend = Some(backend);
                        pin.queued.clear();
                        warn!(target: "miner", %error, "failed to finalize block");
                        Poll::Ready(Some(BlockProduction::Failed(generation)))
                    }
                    Err(err) => {
                        panic!("miner task failed: {err}");
                    }
                };
            }
            pin.block_mining = Some(mining)
        }

        Poll::Pending
    }
}
