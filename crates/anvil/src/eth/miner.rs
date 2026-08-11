//! Mines transactions

use crate::eth::pool::{MiningBatch, Pool, transactions::PoolTransaction};
use alloy_primitives::TxHash;
use futures::{
    channel::mpsc::Receiver,
    stream::{Fuse, StreamExt},
    task::AtomicWaker,
};
use parking_lot::{RawRwLock, RwLock, lock_api::RwLockWriteGuard};
use std::{
    fmt,
    marker::PhantomData,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::time::{Interval, MissedTickBehavior, Sleep};

/// Window for grouping concurrently-submitted transactions into one instant-mined block.
/// Scoped to batch/in-process concurrency; not a guarantee for independent external clients.
const INSTANT_COALESCE_WINDOW: Duration = Duration::from_millis(5);

pub struct Miner<T> {
    /// The mode this miner currently operates in
    mode: Arc<RwLock<MiningMode>>,
    /// Identifies the current mode so stale candidate failures cannot modify its replacement.
    generation: Arc<AtomicU64>,
    /// used for task wake up when the mining mode was forcefully changed
    ///
    /// This will register the task so we can manually wake it up if the mining mode was changed
    inner: Arc<MinerInner>,
    /// Transactions included into the pool before any others are.
    /// Done once on startup.
    force_transactions: Option<(u64, Vec<Arc<PoolTransaction<T>>>)>,
    /// Transaction type handled by the associated pool.
    transaction: PhantomData<fn() -> T>,
}

impl<T> Clone for Miner<T> {
    fn clone(&self) -> Self {
        Self {
            mode: self.mode.clone(),
            generation: self.generation.clone(),
            inner: self.inner.clone(),
            force_transactions: self.force_transactions.clone(),
            transaction: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Miner<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Miner")
            .field("mode", &self.mode)
            .field(
                "force_transactions",
                &self.force_transactions.as_ref().map(|(_, txs)| txs.len()),
            )
            .finish_non_exhaustive()
    }
}

impl<T> Miner<T> {
    /// Returns a new miner with that operates in the given `mode`.
    pub fn new(mode: MiningMode) -> Self {
        Self {
            mode: Arc::new(RwLock::new(mode)),
            generation: Default::default(),
            inner: Default::default(),
            force_transactions: None,
            transaction: PhantomData,
        }
    }

    /// Provide transactions that will cause a block to be mined with transactions
    /// as soon as the miner is polled.
    /// Providing an empty list of transactions will cause the miner to mine an empty block assuming
    /// there are not other transactions in the pool.
    pub fn with_forced_transactions(
        self,
        force_transactions: Option<Vec<PoolTransaction<T>>>,
    ) -> Self {
        self.with_forced_transactions_at_generation(force_transactions, 0)
    }

    pub(crate) fn with_forced_transactions_at_generation(
        mut self,
        force_transactions: Option<Vec<PoolTransaction<T>>>,
        generation: u64,
    ) -> Self {
        self.force_transactions =
            force_transactions.map(|tx| (generation, tx.into_iter().map(Arc::new).collect()));
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

    pub fn get_interval(&self) -> Option<u64> {
        let mode = self.mode.read();
        if let MiningMode::FixedBlockTime(ref mm) = *mode {
            return Some(mm.interval.period().as_secs());
        }
        None
    }

    /// Returns the configured block interval for fixed or mixed mining.
    pub(crate) fn block_interval(&self) -> Option<Duration> {
        let mode = self.mode.read();
        match &*mode {
            MiningMode::FixedBlockTime(miner) | MiningMode::Mixed(_, miner) => {
                Some(miner.interval.period())
            }
            MiningMode::None | MiningMode::Auto(_) => None,
        }
    }

    /// Sets the mining mode to operate in
    pub fn set_mining_mode(&self, mode: MiningMode) {
        let new_mode = format!("{mode:?}");
        let mut current = self.mode_write();
        let mode = std::mem::replace(&mut *current, mode);
        self.generation.fetch_add(1, Ordering::Relaxed);
        drop(current);
        trace!(target: "miner", "updated mining mode from {:?} to {}", mode, new_mode);
        self.inner.wake();
    }

    /// Resets the mode that launched a failed candidate.
    pub(crate) fn handle_failed_candidate(&self, generation: u64) {
        let mut mode = self.mode.write();
        if self.generation.load(Ordering::Relaxed) != generation {
            if let MiningMode::Auto(miner) | MiningMode::Mixed(miner, _) = &mut *mode {
                miner.has_pending_txs = Some(true);
                miner.coalesce = None;
            }
            return;
        }
        match &mut *mode {
            MiningMode::Auto(miner) | MiningMode::Mixed(miner, _) => {
                miner.has_pending_txs = Some(false);
                miner.coalesce = None;
            }
            MiningMode::None | MiningMode::FixedBlockTime(_) => {}
        }
        match &mut *mode {
            MiningMode::FixedBlockTime(miner) | MiningMode::Mixed(_, miner) => {
                let period = miner.interval.period();
                *miner = FixedBlockTimeMiner::new(period);
            }
            MiningMode::None | MiningMode::Auto(_) => {}
        }
    }

    /// polls the [Pool] and returns those transactions that should be put in a block according to
    /// the current mode.
    ///
    /// May return an empty list, if no transactions are ready.
    pub(crate) fn poll(
        &mut self,
        pool: &Arc<Pool<T>>,
        cx: &mut Context<'_>,
    ) -> Poll<MiningWork<T>> {
        self.inner.register(cx);
        let mode_generation = self.generation.load(Ordering::Relaxed);
        if let Some((generation, mut transactions)) = self.force_transactions.take() {
            let mut batch = pool.mining_batch(None);
            if generation == batch.generation {
                transactions.append(&mut batch.transactions);
            }
            return Poll::Ready(MiningWork {
                batch: MiningBatch { generation, transactions },
                generation: mode_generation,
            });
        }
        let mut mode = self.mode.write();
        mode.poll_batch(pool, cx).map(|batch| MiningWork { batch, generation: mode_generation })
    }
}

/// Transactions selected by a specific mining mode generation.
pub(crate) struct MiningWork<T> {
    pub(crate) batch: MiningBatch<T>,
    pub(crate) generation: u64,
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

    /// A miner that uses both Auto and FixedBlockTime
    Mixed(ReadyTransactionMiner, FixedBlockTimeMiner),
}

impl MiningMode {
    pub fn instant(max_transactions: usize, listener: Receiver<TxHash>) -> Self {
        Self::Auto(ReadyTransactionMiner {
            max_transactions,
            has_pending_txs: None,
            rx: listener.fuse(),
            coalesce: None,
        })
    }

    pub fn interval(duration: Duration) -> Self {
        Self::FixedBlockTime(FixedBlockTimeMiner::new(duration))
    }

    pub fn mixed(max_transactions: usize, listener: Receiver<TxHash>, duration: Duration) -> Self {
        Self::Mixed(
            ReadyTransactionMiner {
                max_transactions,
                has_pending_txs: None,
                rx: listener.fuse(),
                coalesce: None,
            },
            FixedBlockTimeMiner::new(duration),
        )
    }

    /// polls the [Pool] and returns those transactions that should be put in a block, if any.
    pub fn poll<T>(
        &mut self,
        pool: &Arc<Pool<T>>,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<PoolTransaction<T>>>> {
        self.poll_batch(pool, cx).map(|batch| batch.transactions)
    }

    fn poll_batch<T>(&mut self, pool: &Arc<Pool<T>>, cx: &mut Context<'_>) -> Poll<MiningBatch<T>> {
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
                        for tx in fixed_txs.transactions {
                            // filter unique transactions
                            if auto_txs
                                .transactions
                                .iter()
                                .any(|auto_tx| auto_tx.hash() == tx.hash())
                            {
                                continue;
                            }
                            auto_txs.transactions.push(tx);
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

    fn poll<T>(&mut self, pool: &Arc<Pool<T>>, cx: &mut Context<'_>) -> Poll<MiningBatch<T>> {
        if self.interval.poll_tick(cx).is_ready() {
            // drain the pool
            return Poll::Ready(pool.mining_batch(None));
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
    /// Active [`INSTANT_COALESCE_WINDOW`] timer; while pending, ready txs are accumulated.
    coalesce: Option<Pin<Box<Sleep>>>,
}

impl ReadyTransactionMiner {
    fn poll<T>(&mut self, pool: &Arc<Pool<T>>, cx: &mut Context<'_>) -> Poll<MiningBatch<T>> {
        // always drain the notification stream so that we're woken up as soon as there's a new tx
        let mut saw_new_ready = false;
        while let Poll::Ready(Some(_hash)) = self.rx.poll_next_unpin(cx) {
            saw_new_ready = true;
        }

        // Arm the coalescing window only on fresh notifications to avoid delaying
        // consecutive chunks when draining a backlog larger than `max_transactions`.
        if saw_new_ready {
            self.has_pending_txs = Some(true);
            if self.coalesce.is_none() {
                self.coalesce = Some(Box::pin(tokio::time::sleep(INSTANT_COALESCE_WINDOW)));
            }
        }

        if self.has_pending_txs == Some(false) {
            return Poll::Pending;
        }

        if let Some(sleep) = self.coalesce.as_mut()
            && sleep.as_mut().poll(cx).is_pending()
        {
            return Poll::Pending;
        }
        self.coalesce = None;

        let batch = pool.mining_batch(Some(self.max_transactions));

        // there are pending transactions if we didn't drain the pool
        self.has_pending_txs = Some(batch.transactions.len() >= self.max_transactions);

        if batch.transactions.is_empty() {
            self.has_pending_txs = Some(false);
            return Poll::Pending;
        }

        Poll::Ready(batch)
    }
}

impl fmt::Debug for ReadyTransactionMiner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadyTransactionMiner")
            .field("max_transactions", &self.max_transactions)
            .finish_non_exhaustive()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, hex};
    use alloy_rlp::Decodable;
    use anvil_core::eth::transaction::PendingTransaction;
    use foundry_primitives::FoundryTxEnvelope;
    use futures::{channel::mpsc, future::poll_fn, task::noop_waker};

    fn forced_tx() -> PoolTransaction<FoundryTxEnvelope> {
        let raw = hex::decode("f86b02843b9aca00830186a094d3e8763675e4c425df46cc3b5c0f6cbdac39604687038d7ea4c68000802ba00eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5aea03a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca18").unwrap();
        let tx = FoundryTxEnvelope::decode(&mut &raw[..]).unwrap();
        let sender: Address = "0x95222290DD7278Aa3Ddd389Cc1E1d165CC4BAfe5".parse().unwrap();
        let pending = PendingTransaction::with_impersonated(tx, sender);
        PoolTransaction::new(pending)
    }

    #[test]
    fn poll_consumes_forced_transactions_before_mode_is_ready() {
        let forced = forced_tx();
        let forced_hash = forced.hash();

        let pool = Arc::new(Pool::default());
        let mut miner = Miner::new(MiningMode::None).with_forced_transactions(Some(vec![forced]));

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        let polled = miner.poll(&pool, &mut cx);
        let txs = match polled {
            Poll::Ready(work) => work.batch.transactions,
            Poll::Pending => panic!("expected forced transactions to be returned immediately"),
        };
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].hash(), forced_hash);

        // Forced transactions are consumed exactly once.
        assert!(miner.poll(&pool, &mut cx).is_pending());
    }

    #[test]
    fn forced_transactions_keep_their_original_generation() {
        let pool = Arc::new(Pool::default());
        let forced = forced_tx();
        let mut miner = Miner::new(MiningMode::None)
            .with_forced_transactions_at_generation(Some(vec![forced]), pool.generation());
        pool.reset();

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let Poll::Ready(work) = miner.poll(&pool, &mut cx) else {
            panic!("expected forced transactions to be returned immediately")
        };

        assert_ne!(work.batch.generation, pool.generation());
    }

    #[test]
    fn stale_failure_resumes_replacement_autominer() {
        let (_tx, rx) = mpsc::channel(1);
        let miner = Miner::<()>::new(MiningMode::None);
        miner.set_mining_mode(MiningMode::instant(1, rx));

        miner.handle_failed_candidate(0);

        let mode = miner.mode.read();
        let MiningMode::Auto(auto) = &*mode else { panic!("expected auto mining") };
        assert_eq!(auto.has_pending_txs, Some(true));
    }

    #[tokio::test]
    async fn failed_fixed_candidate_rearms_interval() {
        let mut miner = Miner::<()>::new(MiningMode::interval(Duration::from_millis(10)));
        let pool = Arc::new(Pool::default());
        tokio::time::timeout(Duration::from_secs(1), poll_fn(|cx| miner.poll(&pool, cx)))
            .await
            .unwrap();

        miner.handle_failed_candidate(0);

        tokio::time::timeout(Duration::from_secs(1), poll_fn(|cx| miner.poll(&pool, cx)))
            .await
            .unwrap();
    }
}
