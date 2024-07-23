//! Mines transactions

use crate::eth::pool::{transactions::PoolTransaction, Pool};
use alloy_primitives::TxHash;
use futures::{
    channel::mpsc::Receiver,
    stream::{Fuse, Stream, StreamExt},
    task::AtomicWaker,
};
use parking_lot::{lock_api::RwLockWriteGuard, RawRwLock, RwLock};
use std::{
    fmt,
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
    time::Duration,
};
use tokio::time::{Interval, MissedTickBehavior};

#[derive(Clone, Debug)]
pub struct Miner {
    /// The mode this miner currently operates in
    mode: Arc<RwLock<MiningMode>>,
    /// used for task wake up when the mining mode was forcefully changed
    ///
    /// This will register the task so we can manually wake it up if the mining mode was changed
    inner: Arc<MinerInner>,
    /// Transactions included into the pool before any others are.
    /// Done once on startup.
    force_transactions: Option<Vec<Arc<PoolTransaction>>>,
}

impl Miner {
    /// Returns a new miner with that operates in the given `mode`.
    pub fn new(mode: MiningMode) -> Self {
        Self {
            mode: Arc::new(RwLock::new(mode)),
            inner: Default::default(),
            force_transactions: None,
        }
    }

    /// Provide transactions that will cause a block to be mined with transactions
    /// as soon as the miner is polled.
    /// Providing an empty list of transactions will cause the miner to mine an empty block assuming
    /// there are not other transactions in the pool.
    pub fn with_forced_transactions(
        mut self,
        force_transactions: Option<Vec<PoolTransaction>>,
    ) -> Self {
        self.force_transactions =
            force_transactions.map(|tx| tx.into_iter().map(Arc::new).collect());
        self
    }

    /// Returns the write lock of the mining mode
    pub fn mode_write(&self) -> RwLockWriteGuard<'_, RawRwLock, MiningMode> {
        self.mode.write()
    }

    /// Returns `true` if auto mining is enabled
    pub fn is_auto_mine(&self) -> bool {
        let mode = self.mode.read();
        matches!(*mode, MiningMode::Auto(_))
    }

    pub fn is_interval(&self) -> bool {
        let mode = self.mode.read();
        matches!(*mode, MiningMode::FixedBlockTime(_))
    }

    /// Sets the mining mode to operate in
    pub fn set_mining_mode(&self, mode: MiningMode) {
        let new_mode = format!("{mode:?}");
        let mode = std::mem::replace(&mut *self.mode_write(), mode);
        trace!(target: "miner", "updated mining mode from {:?} to {}", mode, new_mode);
        self.inner.wake();
    }

    /// polls the [Pool] and returns those transactions that should be put in a block according to
    /// the current mode.
    ///
    /// May return an empty list, if no transactions are ready.
    pub fn poll(
        &mut self,
        pool: &Arc<Pool>,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<PoolTransaction>>> {
        self.inner.register(cx);
        let next = ready!(self.mode.write().poll(pool, cx));
        if let Some(mut transactions) = self.force_transactions.take() {
            transactions.extend(next);
            Poll::Ready(transactions)
        } else {
            Poll::Ready(next)
        }
    }
}

/// A Mining mode that does nothing
#[derive(Debug)]
pub struct MinerInner {
    waker: AtomicWaker,
}

impl MinerInner {
    /// Call the waker again
    fn wake(&self) {
        self.waker.wake();
    }

    fn register(&self, cx: &Context<'_>) {
        self.waker.register(cx.waker());
    }
}

impl Default for MinerInner {
    fn default() -> Self {
        Self { waker: AtomicWaker::new() }
    }
}

/// Mode of operations for the `Miner`
#[derive(Debug)]
pub enum MiningMode {
    /// A miner that does nothing
    None,
    /// A miner that listens for new transactions that are ready.
    ///
    /// Either one transaction will be mined per block, or any number of transactions will be
    /// allowed
    Auto(ReadyTransactionMiner),
    /// A miner that constructs a new block every `interval` tick
    FixedBlockTime(FixedBlockTimeMiner),

    /// A minner that uses both Auto and FixedBlockTime
    Mixed(ReadyTransactionMiner, FixedBlockTimeMiner),
}

impl MiningMode {
    pub fn instant(max_transactions: usize, listener: Receiver<TxHash>) -> Self {
        Self::Auto(ReadyTransactionMiner {
            max_transactions,
            has_pending_txs: None,
            rx: listener.fuse(),
        })
    }

