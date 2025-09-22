use crate::substrate_node::service::TransactionPoolHandle;
use alloy_rpc_types::anvil::MineOptions;
use anvil::eth::backend::time::TimeManager;
use futures::{
    StreamExt,
    channel::oneshot,
    stream::{FusedStream, SelectAll, select_all, unfold},
    task::AtomicWaker,
};
use parking_lot::RwLock;
use polkadot_sdk::{
    sc_consensus_manual_seal::{CreatedBlock, EngineCommand, Error as BlockProducingError},
    sc_service::TransactionPool,
    sp_core,
};
use std::{pin::Pin, sync::Arc};
use substrate_runtime::Hash;
use tokio::{
    sync::mpsc::Sender,
    time::{Duration, Instant, MissedTickBehavior, interval_at},
};

// Errors that can happen during the block production.
#[derive(Debug, thiserror::Error)]
pub enum MiningError {
    #[error("Block production failed: {0}")]
    BlockProducing(#[from] BlockProducingError),
    #[error("Current mining mode can not answer this query.")]
    MiningModeMismatch,
    #[error("Current timestamp is newer.")]
    Timestamp,
    #[error("Closed channel")]
    ClosedChannel,
}

/// Mining modes supported by the MiningEngine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiningMode {
    /// Blocks are produced only in response to RPC calls.
    None,
    /// Automatic block productiona at fixed time intervals.
    Interval { tick: Duration },
    /// Automatic block production triggered by new transactions.
    AutoMining,
    /// Hybrid mode combining interval and transaction-based mining.
    MixedMining { tick: Duration },
}

impl MiningMode {
    /// Create a mining mode based on configuration parameters.
    ///
    /// This method determines the appropriate mining mode based on
    /// the provided timing and behavioral preferences.
    ///
    /// # Arguments
    /// * `block_time` - Optional duration between blocks for interval mining
    /// * `mixed_mining` - Enable mixed mode when block_time is provided
    /// * `no_mining` - Disable automatic mining when no block_time is set
    ///
    /// # Returns
    /// The appropriate `MiningMode` variant based on input parameters
    pub fn new(block_time: Option<Duration>, mixed_mining: bool, no_mining: bool) -> Self {
        block_time.map_or_else(
            || if no_mining { Self::None } else { Self::AutoMining },
            |time| {
                if mixed_mining {
                    Self::MixedMining { tick: time }
                } else {
                    Self::Interval { tick: time }
                }
            },
        )
    }
}

/// Controller for blockchain block production operations.
///
/// The `MiningEngine` provides a high-level interface for managing block production
/// It supports multiple mining modes, time manipulation for testing, and
/// Ethereum-compatible RPC methods.
///
/// The engine coordinates between the transaction pool, consensus layer, and time
/// management to provide flexible block production strategies suitable for both
/// development and production environments.
pub struct MiningEngine {
    /// Coordination mechanism between the MiningEngine and the background
    /// task that runs the "polling" loop from `run_mining_engine`.
    /// Calls to waker.wake() will unpark the background task and force a
    /// recheck of the current mining mode and rebuild of the polled streams.
    waker: Arc<AtomicWaker>,
    mining_mode: Arc<RwLock<MiningMode>>,
    transaction_pool: Arc<TransactionPoolHandle>,
    time_manager: Arc<TimeManager>,
    seal_command_sender: Sender<EngineCommand<sp_core::H256>>,
}

impl MiningEngine {
    /// Create a new mining engine controller.
    ///
    /// Initializes a mining engine with the specified components and configuration.
    /// The engine will coordinate block production according to the initial mining mode.
    ///
    /// # Arguments
    /// * `mining_mode` - Initial mining strategy
    /// * `transaction_pool` - Handle for monitoring transaction pool changes
    /// * `time_manager` - Component for blockchain time management
    /// * `seal_command_sender` - Channel for sending block sealing commands
    ///
    /// # Returns
    /// A new `MiningEngine` instance ready for use
    pub fn new(
        mining_mode: MiningMode,
        transaction_pool: Arc<TransactionPoolHandle>,
        time_manager: Arc<TimeManager>,
        seal_command_sender: Sender<EngineCommand<sp_core::H256>>,
    ) -> Self {
        Self {
            waker: Default::default(),
            mining_mode: Arc::new(RwLock::new(mining_mode)),
            transaction_pool,
            time_manager,
            seal_command_sender,
        }
    }

