//! Mines transactions

use crate::eth::pool::{Pool, transactions::PoolTransaction};
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
use tokio::time::{Instant, Sleep};

/// Default delay before instant mining. Zero avoids delaying receipt availability.
pub(crate) const INSTANT_COALESCE_WINDOW: Duration = Duration::ZERO;

/// How far ahead to schedule a block whose deadline overflows an instant, as tokio's timers do.
const FAR_FUTURE: Duration = Duration::from_secs(86400 * 365 * 30);

pub struct Miner<T> {
    /// The mode this miner currently operates in
    mode: Arc<RwLock<MiningMode>>,
    /// Identifies the current mode and pause state so stale candidate failures cannot modify the
    /// schedule that replaced them.
    generation: Arc<AtomicU64>,
    /// Incremented whenever automatic block production is paused or resumed, so it is paused
    /// while odd, see [`Miner::set_paused`]. Candidates selected before a pause compare their
    /// epoch to skip mining after it, even once mining resumed.
    pause_epoch: Arc<AtomicU64>,
    /// used for task wake up when the mining mode was forcefully changed
    ///
    /// This will register the task so we can manually wake it up if the mining mode was changed
    inner: Arc<MinerInner>,
    /// Transaction type handled by the associated pool.
    transaction: PhantomData<fn() -> T>,
}

impl<T> Clone for Miner<T> {
    fn clone(&self) -> Self {
        Self {
            mode: self.mode.clone(),
            generation: self.generation.clone(),
            pause_epoch: self.pause_epoch.clone(),
            inner: self.inner.clone(),
            transaction: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Miner<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Miner").field("mode", &self.mode).finish_non_exhaustive()
    }
}

impl<T> Miner<T> {
    /// Returns a new miner with that operates in the given `mode`.
    pub fn new(mode: MiningMode) -> Self {
        Self {
            mode: Arc::new(RwLock::new(mode)),
            generation: Default::default(),
            pause_epoch: Default::default(),
            inner: Default::default(),
            transaction: PhantomData,
        }
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
            return Some(mm.period.as_secs());
        }
        None
    }

    /// Returns the configured block interval for fixed or mixed mining.
    pub(crate) fn block_interval(&self) -> Option<Duration> {
        let mode = self.mode.read();
        match &*mode {
            MiningMode::FixedBlockTime(miner) | MiningMode::Mixed(_, miner) => Some(miner.period),
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
        // Selecting the candidate already cleared `has_pending_txs`, so keep any retry requested
        // since then, e.g. by a block mined via `evm_mine` before this failure was processed.
        if let MiningMode::Auto(miner) | MiningMode::Mixed(miner, _) = &mut *mode {
            miner.coalesce = None;
        }
        match &mut *mode {
            MiningMode::FixedBlockTime(miner) | MiningMode::Mixed(_, miner) => {
                *miner = FixedBlockTimeMiner::new(miner.period);
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
        let mut mode = self.mode.write();
        if self.is_paused() {
            mode.poll_paused(cx);
            return Poll::Pending;
        }
        let generation = self.generation.load(Ordering::Relaxed);
        let pause_epoch = self.pause_epoch.load(Ordering::SeqCst);
        mode.poll(pool, cx).map(|transactions| MiningWork { transactions, generation, pause_epoch })
    }

    /// Makes the instant miner select ready transactions again without waiting for a new one.
    ///
    /// Used when a mined block left ready transactions behind, e.g. because it ran out of gas.
    pub(crate) fn retry_ready_transactions(&self) {
        if let MiningMode::Auto(miner) | MiningMode::Mixed(miner, _) = &mut *self.mode.write() {
            miner.has_pending_txs = Some(true);
            self.inner.wake();
        }
    }

    /// Pauses or resumes automatic block production.
    ///
    /// While paused, the block time interval keeps ticking but blocks that come due are skipped,
    /// so the next block is mined on the first tick after mining resumes. Ready transactions in
    /// auto mining mode are mined once mining resumes. The pause persists across mining mode
    /// changes. Manual mining, e.g. via `evm_mine`, is not affected.
    pub fn set_paused(&self, paused: bool) {
        let mut mode = self.mode.write();
        if paused != self.is_paused() {
            if !paused {
                mode.resume();
            }
            self.generation.fetch_add(1, Ordering::Relaxed);
            self.pause_epoch.fetch_add(1, Ordering::SeqCst);
        }
        drop(mode);
        trace!(target: "miner", paused, "updated mining pause");
        self.inner.wake();
    }

    /// Returns `true` if automatic block production is paused.
    pub(crate) fn is_paused(&self) -> bool {
        self.pause_epoch.load(Ordering::SeqCst) % 2 == 1
    }

    /// Returns `true` if mining was paused since a candidate was selected at `pause_epoch`.
    pub(crate) fn is_stale(&self, pause_epoch: u64) -> bool {
        self.pause_epoch.load(Ordering::SeqCst) != pause_epoch
    }
}

/// Transactions selected by a specific mining mode generation.
pub(crate) struct MiningWork<T> {
    pub(crate) transactions: Vec<Arc<PoolTransaction<T>>>,
    pub(crate) generation: u64,
    /// Pause epoch when the transactions were selected, see [`Miner::is_stale`].
    pub(crate) pause_epoch: u64,
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
            coalesce_window: INSTANT_COALESCE_WINDOW,
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
                coalesce_window: INSTANT_COALESCE_WINDOW,
            },
            FixedBlockTimeMiner::new(duration),
        )
    }

