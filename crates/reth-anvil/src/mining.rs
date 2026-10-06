use futures::Stream;
use reth_ethereum::pool::{TransactionListenerKind, TransactionPool};
use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    select,
    sync::{
        Notify,
        futures::OwnedNotified,
        watch::{Receiver, Sender, channel},
    },
    time::sleep,
};

/// The block production mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiningMode {
    /// Mine a block as soon as a transaction enters the pool.
    Automine,
    /// Mine only on explicit `anvil_mine` requests.
    Manual,
    /// Mine a block at a fixed interval.
    Interval(Duration),
}

/// Controls the local miner: the mining mode and on-demand block triggers.
#[derive(Debug, Clone)]
pub struct MiningController {
    mode_tx: Sender<MiningMode>,
    triggers: MiningTriggers,
}

impl Default for MiningController {
    fn default() -> Self {
        let (mode_tx, _) = channel(MiningMode::Automine);
        Self { mode_tx, triggers: MiningTriggers::default() }
    }
}

impl MiningController {
    /// Returns whether automine is enabled.
    pub fn is_automine(&self) -> bool {
        matches!(*self.mode_tx.borrow(), MiningMode::Automine)
    }

    /// Returns the interval mining period in seconds, if interval mining is enabled.
    pub fn interval_mining(&self) -> Option<u64> {
        match *self.mode_tx.borrow() {
            MiningMode::Interval(duration) => Some(duration.as_secs()),
            MiningMode::Automine | MiningMode::Manual => None,
        }
    }

    /// Enables or disables automine.
    pub fn set_automine(&self, enabled: bool) {
        let next_mode = match (*self.mode_tx.borrow(), enabled) {
            (MiningMode::Automine, true) => return,
            (MiningMode::Automine, false) => MiningMode::Manual,
            (_, true) => MiningMode::Automine,
            (_, false) => return,
        };

        self.mode_tx.send_replace(next_mode);
    }

    /// Sets interval mining. A zero interval switches to manual mining.
    pub fn set_interval_mining(&self, interval_secs: u64) {
        self.mode_tx.send_replace(if interval_secs == 0 {
            MiningMode::Manual
        } else {
            MiningMode::Interval(Duration::from_secs(interval_secs))
        });
    }

    /// Subscribes to mining mode changes.
    pub fn subscribe_mode(&self) -> Receiver<MiningMode> {
        self.mode_tx.subscribe()
    }

    /// Requests one block.
    pub fn trigger(&self) {
        self.triggers.trigger();
    }

    /// Returns the stream of block requests consumed by the local miner.
    pub fn trigger_stream(&self) -> MiningTriggerStream {
        self.triggers.stream()
    }
}

#[derive(Debug, Clone, Default)]
struct MiningTriggers {
    pending: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

impl MiningTriggers {
    fn trigger(&self) {
        self.pending.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_one();
    }

    fn stream(&self) -> MiningTriggerStream {
        MiningTriggerStream {
            pending: Arc::clone(&self.pending),
            notify: Arc::clone(&self.notify),
            notified: None,
        }
    }
}

/// Yields one item per requested block.
#[derive(Debug)]
pub struct MiningTriggerStream {
    pending: Arc<AtomicUsize>,
    notify: Arc<Notify>,
    notified: Option<Pin<Box<OwnedNotified>>>,
}

impl Stream for MiningTriggerStream {
    type Item = ();

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            let pending = self.pending.load(Ordering::Acquire);
            if pending > 0 {
                if self
                    .pending
                    .compare_exchange(pending, pending - 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Poll::Ready(Some(()));
                }
                continue;
            }

            if self.notified.is_none() {
                self.notified = Some(Box::pin(Arc::clone(&self.notify).notified_owned()));
            }

            if self.notified.as_mut().unwrap().as_mut().poll(cx).is_ready() {
                self.notified = None;
                continue;
            }

            return Poll::Pending;
        }
    }
}

/// Requests a block for every pool transaction while automine is enabled.
pub async fn run_automine_task<Pool>(pool: Pool, mining: MiningController)
where
    Pool: TransactionPool + Clone + Unpin + Send + Sync + 'static,
{
    let mut pending_txs = pool.pending_transactions_listener_for(TransactionListenerKind::All);

    while pending_txs.recv().await.is_some() {
        if mining.is_automine() && pool.pending_and_queued_txn_count().0 > 0 {
            mining.trigger();
        }
    }
}

/// Requests a block at the configured interval while interval mining is enabled.
pub async fn run_interval_mining_task(mining: MiningController) {
    let mut mode_rx = mining.subscribe_mode();

    loop {
        let mode = *mode_rx.borrow_and_update();
        match mode {
            MiningMode::Automine | MiningMode::Manual => {
                if mode_rx.changed().await.is_err() {
                    return;
                }
            }
            MiningMode::Interval(duration) => {
                select! {
                    changed = mode_rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                    _ = sleep(duration) => {
                        if matches!(*mode_rx.borrow(), MiningMode::Interval(_)) {
                            mining.trigger();
                        }
                    }
                }
            }
        }
    }
}
