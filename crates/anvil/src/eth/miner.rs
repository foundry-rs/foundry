//! Mines transactions

use crate::eth::pool::{Pool, transactions::PoolTransaction};
use alloy_primitives::TxHash;
use foundry_primitives::FoundryTxEnvelope;
use futures::{
    channel::mpsc::Receiver,
    stream::{Fuse, StreamExt},
    task::AtomicWaker,
};
use parking_lot::{RawRwLock, RwLock, lock_api::RwLockWriteGuard};
use std::{
    fmt,
    sync::Arc,
    task::{Context, Poll, ready},
    time::Duration,
};
use tokio::time::{Interval, MissedTickBehavior};

pub struct Miner<T = FoundryTxEnvelope> {
    /// The mode this miner currently operates in
    mode: Arc<RwLock<MiningMode>>,
    /// used for task wake up when the mining mode was forcefully changed
    ///
    /// This will register the task so we can manually wake it up if the mining mode was changed
    inner: Arc<MinerInner>,
    /// Transactions included into the pool before any others are.
    /// Done once on startup.
    force_transactions: Option<Vec<Arc<PoolTransaction<T>>>>,
}

impl<T> Clone for Miner<T> {
    fn clone(&self) -> Self {
        Self {
            mode: self.mode.clone(),
            inner: self.inner.clone(),
            force_transactions: self.force_transactions.clone(),
        }
    }
}

impl<T> fmt::Debug for Miner<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Miner")
            .field("mode", &self.mode)
            .field("force_transactions", &self.force_transactions.as_ref().map(|txs| txs.len()))
            .finish_non_exhaustive()
    }
}

impl<T> Miner<T> {
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
        force_transactions: Option<Vec<PoolTransaction<T>>>,
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

    pub fn get_interval(&self) -> Option<u64> {
        let mode = self.mode.read();
        if let MiningMode::FixedBlockTime(ref mm) = *mode {
            return Some(mm.interval.period().as_secs());
        }
        None
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
        pool: &Arc<Pool<T>>,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<PoolTransaction<T>>>> {
        self.inner.register(cx);

        // Consume force transactions immediately on the first poll, without waiting for the mining
        // mode to signal readiness. This avoids a deadlock: in automine mode the miner waits for
        // pool readiness, but a user-submitted tx may depend on a force_transaction (same sender,
        // next nonce). Since force_transactions live outside the pool, readiness never comes.
        if let Some(transactions) = self.force_transactions.take() {
            return Poll::Ready(transactions);
        }

        let next = ready!(self.mode.write().poll(pool, cx));
        Poll::Ready(next)
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

    /// A miner that uses both Auto and FixedBlockTime
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
    pub fn poll<T>(
        &mut self,
        pool: &Arc<Pool<T>>,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<PoolTransaction<T>>>> {
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

    fn poll<T>(
        &mut self,
        pool: &Arc<Pool<T>>,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<PoolTransaction<T>>>> {
        if self.interval.poll_tick(cx).is_ready() {
            // drain the pool
            return Poll::Ready(pool.ready_transactions().collect());
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
    fn poll<T>(
        &mut self,
        pool: &Arc<Pool<T>>,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<PoolTransaction<T>>>> {
        // always drain the notification stream so that we're woken up as soon as there's a new tx
        while let Poll::Ready(Some(_hash)) = self.rx.poll_next_unpin(cx) {
            self.has_pending_txs = Some(true);
        }

        if self.has_pending_txs == Some(false) {
            return Poll::Pending;
        }

        let transactions =
            pool.ready_transactions().take(self.max_transactions).collect::<Vec<_>>();

        // there are pending transactions if we didn't drain the pool
        self.has_pending_txs = Some(transactions.len() >= self.max_transactions);

        if transactions.is_empty() {
            return Poll::Pending;
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{Signed, TxLegacy};
    use alloy_primitives::{Address, Bytes, TxKind, U256, b256};
    use alloy_signer::Signature;
    use anvil_core::eth::transaction::PendingTransaction;
    use std::pin::Pin;
    use std::str::FromStr;

    /// Creates a dummy [`PoolTransaction`] for testing.
    fn dummy_pool_tx() -> PoolTransaction {
        let tx = TxLegacy {
            nonce: 0,
            gas_price: 1_000_000_000,
            gas_limit: 21_000,
            to: TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::default(),
            chain_id: Some(1),
        };
        let signature = Signature::from_str(
            "0eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5ae\
             3a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca18\
             2b",
        )
        .unwrap();
        let envelope = FoundryTxEnvelope::Legacy(Signed::new_unchecked(
            tx,
            signature,
            b256!("0xa517b206d2223278f860ea017d3626cacad4f52ff51030dc9a96b432f17f8d34"),
        ));
        let pending = PendingTransaction::with_impersonated(envelope, Address::ZERO);
        PoolTransaction::new(pending)
    }

    /// Regression test for <https://github.com/foundry-rs/foundry/issues/13630>.
    ///
    /// `force_transactions` must be returned on the first poll even when the mining mode returns
    /// `Poll::Pending` (empty pool). Previously, `ready!()` on the mining mode blocked the
    /// entire `Miner::poll`, so force_transactions were never consumed — a permanent deadlock.
    #[tokio::test]
    async fn force_transactions_returned_when_mining_mode_pending() {
        let pool: Arc<Pool> = Arc::new(Pool::default());

        // MiningMode::None always returns Poll::Pending — simulates an empty pool in automine.
        let mut miner = Miner::new(MiningMode::None)
            .with_forced_transactions(Some(vec![dummy_pool_tx()]));

        // First poll must return force_transactions immediately.
        let result = futures::future::poll_fn(|cx| miner.poll(&pool, cx)).await;
        assert_eq!(result.len(), 1, "force_transactions should be returned on first poll");

        // Second poll with MiningMode::None and empty pool should be pending (no more forced txs).
        let mut fut = futures::future::poll_fn(|cx| miner.poll(&pool, cx));
        let pending = Pin::new(&mut fut);
        let waker = futures::task::noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);
        assert!(
            pending.poll(&mut cx).is_pending(),
            "second poll should be Pending with MiningMode::None and empty pool"
        );
    }
}
