//! Mines transactions

use crate::eth::pool::{transactions::PoolTransaction, Pool};
use ethers::prelude::TxHash;
use futures::{
    channel::mpsc::Receiver,
    stream::{Fuse, Stream, StreamExt},
    task::AtomicWaker,
};
use parking_lot::{lock_api::RwLockWriteGuard, RawRwLock, RwLock};
use std::{
    collections::HashSet,
    fmt,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::Interval;
use tracing::trace;

#[derive(Debug, Clone)]
pub struct Miner {
    /// The mode this miner currently operates in
    mode: Arc<RwLock<MiningMode>>,
    /// used for task wake up when the mining mode was forcefully changed
    ///
    /// This will register the task so we can manually wake it up if the mining mode was changed
    inner: Arc<MinerInner>,
}

// === impl Miner ===

impl Miner {
    /// Returns a new miner with that operates in the given `mode`
    pub fn new(mode: MiningMode) -> Self {
        Self { mode: Arc::new(RwLock::new(mode)), inner: Default::default() }
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
        let mode = std::mem::replace(&mut *self.mode_write(), mode);
        trace!(target: "miner", "updated mining mode {:?}", mode);
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
        self.mode.write().poll(pool, cx)
    }
}

/// A Mining mode that does nothing
#[derive(Debug)]
pub struct MinerInner {
    waker: AtomicWaker,
    set: AtomicBool,
}

// === impl MinerInner ===

impl MinerInner {
    /// Call the waker again
    fn wake(&self) {
        self.set.store(true, Relaxed);
        self.waker.wake();
    }

    fn register(&self, cx: &mut Context<'_>) {
        // avoid waker reregistration.
        if self.set.load(Relaxed) {
            return
        }

        self.waker.register(cx.waker());
        self.set.load(Relaxed);
    }
}

impl Default for MinerInner {
    fn default() -> Self {
        Self { waker: AtomicWaker::new(), set: AtomicBool::new(false) }
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
}

// === impl MiningMode ===

impl MiningMode {
    pub fn instant(max_transactions: usize, listener: Receiver<TxHash>) -> Self {
        MiningMode::Auto(ReadyTransactionMiner {
            max_transactions,
            ready: Default::default(),
            rx: listener.fuse(),
        })
    }

    pub fn interval(duration: Duration) -> Self {
        MiningMode::FixedBlockTime(FixedBlockTimeMiner::new(duration))
    }

    /// polls the [Pool] and returns those transactions that should be put in a block, if any.
    pub fn poll(
        &mut self,
        pool: &Arc<Pool>,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<PoolTransaction>>> {
        match self {
            MiningMode::None => Poll::Pending,
            MiningMode::Auto(miner) => miner.poll(pool, cx),
            MiningMode::FixedBlockTime(miner) => miner.poll(pool, cx),
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

// === impl FixedBlockTimeMiner ===

impl FixedBlockTimeMiner {
    /// Creates a new instance with an interval of `duration`
    pub fn new(duration: Duration) -> Self {
        Self { interval: tokio::time::interval(duration) }
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
    /// transactions received
    ready: HashSet<TxHash>,
    /// receives hashes of transactions that are ready
    rx: Fuse<Receiver<TxHash>>,
}

// === impl ReadyTransactionMiner ===

impl ReadyTransactionMiner {
    fn poll(&mut self, pool: &Arc<Pool>, cx: &mut Context<'_>) -> Poll<Vec<Arc<PoolTransaction>>> {
        while let Poll::Ready(Some(hash)) = Pin::new(&mut self.rx).poll_next(cx) {
            self.ready.insert(hash);
        }

        if self.ready.is_empty() {
            return Poll::Pending
        }

        let transactions =
            pool.ready_transactions().take(self.max_transactions).collect::<Vec<_>>();

        for tx in transactions.iter() {
            self.ready.remove(tx.hash());
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