    /// Mine a specified number of blocks manually.
    ///
    /// This method implements the `anvil_mine` RPC call, allowing manual control
    /// over block production. Blocks are mined sequentially, with optional time
    /// advancement between each block.
    ///
    /// # Arguments
    /// * `num_blocks` - Number of blocks to mine (defaults to 1 if None)
    /// * `interval` - Optional time to advance between blocks (in seconds)
    ///
    /// # Returns
    /// * `Ok(())` - All blocks were mined successfully
    /// * `Err(MiningError)` - Block production failed
    pub async fn mine(
        &self,
        num_blocks: Option<u64>,
        interval: Option<Duration>,
    ) -> Result<(), MiningError> {
        let blocks = num_blocks.unwrap_or(1);
        for _ in 0..blocks {
            if let Some(interval) = interval {
                self.time_manager.increase_time(interval.as_secs());
            }
            seal_now(&self.seal_command_sender).await?;
        }
        Ok(())
    }

    /// Ethereum-compatible block mining RPC method.
    ///
    /// Implements the `evm_mine` RPC call from the Ethereum JSON-RPC API.
    /// This method provides compatibility with Ethereum development tools
    /// and testing frameworks.
    ///
    /// # Arguments
    /// * `opts` - Optional mining parameters including timestamp and block count
    ///
    /// # Returns
    /// * `Ok(())` - Success response
    /// * `Err(MiningError)` - Mining operation failed
    pub async fn evm_mine(&self, opts: Option<MineOptions>) -> Result<(), MiningError> {
        self.do_evm_mine(opts).await.map(|_| ())
    }

    /// Configure interval-based mining mode.
    ///
    /// Sets the mining engine to produce blocks at regular time intervals.
    /// An interval of 0 disables interval mining. Changes take effect immediately
    /// and will wake the background mining task.
    ///
    /// # Arguments
    /// * `interval` - Block production interval in seconds (0 to disable)
    pub fn set_interval_mining(&self, interval: Duration) {
        let new_mode = if interval.as_secs() == 0 {
            MiningMode::None
        } else {
            MiningMode::Interval { tick: interval }
        };
        *self.mining_mode.write() = new_mode;
        self.wake();
    }

    /// Get the current interval mining configuration.
    ///
    /// Returns the current block production interval if interval or mixed
    /// mining is active, or None if interval mining is disabled.
    ///
    /// # Returns
    /// * `Some(seconds)` - Interval mining active with specified interval
    /// * `None` - Interval mining is disabled
    pub fn get_interval_mining(&self) -> Option<u64> {
        let mode = *self.mining_mode.read();
        match mode {
            MiningMode::Interval { tick } | MiningMode::MixedMining { tick } => {
                Some(tick.as_secs())
            }
            _ => None,
        }
    }

    /// Check if automatic mining is enabled.
    ///
    /// Returns true if the mining engine will automatically produce blocks
    /// when new transactions are added to the transaction pool.
    pub fn is_automine(&self) -> bool {
        matches!(*self.mining_mode.read(), MiningMode::AutoMining)
    }

    /// Enable or disable automatic mining mode.
    ///
    /// When enabled, the mining engine will automatically produce a new block
    /// whenever a transaction is added to the transaction pool. This provides
    /// instant transaction confirmation in development environments.
    ///
    /// # Arguments
    /// * `enabled` - True to enable auto-mining, false to disable
    pub fn set_auto_mine(&self, enabled: bool) {
        let mining_mode = match (self.is_automine(), enabled) {
            (true, false) => Some(MiningMode::None),
            (false, true) => Some(MiningMode::AutoMining),
            _ => None,
        };
        if let Some(mining_mode) = mining_mode {
            *self.mining_mode.write() = mining_mode;
            self.wake();
        }
    }

