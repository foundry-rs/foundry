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
use tracing::{error, warn};

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
    /// Builds and inserts one block. The sender receives the mined header.
    Mine(oneshot::Sender<Result<SealedHeader<H>, String>>),
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

    /// Returns the interval mining period, if interval mining is enabled.
    pub fn interval(&self) -> Option<Duration> {
        match *self.mode_tx.borrow() {
            MiningMode::Interval(duration) | MiningMode::Mixed(duration) => Some(duration),
            MiningMode::Automine | MiningMode::Manual => None,
        }
    }

    /// Returns the interval mining period in seconds, if interval mining is enabled.
    pub fn interval_mining(&self) -> Option<u64> {
        self.interval().map(|duration| duration.as_secs())
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

    /// Requests one block for the pending transactions without waiting for it.
    pub fn trigger_if_pending(&self) {
        let _ = self.requests.send(MinerRequest::MineIfPending);
    }

    /// Mines one block and returns its header.
    pub async fn mine_block(&self) -> Result<SealedHeader<H>, String> {
        let (tx, rx) = oneshot::channel();
        self.requests
            .send(MinerRequest::Mine(tx))
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

/// How long the instant miner waits for the pool to see a mined block before it reads the pool
/// anyway.
const POOL_SYNC_TIMEOUT: Duration = Duration::from_secs(10);

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
                        // Wait for the block, so blocks slower than the interval do not queue
                        // requests that a switch to manual mining would still mine.
                        if matches!(*mode_rx.borrow(), MiningMode::Interval(_) | MiningMode::Mixed(_))
                            && let Err(error) = mining.mine_block().await
                        {
                            error!(target: "reth_anvil::miner", %error, "failed to mine block");
                        }
                    }
                }
            }
        }
    }
}

/// The hashes of the pending transactions in the pool, sorted.
pub type PendingTxs = Vec<B256>;

/// Returns the pending transactions once the pool has seen the block `head`, if there are any.
/// The pool learns of a block after the miner does, so a check right after a block waits for
/// it; a pool that stays behind is read as it is, so no transaction waits for a block that
/// never comes.
pub async fn pool_pending_after<Pool: TransactionPool>(
    pool: Pool,
    head: B256,
) -> Option<PendingTxs> {
    wait_for_pool(&pool, head).await;
    let mut pending: PendingTxs = pool.pending_transactions().iter().map(|tx| *tx.hash()).collect();
    pending.sort_unstable();
    (!pending.is_empty()).then_some(pending)
}

/// Waits until the pool has seen the block `head`, and returns whether it did in time.
pub async fn wait_for_pool<Pool: TransactionPool>(pool: &Pool, head: B256) -> bool {
    let deadline = Instant::now() + POOL_SYNC_TIMEOUT;
    while pool.block_info().last_seen_block_hash != head {
        if Instant::now() >= deadline {
            warn!(target: "reth_anvil::mining", %head, "the pool did not see the block in time");
            return false;
        }
        sleep(Duration::from_millis(1)).await;
    }
    true
}
