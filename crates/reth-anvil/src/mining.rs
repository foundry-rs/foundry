use alloy_consensus::Header;
use alloy_primitives::B256;
use reth_ethereum::{pool::TransactionPool, primitives::SealedHeader};
use std::time::{Duration, Instant};
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
pub enum MinerRequest<H = Header> {
    /// Builds and inserts one block. The sender, if any, receives the mined header.
    Mine(Option<oneshot::Sender<Result<SealedHeader<H>, String>>>),
    /// Builds and inserts one block if the pool holds pending transactions.
    MineIfPending,
    /// Rewinds the chain head to the given canonical header.
    Rewind(Box<SealedHeader<H>>, oneshot::Sender<Result<(), String>>),
}

/// Controls the miner task: the mining mode, block requests, and head rewinds.
#[derive(Debug)]
pub struct MiningController<H = Header> {
    mode_tx: Sender<MiningMode>,
    requests: UnboundedSender<MinerRequest<H>>,
}

impl<H> Clone for MiningController<H> {
    fn clone(&self) -> Self {
        Self { mode_tx: self.mode_tx.clone(), requests: self.requests.clone() }
    }
}

impl<H> MiningController<H> {
    /// Creates a controller in the given mode and the request stream the miner task consumes.
    pub fn new(mode: MiningMode) -> (Self, UnboundedReceiver<MinerRequest<H>>) {
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

    /// Requests one block for the pending transactions without waiting for it.
    pub fn trigger_if_pending(&self) {
        let _ = self.requests.send(MinerRequest::MineIfPending);
    }

    /// Mines one block and returns its header.
    pub async fn mine_block(&self) -> Result<SealedHeader<H>, String> {
        let (tx, rx) = oneshot::channel();
        self.requests
            .send(MinerRequest::Mine(Some(tx)))
            .map_err(|_| "the miner task has stopped".to_string())?;
        rx.await.map_err(|_| "the miner task has stopped".to_string())?
    }

    /// Rewinds the chain head to the given canonical header.
    pub async fn rewind(&self, header: SealedHeader<H>) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.requests
            .send(MinerRequest::Rewind(Box::new(header), tx))
            .map_err(|_| "the miner task has stopped".to_string())?;
        rx.await.map_err(|_| "the miner task has stopped".to_string())?
    }
}

/// How long the instant miner waits for more transactions after one arrives, so transactions
/// sent together, in one JSON-RPC batch or in parallel, land in one block.
const INSTANT_COALESCE_WINDOW: Duration = Duration::from_millis(5);

/// How long the instant miner waits for the pool to see a mined block.
const POOL_SYNC_TIMEOUT: Duration = Duration::from_secs(2);

/// Requests a block for every transaction that enters the pool while automine is enabled. Only
/// new transactions request blocks: a transaction a block left behind is picked up by the
/// follow-up blocks, or by the next new transaction.
pub async fn run_automine_task<Pool, H>(pool: Pool, mining: MiningController<H>)
where
    Pool: TransactionPool + Clone + Unpin + Send + Sync + 'static,
{
    let mut new_txs = pool.new_transactions_listener();

    while new_txs.recv().await.is_some() {
        sleep(INSTANT_COALESCE_WINDOW).await;
        while new_txs.try_recv().is_ok() {}
        if mining.is_automine() {
            mining.trigger_if_pending();
        }
    }
}

/// Requests a block at the configured interval while interval mining is enabled.
pub async fn run_interval_mining_task<H>(mining: MiningController<H>) {
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

/// The number of pending and of queued transactions in the pool.
pub type PoolCounts = (usize, usize);

/// Returns the pool counts once the pool has seen the block `head`, if the pool holds pending
/// transactions. The pool learns of a block after the miner does, so a check right after a
/// block waits for it.
pub async fn pool_pending_after<Pool: TransactionPool>(
    pool: Pool,
    head: B256,
) -> Option<PoolCounts> {
    let deadline = Instant::now() + POOL_SYNC_TIMEOUT;
    while pool.block_info().last_seen_block_hash != head {
        if Instant::now() >= deadline {
            return None;
        }
        sleep(Duration::from_millis(1)).await;
    }
    let counts = pool.pending_and_queued_txn_count();
    (counts.0 > 0).then_some(counts)
}