    /// Set the timestamp for the next block to be mined.
    ///
    /// Allows precise control over block timestamps for testing scenarios.
    /// The timestamp must not be older than the current blockchain time.
    ///
    /// # Arguments
    /// * `time_in_seconds` - Unix timestamp in seconds for the next block
    ///
    /// # Returns
    /// * `Ok(())` - Timestamp set successfully
    /// * `Err(MiningError::Timestamp)` - Invalid timestamp
    pub fn set_next_block_timestamp(&self, time: Duration) -> Result<(), MiningError> {
        self.time_manager
            // this will convert the time_in_seconds in milliseconds. It is transparent
            // to the user
            .set_next_block_timestamp(time.as_secs())
            .map_err(|_| MiningError::Timestamp)
    }

    /// Advance the blockchain time by a specified duration.
    ///
    /// Increases the current blockchain time, affecting timestamps of future blocks.
    ///
    /// # Arguments
    /// * `time_in_seconds` - Duration to advance in seconds
    ///
    /// # Returns
    /// * `new_timestamp` - The new current timestamp as i64
    pub fn increase_time(&self, time: Duration) -> i64 {
        self.time_manager.increase_time(time.as_secs()).saturating_div(1000) as i64
    }

    /// Set the blockchain time to a specific timestamp.
    ///
    /// Resets the blockchain time to the specified timestamp and returns
    /// the time difference from the previous timestamp.
    ///
    /// # Arguments
    /// * `timestamp` - Target timestamp in seconds since Unix epoch
    ///
    /// # Returns
    /// * `offset_seconds` - Time difference from previous timestamp
    pub fn set_time(&self, timestamp: Duration) -> u64 {
        let now = self.time_manager.current_call_timestamp();
        self.time_manager.reset(timestamp.as_secs());
        let offset = (timestamp.as_millis() as u64).saturating_sub(now);
        Duration::from_millis(offset).as_secs()
    }

    /// Configure automatic timestamp intervals between blocks.
    ///
    /// Sets a fixed time interval that will be automatically added to each
    /// block's timestamp relative to the previous block. This ensures
    /// consistent time progression in the blockchain.
    ///
    /// # Arguments
    /// * `interval_in_seconds` - Time interval to add between block timestamps
    pub fn set_block_timestamp_interval(&self, interval_in_seconds: Duration) {
        self.time_manager.set_block_timestamp_interval(interval_in_seconds.as_secs())
    }

    /// Remove automatic timestamp intervals between blocks.
    ///
    /// Disables the automatic timestamp interval feature, allowing blocks
    /// to have timestamps based on the actual mining time rather than
    /// fixed intervals.
    ///
    /// # Returns
    /// * `true` - Timestamp interval was removed
    /// * `false` - No timestamp interval was configured
    pub fn remove_block_timestamp_interval(&self) -> bool {
        self.time_manager.remove_block_timestamp_interval()
    }

    //---------- Helpers ---------------

    fn wake(&self) {
        self.waker.wake();
    }

    async fn do_evm_mine(&self, opts: Option<MineOptions>) -> Result<u64, MiningError> {
        let mut blocks_to_mine = 1u64;

        if let Some(opts) = opts {
            let timestamp = match opts {
                MineOptions::Timestamp(timestamp) => timestamp,
                MineOptions::Options { timestamp, blocks } => {
                    if let Some(blocks) = blocks {
                        blocks_to_mine = blocks;
                    }
                    timestamp
                }
            };
            if let Some(timestamp) = timestamp {
                // timestamp was explicitly provided to be the next timestamp
                self.time_manager
                    .set_next_block_timestamp(timestamp)
                    .map_err(|_| MiningError::Timestamp)?;
            }
        }

        for _ in 0..blocks_to_mine {
            seal_now(&self.seal_command_sender).await?;
        }

        Ok(blocks_to_mine)
    }
}