    pub fn interval(duration: Duration) -> Self {
        Self::FixedBlockTime(FixedBlockTimeMiner::new(duration))
    }

    pub fn mixed(max_transactions: usize, listener: Receiver<TxHash>, duration: Duration) -> Self {
        Self::Mixed(
            ReadyTransactionMiner { max_transactions, has_pending_txs: None, rx: listener.fuse() },
            FixedBlockTimeMiner::new(duration),
        )
    }

    /// polls the [Pool] and returns those transactions that should be put in a block, if any.
    pub fn poll(
        &mut self,
        pool: &Arc<Pool>,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<PoolTransaction>>> {
        match self {
            Self::None => Poll::Pending,
            Self::Auto(miner) => miner.poll(pool, cx),
            Self::FixedBlockTime(miner) => miner.poll(pool, cx),
            Self::Mixed(auto, fixed) => {
                let auto_txs = auto.poll(pool, cx);
                let fixed_txs = fixed.poll(pool, cx);

                match (auto_txs, fixed_txs) {
                    // Both auto and fixed transactions are ready, combine them
                    (Poll::Ready(mut auto_txs), Poll::Ready(fixed_txs)) => {
                        for tx in fixed_txs {
                            // filter unique transactions
                            if auto_txs.iter().any(|auto_tx| auto_tx.hash() == tx.hash()) {
                                continue;
                            }
                            auto_txs.push(tx);
                        }
                        Poll::Ready(auto_txs)
                    }
                    // Only auto transactions are ready, return them
                    (Poll::Ready(auto_txs), Poll::Pending) => Poll::Ready(auto_txs),
                    // Only fixed transactions are ready or both are pending,
                    // return fixed transactions or pending status
                    (Poll::Pending, fixed_txs) => fixed_txs,
                }
            }
        }
    }
}

/// A miner that's supposed to create a new block every `interval`, mining all transactions that are
/// ready at that time.
///
/// The default blocktime is set to 6 seconds
#[derive(Debug)]
pub struct FixedBlockTimeMiner {
    /// The interval this fixed block time miner operates with
    interval: Interval,
}

impl FixedBlockTimeMiner {
    /// Creates a new instance with an interval of `duration`
    pub fn new(duration: Duration) -> Self {
        let start = tokio::time::Instant::now() + duration;
        let mut interval = tokio::time::interval_at(start, duration);
        // we use delay here, to ensure ticks are not shortened and to tick at multiples of interval
        // from when tick was called rather than from start
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self { interval }
    }

    fn poll(&mut self, pool: &Arc<Pool>, cx: &mut Context<'_>) -> Poll<Vec<Arc<PoolTransaction>>> {
        if self.interval.poll_tick(cx).is_ready() {
            // drain the pool
            return Poll::Ready(pool.ready_transactions().collect())
        }
        Poll::Pending
    }
}

impl Default for FixedBlockTimeMiner {
    fn default() -> Self {
        Self::new(Duration::from_secs(6))
    }
}

/// A miner that Listens for new ready transactions
pub struct ReadyTransactionMiner {
    /// how many transactions to mine per block
    max_transactions: usize,
    /// stores whether there are pending transactions (if known)
    has_pending_txs: Option<bool>,
    /// Receives hashes of transactions that are ready
    rx: Fuse<Receiver<TxHash>>,
}

impl ReadyTransactionMiner {
    fn poll(&mut self, pool: &Arc<Pool>, cx: &mut Context<'_>) -> Poll<Vec<Arc<PoolTransaction>>> {
        // drain the notification stream
        while let Poll::Ready(Some(_hash)) = Pin::new(&mut self.rx).poll_next(cx) {
            self.has_pending_txs = Some(true);
        }

        if self.has_pending_txs == Some(false) {
            return Poll::Pending
        }

        let transactions =
            pool.ready_transactions().take(self.max_transactions).collect::<Vec<_>>();

        // there are pending transactions if we didn't drain the pool
        self.has_pending_txs = Some(transactions.len() >= self.max_transactions);

        if transactions.is_empty() {
            return Poll::Pending
        }

        Poll::Ready(transactions)
    }
}

impl fmt::Debug for ReadyTransactionMiner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadyTransactionMiner")
            .field("max_transactions", &self.max_transactions)
            .finish_non_exhaustive()
    }
}
