//! background service

use crate::{
    NodeResult,
    eth::{
        backend::validate::TransactionValidator, error::BlockchainError, fees::FeeHistoryService,
        miner::Miner, pool::Pool,
    },
    filter::Filters,
    mem::Backend,
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
            block_producer: BlockProducer::new(backend, pool.clone(), miner.clone()),
            pool,
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
                    BlockProduction::Mined(block_number) => {
                        trace!(target: "node", "mined block {block_number}");
                    }
                    BlockProduction::Failed(generation) => {
                        pin.miner.handle_failed_candidate(generation);
                        break;
                    }
                    BlockProduction::Skipped => {}
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

type MiningResult<N> = (Result<Option<u64>, BlockchainError>, Arc<Backend<N>>, u64);

enum BlockProduction {
    Mined(u64),
    Failed(u64),
    Skipped,
}

/// Whether the [`BlockProducer`] owns the backend or lent it to a block mining task.
enum ProducerState<N: Network> {
    /// No block is being mined; the producer holds the backend.
    Idle(Arc<Backend<N>>),
    /// A single task is mining a block and hands the backend back when it completes.
    Mining(JoinHandle<MiningResult<N>>),
}

/// A type that exclusively mines one block at a time
#[must_use = "streams do nothing unless polled"]
struct BlockProducer<N: Network> {
    /// Owns the backend while idle, or the task mining the current block.
    state: ProducerState<N>,
    /// Pool used to select transactions while holding the mining lock.
    pool: Arc<Pool<N::TxEnvelope>>,
    /// Miner to wake when a mined block leaves ready transactions behind.
    miner: Miner<N::TxEnvelope>,
    /// backlog of sets of transactions ready to be mined
    queued: VecDeque<crate::eth::miner::MiningWork<N::TxEnvelope>>,
}

impl<N: Network> BlockProducer<N>
where
    Backend<N>: TransactionValidator<N::TxEnvelope>,
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
{
    fn new(
        backend: Arc<Backend<N>>,
        pool: Arc<Pool<N::TxEnvelope>>,
        miner: Miner<N::TxEnvelope>,
    ) -> Self {
        Self { state: ProducerState::Idle(backend), pool, miner, queued: Default::default() }
    }

    fn is_idle(&self) -> bool {
        matches!(self.state, ProducerState::Idle(_)) && self.queued.is_empty()
    }
}

impl<N: Network> Stream for BlockProducer<N>
where
    Backend<N>: TransactionValidator<N::TxEnvelope> + Send + Sync + 'static,
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope> + 'static,
{
    type Item = BlockProduction;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();

        // only spawn a building task if there's none in progress already
        if let ProducerState::Idle(backend) = &pin.state
            && let Some(work) = pin.queued.pop_front()
        {
            let backend = backend.clone();
            let generation = work.generation;
            let pause_epoch = work.pause_epoch;
            let selected = work.transactions.len();
            let pool = pin.pool.clone();
            let miner = pin.miner.clone();

            // we spawn this on as blocking task because this can be blocking for a while in
            // forking mode, because of all the rpc calls to fetch the required state
            let handle = tokio::runtime::Handle::current();
            let mining = tokio::task::spawn_blocking(move || {
                handle.block_on(async move {
                    let mining_guard = backend.lock_mining_owned().await;
                    // Mining was paused after this candidate was selected, so skip it even if
                    // it has resumed since. Ready transactions are mined by auto mining on
                    // resume, or by the next tick.
                    if miner.is_stale(pause_epoch) {
                        return (Ok(None), backend, generation);
                    }
                    let transactions = pool.ready_transactions().take(selected).collect::<Vec<_>>();
                    if selected > 0 && transactions.is_empty() {
                        return (Ok(None), backend, generation);
                    }
                    trace!(target: "miner", "creating new block");
                    let result =
                        backend.mine_block_locked(transactions, false).await.map(|outcome| {
                            let block_number = outcome.block_number;
                            if pool.on_mined_block(outcome) {
                                miner.retry_ready_transactions();
                            }
                            trace!(target: "miner", "created new block: {block_number}");
                            Some(block_number)
                        });
                    drop(mining_guard);
                    (result, backend, generation)
                })
            });
            pin.state = ProducerState::Mining(mining);
        }

        if let ProducerState::Mining(mining) = &mut pin.state
            && let Poll::Ready(res) = mining.poll_unpin(cx)
        {
            let (result, backend, generation) = match res {
                Ok(output) => output,
                Err(err) => panic!("miner task failed: {err}"),
            };
            pin.state = ProducerState::Idle(backend);
            return Poll::Ready(Some(match result {
                Ok(Some(block_number)) => BlockProduction::Mined(block_number),
                Ok(None) => BlockProduction::Skipped,
                Err(error) => {
                    pin.queued.clear();
                    warn!(target: "miner", %error, "failed to finalize block");
                    BlockProduction::Failed(generation)
                }
            }));
        }

        Poll::Pending
    }
}