    /// Sets the window for grouping ready transactions in auto or mixed mining mode.
    /// A zero window mines ready transactions without a coalescing timer.
    #[must_use]
    pub fn with_coalescing_window(mut self, window: Duration) -> Self {
        if let Self::Auto(miner) | Self::Mixed(miner, _) = &mut self {
            miner.coalesce_window = window;
            miner.coalesce = None;
        }
        self
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

    /// Keeps the mode's notifications moving while mining is paused, without selecting any
    /// transactions.
    fn poll_paused(&mut self, cx: &mut Context<'_>) {
        if let Self::Auto(miner) | Self::Mixed(miner, _) = self {
            miner.poll_notifications(cx);
        }
    }

    /// Skips the block time ticks that came due while mining was paused, and makes auto mining
    /// select ready transactions again, including any left behind by a failed block.
    fn resume(&mut self) {
        if let Self::Auto(miner) | Self::Mixed(miner, _) = self {
            miner.has_pending_txs = Some(true);
        }
        if let Self::FixedBlockTime(miner) | Self::Mixed(_, miner) = self {
            miner.skip_missed_ticks();
        }
    }
}

/// A miner that's supposed to create a new block every `interval`, mining all transactions that are
/// ready at that time.
///
/// The default blocktime is set to 6 seconds
#[derive(Debug)]
pub struct FixedBlockTimeMiner {
    /// The time between blocks.
    period: Duration,
    /// Completes when the next block is due.
    ///
    /// This owns the schedule instead of an [`Interval`](tokio::time::Interval) so that resuming
    /// can skip ticks by their deadline, which the interval does not expose.
    next_tick: Pin<Box<Sleep>>,
}

impl FixedBlockTimeMiner {
    /// Creates a new instance with an interval of `duration`
    pub fn new(duration: Duration) -> Self {
        assert!(!duration.is_zero(), "`period` must be non-zero.");
        let next_tick = Box::pin(tokio::time::sleep(duration));
        Self { period: duration, next_tick }
    }