async fn seal_now(
    seal_command_sender: &Sender<EngineCommand<sp_core::H256>>,
) -> Result<CreatedBlock<Hash>, MiningError> {
    let (sender, receiver) = oneshot::channel();
    let seal_command = EngineCommand::SealNewBlock {
        create_empty: true,
        finalize: true,
        parent_hash: None,
        sender: Some(sender),
    };
    seal_command_sender.send(seal_command).await.map_err(|_| MiningError::ClosedChannel)?;
    match receiver.await {
        Ok(Ok(rx)) => Ok(rx),
        Ok(Err(e)) => Err(MiningError::BlockProducing(e)),
        Err(_e) => Err(MiningError::ClosedChannel),
    }
}

// --------------- MiningEngine runner
type SealCommandStream = Pin<Box<dyn FusedStream<Item = ()> + Send>>;

fn build_interval_stream(interval: Duration) -> SealCommandStream {
    let mut interval_ticker = interval_at(Instant::now() + interval, interval);
    interval_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let stream = unfold(interval_ticker, |mut interval_tick| async {
        interval_tick.tick().await;
        Some(((), interval_tick))
    });
    Box::pin(stream.fuse())
}

fn build_auto_stream(engine: &Arc<MiningEngine>) -> SealCommandStream {
    let stream = engine.transaction_pool.import_notification_stream().map(|_| ());
    Box::pin(stream.fuse())
}

fn build_streams_for_mode(
    mode: MiningMode,
    engine: &Arc<MiningEngine>,
) -> SelectAll<SealCommandStream> {
    let mut streams: Vec<SealCommandStream> = Vec::new();
    if let Some(stream) = match mode {
        MiningMode::Interval { tick } | MiningMode::MixedMining { tick } => Some(tick),
        _ => None,
    }
    .map(build_interval_stream)
    {
        streams.push(stream)
    }
    if let Some(stream) = matches!(mode, MiningMode::AutoMining | MiningMode::MixedMining { .. })
        .then(|| build_auto_stream(engine))
    {
        streams.push(stream)
    }
    select_all(streams)
}

async fn wait_for_mode_change(
    engine: &Arc<MiningEngine>,
    current: Option<MiningMode>,
) -> MiningMode {
    futures::future::poll_fn(|cx| {
        let mode = *engine.mining_mode.read();
        if current.as_ref().is_none_or(|m| *m != mode) {
            return std::task::Poll::Ready(mode);
        }
        engine.waker.register(cx.waker());
        std::task::Poll::Pending
    })
    .await
}

/// Run the mining engine background task.
///
/// This is the main event loop that handles block production based on the current
/// mining mode. It monitors for mining mode changes, manages stream selectors for
/// different trigger types (intervals, transactions), and coordinates block sealing
/// operations.
///
/// The function runs indefinitely until the mining engine is shut down or a fatal
/// error occurs.
///
/// # Arguments
/// * `engine` - Shared reference to the mining engine to control
///
/// # Behavior
/// - Monitors for mining mode changes and rebuilds event streams accordingly
/// - Handles interval-based mining triggers using tokio timers
/// - Handles transaction-based mining triggers from the transaction pool
/// - Processes block sealing commands and logs results
/// - Gracefully handles non-fatal errors and continues operation
/// - Terminates on fatal errors (communication failures)
///
/// # Error Handling
/// - **Fatal errors** (Canceled, SendError): Breaks the main loop and terminates
/// - **Non-fatal errors**: Logged and operation continues
/// - **Successful operations**: Block hash is logged at debug level
pub async fn run_mining_engine(engine: Arc<MiningEngine>) {
    let mut current_mode = None;
    let mut combined_stream: SelectAll<SealCommandStream> = select_all(vec![]);

    loop {
        tokio::select! {
            new_mode = wait_for_mode_change(&engine, current_mode) => {
                current_mode = Some(new_mode);
                combined_stream = build_streams_for_mode(new_mode, &engine);
            }
            Some(_) = combined_stream.next(), if !combined_stream.is_empty() => {
                match seal_now(&engine.seal_command_sender).await {
                    Ok(block) => {
                        debug!(hash=?block.hash, "sealed");
                    }
                    Err(MiningError::ClosedChannel) => {
                        break; // fatal: break outer loop
                    }
                    Err(e) => {
                        error!(?e, "block production failed");
                    }
                }
            }
        }
    }
}
