use reth_ethereum::{
    pool::{TransactionListenerKind, TransactionPool},
    primitives::SealedHeader,
};
use std::time::Duration;
use tokio::{
    select,
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
        oneshot,
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
    /// Mine a block per transaction and at a fixed interval.
    Mixed(Duration),
}

/// A request for the miner task.
#[derive(Debug)]
pub enum MinerRequest {
    /// Builds and inserts one block. The sender, if any, receives the mined header.
    Mine(Option<oneshot::Sender<Result<SealedHeader, String>>>),
    /// Rewinds the chain head to the given canonical header.
    Rewind(Box<SealedHeader>, oneshot::Sender<Result<(), String>>),
}

/// Controls the miner task: the mining mode, block requests, and head rewinds.
#[derive(Debug, Clone)]
pub struct MiningController {
    mode_tx: Sender<MiningMode>,
    requests: UnboundedSender<MinerRequest>,
}

impl MiningController {
    /// Creates a controller in the given mode and the request stream the miner task consumes.
    pub fn new(mode: MiningMode) -> (Self, UnboundedReceiver<MinerRequest>) {
        let (mode_tx, _) = channel(mode);
        let (requests, rx) = unbounded_channel();
        (Self { mode_tx, requests }, rx)
    }

    /// Returns whether automine is enabled.
    pub fn is_automine(&self) -> bool {
        matches!(*self.mode_tx.borrow(), MiningMode::Automine | MiningMode::Mixed(_))
    }

    /// Returns the interval mining period in seconds, if interval mining is enabled.
    pub fn interval_mining(&self) -> Option<u64> {
        match *self.mode_tx.borrow() {
            MiningMode::Interval(duration) | MiningMode::Mixed(duration) => {
                Some(duration.as_secs())
            }
            MiningMode::Automine | MiningMode::Manual => None,
        }
    }

    /// Enables or disables automine. The interval, if any, stays.
    pub fn set_automine(&self, enabled: bool) {
        let next_mode = match (*self.mode_tx.borrow(), enabled) {
            (MiningMode::Automine, false) => MiningMode::Manual,
            (MiningMode::Mixed(duration), false) => MiningMode::Interval(duration),
            (MiningMode::Manual, true) => MiningMode::Automine,
            (MiningMode::Interval(duration), true) => MiningMode::Mixed(duration),
            _ => return,
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

    /// Requests one block without waiting for it.
    pub fn trigger(&self) {
        let _ = self.requests.send(MinerRequest::Mine(None));
    }

    /// Mines one block and returns its header.
    pub async fn mine_block(&self) -> Result<SealedHeader, String> {
        let (tx, rx) = oneshot::channel();
        self.requests
            .send(MinerRequest::Mine(Some(tx)))
            .map_err(|_| "the miner task has stopped".to_string())?;
        rx.await.map_err(|_| "the miner task has stopped".to_string())?
    }

    /// Rewinds the chain head to the given canonical header.
    pub async fn rewind(&self, header: SealedHeader) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.requests
            .send(MinerRequest::Rewind(Box::new(header), tx))
            .map_err(|_| "the miner task has stopped".to_string())?;
        rx.await.map_err(|_| "the miner task has stopped".to_string())?
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
            MiningMode::Interval(duration) | MiningMode::Mixed(duration) => {
                select! {
                    changed = mode_rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                    _ = sleep(duration) => {
                        if matches!(*mode_rx.borrow(), MiningMode::Interval(_) | MiningMode::Mixed(_)) {
                            mining.trigger();
                        }
                    }
                }
            }
        }
    }
}