    fn poll<T>(
        &mut self,
        pool: &Arc<Pool<T>>,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<PoolTransaction<T>>>> {
        if self.next_tick.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        let next = next_deadline(self.next_tick.deadline(), Instant::now(), self.period);
        self.next_tick.as_mut().reset(next);
        // Drain the pool.
        Poll::Ready(pool.ready_transactions().collect())
    }

    /// Moves the next tick past any ticks that came due while mining was paused, keeping the
    /// schedule.
    ///
    /// This compares deadlines to the current time instead of polling the timer, which may not
    /// have fired yet for a deadline that already passed.
    fn skip_missed_ticks(&mut self) {
        if let Some(next) = resumed_deadline(self.next_tick.deadline(), Instant::now(), self.period)
        {
            self.next_tick.as_mut().reset(next);
        }
    }
}

/// Returns the deadline of the tick after the one due at `deadline`, which fired at `now`.
///
/// Schedules like an interval with `MissedTickBehavior::Delay`, so that ticks are not shortened
/// and late ticks shift the schedule rather than producing a burst of blocks.
fn next_deadline(deadline: Instant, now: Instant, period: Duration) -> Instant {
    let next = if now > deadline + Duration::from_millis(5) {
        now.checked_add(period)
    } else {
        deadline.checked_add(period)
    };
    next.unwrap_or_else(|| now + FAR_FUTURE)
}

/// Returns the first deadline after `now` on the schedule of `deadline`, or `None` if the tick
/// due at `deadline` has not come due.
fn resumed_deadline(deadline: Instant, now: Instant, period: Duration) -> Option<Instant> {
    if deadline > now {
        return None;
    }
    let late = (now - deadline).as_nanos() % period.as_nanos();
    let next = now.checked_add(period - Duration::from_nanos_u128(late));
    Some(next.unwrap_or_else(|| now + FAR_FUTURE))
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
    /// Delay for accumulating ready transactions; zero disables coalescing.
    coalesce_window: Duration,
    /// Active coalescing timer; while pending, ready txs are accumulated.
    coalesce: Option<Pin<Box<Sleep>>>,
}

impl ReadyTransactionMiner {
    fn poll<T>(
        &mut self,
        pool: &Arc<Pool<T>>,
        cx: &mut Context<'_>,
    ) -> Poll<Vec<Arc<PoolTransaction<T>>>> {
        // Arm the coalescing window only on fresh notifications to avoid delaying
        // consecutive chunks when draining a backlog larger than `max_transactions`.
        if self.poll_notifications(cx) && self.coalesce.is_none() && !self.coalesce_window.is_zero()
        {
            self.coalesce = Some(Box::pin(tokio::time::sleep(self.coalesce_window)));
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

        let transactions =
            pool.ready_transactions().take(self.max_transactions).collect::<Vec<_>>();

        // Whether ready txs left behind are worth another block is decided once this block is
        // mined, see `Miner::retry_ready_transactions`.
        self.has_pending_txs = Some(false);

        if transactions.is_empty() {
            return Poll::Pending;
        }

        Poll::Ready(transactions)
    }

    /// Drains ready notifications and reports whether any arrived.
    fn poll_notifications(&mut self, cx: &mut Context<'_>) -> bool {
        let mut saw_new_ready = false;
        while let Poll::Ready(Some(_hash)) = self.rx.poll_next_unpin(cx) {
            saw_new_ready = true;
        }
        if saw_new_ready {
            self.has_pending_txs = Some(true);
        }
        saw_new_ready
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
    use futures::{channel::mpsc, future::poll_fn};

    fn with_fixed<R>(miner: &Miner<()>, f: impl FnOnce(&mut FixedBlockTimeMiner) -> R) -> R {
        let mut mode = miner.mode.write();
        let (MiningMode::FixedBlockTime(fixed) | MiningMode::Mixed(_, fixed)) = &mut *mode else {
            panic!("expected block time mining")
        };
        f(fixed)
    }

    fn deadline(miner: &Miner<()>) -> Instant {
        with_fixed(miner, |fixed| fixed.next_tick.deadline())
    }

    fn set_deadline(miner: &Miner<()>, deadline: Instant) {
        with_fixed(miner, |fixed| fixed.next_tick.as_mut().reset(deadline));
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

    #[test]
    fn failure_keeps_retry_requested_after_candidate_selection() {
        let (_tx, rx) = mpsc::channel(1);
        let miner = Miner::<()>::new(MiningMode::instant(1, rx));
        miner.retry_ready_transactions();

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

    #[tokio::test]
    async fn coalescing_window_controls_fresh_notifications() {
        for mixed in [false, true] {
            for window in [None, Some(Duration::ZERO), Some(Duration::from_secs(60))] {
                let (mut tx, rx) = mpsc::channel(1);
                let mode = if mixed {
                    MiningMode::mixed(1, rx, Duration::from_secs(120))
                } else {
                    MiningMode::instant(1, rx)
                };
                let mut mode = if let Some(window) = window {
                    mode.with_coalescing_window(window)
                } else {
                    mode
                };
                let pool = Arc::new(Pool::<()>::default());
                tx.try_send(TxHash::ZERO).unwrap();
                let mut cx = Context::from_waker(futures::task::noop_waker_ref());
                assert!(mode.poll(&pool, &mut cx).is_pending());
                let auto = match &mode {
                    MiningMode::Auto(auto) | MiningMode::Mixed(auto, _) => auto,
                    _ => unreachable!(),
                };
                if window.is_none_or(|window| window.is_zero()) {
                    assert!(auto.coalesce.is_none());
                    assert_eq!(auto.has_pending_txs, Some(false));
                } else {
                    assert!(auto.coalesce.is_some());
                    assert_eq!(auto.has_pending_txs, Some(true));
                }
            }
        }
    }

    #[tokio::test]
    async fn paused_interval_skips_due_block() {
        let t0 = Instant::now();
        let period = Duration::from_secs(1);
        assert_eq!(resumed_deadline(t0 + period, t0 + Duration::from_millis(500), period), None);
        // Resume must skip overdue ticks without moving the next tick from 4s to 4.5s.
        assert_eq!(
            resumed_deadline(t0 + period, t0 + Duration::from_millis(3500), period),
            Some(t0 + period * 4)
        );
        assert_eq!(resumed_deadline(t0 + period, t0 + period * 4, period), Some(t0 + period * 5));

        let period = Duration::from_millis(100);
        for mixed in [false, true] {
            for poll_while_paused in [false, true] {
                let (_tx, rx) = mpsc::channel(1);
                let mode = if mixed {
                    MiningMode::mixed(1, rx, period)
                } else {
                    MiningMode::interval(period)
                };
                let mut miner = Miner::<()>::new(mode);
                let pool = Arc::new(Pool::default());
                let mut cx = Context::from_waker(futures::task::noop_waker_ref());
                let due = Instant::now() - period * 5 / 2;
                set_deadline(&miner, due);
                miner.set_paused(true);
                if poll_while_paused {
                    assert!(miner.poll(&pool, &mut cx).is_pending());
                }

                let before = Instant::now();
                miner.set_paused(false);
                let next = deadline(&miner);
                assert!(next > before, "overdue tick not skipped");
                assert_eq!((next - due).as_nanos() % period.as_nanos(), 0, "schedule moved");
            }
        }
    }

    #[test]
    fn paused_autominer_keeps_ready_notifications() {
        let (mut tx, rx) = mpsc::channel(1);
        let mut miner = Miner::<()>::new(MiningMode::instant(1, rx));
        let pool = Arc::new(Pool::default());
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        miner.set_paused(true);
        // A channel of size 1 with one sender buffers two notifications.
        let fill = |tx: &mut mpsc::Sender<TxHash>| {
            tx.try_send(TxHash::ZERO).unwrap();
            tx.try_send(TxHash::ZERO).unwrap();
        };
        fill(&mut tx);

        assert!(miner.poll(&pool, &mut cx).is_pending());
        // Polling while paused drained the notifications.
        fill(&mut tx);
        let mode = miner.mode.read();
        let MiningMode::Auto(auto) = &*mode else { panic!("expected auto mining") };
        assert_eq!(auto.has_pending_txs, Some(true));
    }

    #[tokio::test]
    async fn resume_skips_due_tick_before_timer_fires() {
        let period = Duration::from_millis(100);
        let mut miner = Miner::<()>::new(MiningMode::interval(period));
        let pool = Arc::new(Pool::default());
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        assert!(miner.poll(&pool, &mut cx).is_pending());
        miner.set_paused(true);

        // Resume decides from the deadline alone, so it skips the tick even though the timer
        // never got to fire.
        let due = Instant::now() - period / 2;
        set_deadline(&miner, due);
        let before = Instant::now();
        miner.set_paused(false);

        let next = deadline(&miner);
        assert!(next > before, "overdue tick not skipped");
        assert_eq!((next - due).as_nanos() % period.as_nanos(), 0, "schedule moved");
    }

    #[tokio::test]
    async fn late_tick_delays_schedule() {
        let t0 = Instant::now();
        let period = Duration::from_secs(1);
        // A tick polled on time keeps the schedule.
        assert_eq!(next_deadline(t0 + period, t0 + period, period), t0 + period * 2);
        assert_eq!(
            next_deadline(t0 + period, t0 + period + Duration::from_millis(5), period),
            t0 + period * 2
        );
        // A late tick delays it by its lateness.
        assert_eq!(
            next_deadline(t0 + period * 2, t0 + Duration::from_millis(2500), period),
            t0 + Duration::from_millis(3500)
        );

        let period = Duration::from_millis(100);
        let mut miner = Miner::<()>::new(MiningMode::interval(period));
        let pool = Arc::new(Pool::default());
        set_deadline(&miner, Instant::now() - period * 3 / 2);
        let before = Instant::now();
        poll_fn(|cx| miner.poll(&pool, cx)).await;
        let after = Instant::now();
        let next = deadline(&miner);
        assert!(next >= before + period && next <= after + period, "late tick kept schedule");
    }

    #[tokio::test]
    async fn failure_selected_before_pause_keeps_schedule() {
        let period = Duration::from_secs(3600);
        let miner = Miner::<()>::new(MiningMode::interval(period));
        let next = Instant::now() + period * 2;
        set_deadline(&miner, next);
        miner.set_paused(true);
        miner.set_paused(false);

        // The candidate selected at generation 0, before the pause, fails after mining resumed.
        miner.handle_failed_candidate(0);

        assert_eq!(deadline(&miner), next);
    }

    #[test]
    fn resume_retries_ready_transactions_after_failure() {
        let (_tx, rx) = mpsc::channel(1);
        let mut miner = Miner::<()>::new(MiningMode::instant(1, rx));
        let pool = Arc::new(Pool::default());
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        // The candidate failed before the pause, so its transactions are no longer retried.
        assert!(miner.poll(&pool, &mut cx).is_pending());
        miner.handle_failed_candidate(0);
        miner.set_paused(true);
        miner.set_paused(false);

        let mode = miner.mode.read();
        let MiningMode::Auto(auto) = &*mode else { panic!("expected auto mining") };
        assert_eq!(auto.has_pending_txs, Some(true));
    }

    #[test]
    fn overflowing_tick_waits_indefinitely() {
        let t0 = Instant::now();
        let now = t0 + Duration::from_secs(1);
        assert_eq!(next_deadline(t0, now, Duration::MAX), now + FAR_FUTURE);
        assert_eq!(resumed_deadline(t0, now, Duration::MAX), Some(now + FAR_FUTURE));
    }
}
