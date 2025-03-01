//! Block heartbeat and pending transaction watcher.

use crate::{Provider, RootProvider};
use alloy_consensus::BlockHeader;
use alloy_json_rpc::RpcError;
use alloy_network::{BlockResponse, Network};
use alloy_primitives::{
    map::{B256HashMap, B256HashSet},
    TxHash, B256,
};
use alloy_transport::{utils::Spawnable, TransportError};
use futures::{stream::StreamExt, FutureExt, Stream};
use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    future::Future,
    time::Duration,
};
use tokio::{
    select,
    sync::{mpsc, oneshot, watch},
};

#[cfg(target_arch = "wasm32")]
use wasmtimer::{std::Instant, tokio::sleep_until};

#[cfg(not(target_arch = "wasm32"))]
use {std::time::Instant, tokio::time::sleep_until};

/// Errors which may occur when watching a pending transaction.
#[derive(Debug, thiserror::Error)]
pub enum PendingTransactionError {
    /// Failed to register pending transaction in heartbeat.
    #[error("failed to register pending transaction to watch")]
    FailedToRegister,

    /// Underlying transport error.
    #[error(transparent)]
    TransportError(#[from] TransportError),

    /// Error occured while getting response from the heartbeat.
    #[error(transparent)]
    Recv(#[from] oneshot::error::RecvError),

    /// Errors that may occur when watching a transaction.
    #[error(transparent)]
    TxWatcher(#[from] WatchTxError),
}

/// A builder for configuring a pending transaction watcher.
///
/// # Examples
///
/// Send and wait for a transaction to be confirmed 2 times, with a timeout of 60 seconds:
///
/// ```no_run
/// # async fn example<N: alloy_network::Network>(provider: impl alloy_provider::Provider, tx: alloy_rpc_types_eth::transaction::TransactionRequest) -> Result<(), Box<dyn std::error::Error>> {
/// // Send a transaction, and configure the pending transaction.
/// let builder = provider.send_transaction(tx)
///     .await?
///     .with_required_confirmations(2)
///     .with_timeout(Some(std::time::Duration::from_secs(60)));
/// // Register the pending transaction with the provider.
/// let pending_tx = builder.register().await?;
/// // Wait for the transaction to be confirmed 2 times.
/// let tx_hash = pending_tx.await?;
/// # Ok(())
/// # }
/// ```
///
/// This can also be more concisely written using `watch`:
/// ```no_run
/// # async fn example<N: alloy_network::Network>(provider: impl alloy_provider::Provider, tx: alloy_rpc_types_eth::transaction::TransactionRequest) -> Result<(), Box<dyn std::error::Error>> {
/// let tx_hash = provider.send_transaction(tx)
///     .await?
///     .with_required_confirmations(2)
///     .with_timeout(Some(std::time::Duration::from_secs(60)))
///     .watch()
///     .await?;
/// # Ok(())
/// # }
/// ```
#[must_use = "this type does nothing unless you call `register`, `watch` or `get_receipt`"]
#[derive(Debug)]
#[doc(alias = "PendingTxBuilder")]
pub struct PendingTransactionBuilder<N: Network> {
    config: PendingTransactionConfig,
    provider: RootProvider<N>,
}

impl<N: Network> PendingTransactionBuilder<N> {
    /// Creates a new pending transaction builder.
    pub const fn new(provider: RootProvider<N>, tx_hash: TxHash) -> Self {
        Self::from_config(provider, PendingTransactionConfig::new(tx_hash))
    }

    /// Creates a new pending transaction builder from the given configuration.
    pub const fn from_config(provider: RootProvider<N>, config: PendingTransactionConfig) -> Self {
        Self { config, provider }
    }

    /// Returns the inner configuration.
    pub const fn inner(&self) -> &PendingTransactionConfig {
        &self.config
    }

    /// Consumes this builder, returning the inner configuration.
    pub fn into_inner(self) -> PendingTransactionConfig {
        self.config
    }

    /// Returns the provider.
    pub const fn provider(&self) -> &RootProvider<N> {
        &self.provider
    }

    /// Consumes this builder, returning the provider and the configuration.
    pub fn split(self) -> (RootProvider<N>, PendingTransactionConfig) {
        (self.provider, self.config)
    }

    /// Returns the transaction hash.
    #[doc(alias = "transaction_hash")]
    pub const fn tx_hash(&self) -> &TxHash {
        self.config.tx_hash()
    }

    /// Sets the transaction hash.
    #[doc(alias = "set_transaction_hash")]
    pub fn set_tx_hash(&mut self, tx_hash: TxHash) {
        self.config.set_tx_hash(tx_hash);
    }

    /// Sets the transaction hash.
    #[doc(alias = "with_transaction_hash")]
    pub const fn with_tx_hash(mut self, tx_hash: TxHash) -> Self {
        self.config.tx_hash = tx_hash;
        self
    }

    /// Returns the number of confirmations to wait for.
    #[doc(alias = "confirmations")]
    pub const fn required_confirmations(&self) -> u64 {
        self.config.required_confirmations()
    }

    /// Sets the number of confirmations to wait for.
    #[doc(alias = "set_confirmations")]
    pub fn set_required_confirmations(&mut self, confirmations: u64) {
        self.config.set_required_confirmations(confirmations);
    }

    /// Sets the number of confirmations to wait for.
    #[doc(alias = "with_confirmations")]
    pub const fn with_required_confirmations(mut self, confirmations: u64) -> Self {
        self.config.required_confirmations = confirmations;
        self
    }

    /// Returns the timeout.
    pub const fn timeout(&self) -> Option<Duration> {
        self.config.timeout()
    }

    /// Sets the timeout.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.config.set_timeout(timeout);
    }

    /// Sets the timeout.
    pub const fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Registers the watching configuration with the provider.
    ///
    /// This does not wait for the transaction to be confirmed, but returns a [`PendingTransaction`]
    /// that can be awaited at a later moment.
    ///
    /// See:
    /// - [`watch`](Self::watch) for watching the transaction without fetching the receipt.
    /// - [`get_receipt`](Self::get_receipt) for fetching the receipt after the transaction has been
    ///   confirmed.
    #[doc(alias = "build")]
    pub async fn register(self) -> Result<PendingTransaction, PendingTransactionError> {
        self.provider.watch_pending_transaction(self.config).await
    }

    /// Waits for the transaction to confirm with the given number of confirmations.
    ///
    /// See:
    /// - [`register`](Self::register): for registering the transaction without waiting for it to be
    ///   confirmed.
    /// - [`get_receipt`](Self::get_receipt) for fetching the receipt after the transaction has been
    ///   confirmed.
    pub async fn watch(self) -> Result<TxHash, PendingTransactionError> {
        self.register().await?.await
    }

    /// Waits for the transaction to confirm with the given number of confirmations, and
    /// then fetches its receipt.
    ///
    /// Note that this method will call `eth_getTransactionReceipt` on the [**root
    /// provider**](RootProvider), and not on a specific network provider. This means that any
    /// overrides or customizations made to the network provider will not be used.
    ///
    /// See:
    /// - [`register`](Self::register): for registering the transaction without waiting for it to be
    ///   confirmed.
    /// - [`watch`](Self::watch) for watching the transaction without fetching the receipt.
    pub async fn get_receipt(self) -> Result<N::ReceiptResponse, PendingTransactionError> {
        let hash = self.config.tx_hash;
        let mut pending_tx = self.provider.watch_pending_transaction(self.config).await?;

        // FIXME: this is a hotfix to prevent a race condition where the heartbeat would miss the
        // block the tx was mined in
        let mut interval = tokio::time::interval(self.provider.client().poll_interval());

        loop {
            let mut confirmed = false;

            select! {
                _ = interval.tick() => {},
                res = &mut pending_tx => {
                    let _ = res?;
                    confirmed = true;
                }
            }

            // try to fetch the receipt
            let receipt = self.provider.get_transaction_receipt(hash).await?;
            if let Some(receipt) = receipt {
                return Ok(receipt);
            }

            if confirmed {
                return Err(RpcError::NullResp.into());
            }
        }
    }
}

/// Configuration for watching a pending transaction.
///
/// This type can be used to create a [`PendingTransactionBuilder`], but in general it is only used
/// internally.
#[must_use = "this type does nothing unless you call `with_provider`"]
#[derive(Clone, Debug)]
#[doc(alias = "PendingTxConfig", alias = "TxPendingConfig")]
pub struct PendingTransactionConfig {
    /// The transaction hash to watch for.
    #[doc(alias = "transaction_hash")]
    tx_hash: TxHash,

    /// Require a number of confirmations.
    required_confirmations: u64,

    /// Optional timeout for the transaction.
    timeout: Option<Duration>,
}

impl PendingTransactionConfig {
    /// Create a new watch for a transaction.
    pub const fn new(tx_hash: TxHash) -> Self {
        Self { tx_hash, required_confirmations: 1, timeout: None }
    }

    /// Returns the transaction hash.
    #[doc(alias = "transaction_hash")]
    pub const fn tx_hash(&self) -> &TxHash {
        &self.tx_hash
    }

    /// Sets the transaction hash.
    #[doc(alias = "set_transaction_hash")]
    pub fn set_tx_hash(&mut self, tx_hash: TxHash) {
        self.tx_hash = tx_hash;
    }

    /// Sets the transaction hash.
    #[doc(alias = "with_transaction_hash")]
    pub const fn with_tx_hash(mut self, tx_hash: TxHash) -> Self {
        self.tx_hash = tx_hash;
        self
    }

    /// Returns the number of confirmations to wait for.
    #[doc(alias = "confirmations")]
    pub const fn required_confirmations(&self) -> u64 {
        self.required_confirmations
    }

    /// Sets the number of confirmations to wait for.
    #[doc(alias = "set_confirmations")]
    pub fn set_required_confirmations(&mut self, confirmations: u64) {
        self.required_confirmations = confirmations;
    }

    /// Sets the number of confirmations to wait for.
    #[doc(alias = "with_confirmations")]
    pub const fn with_required_confirmations(mut self, confirmations: u64) -> Self {
        self.required_confirmations = confirmations;
        self
    }

    /// Returns the timeout.
    pub const fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// Sets the timeout.
    pub fn set_timeout(&mut self, timeout: Option<Duration>) {
        self.timeout = timeout;
    }

    /// Sets the timeout.
    pub const fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Wraps this configuration with a provider to expose watching methods.
    pub const fn with_provider<N: Network>(
        self,
        provider: RootProvider<N>,
    ) -> PendingTransactionBuilder<N> {
        PendingTransactionBuilder::from_config(provider, self)
    }
}

/// Errors which may occur in heartbeat when watching a transaction.
#[derive(Debug, thiserror::Error)]
pub enum WatchTxError {
    /// Transaction was not confirmed after configured timeout.
    #[error("transaction was not confirmed within the timeout")]
    Timeout,
}

#[doc(alias = "TransactionWatcher")]
struct TxWatcher {
    config: PendingTransactionConfig,
    /// The block at which the transaction was received. To be filled once known.
    /// Invariant: any confirmed transaction in `Heart` has this value set.
    received_at_block: Option<u64>,
    tx: oneshot::Sender<Result<(), WatchTxError>>,
}

impl TxWatcher {
    /// Notify the waiter.
    fn notify(self, result: Result<(), WatchTxError>) {
        debug!(tx=%self.config.tx_hash, "notifying");
        let _ = self.tx.send(result);
    }
}

/// Represents a transaction that is yet to be confirmed a specified number of times.
///
/// This struct is a future created by [`PendingTransactionBuilder`] that resolves to the
/// transaction hash once the underlying transaction has been confirmed the specified number of
/// times in the network.
#[doc(alias = "PendingTx", alias = "TxPending")]
pub struct PendingTransaction {
    /// The transaction hash.
    #[doc(alias = "transaction_hash")]
    pub(crate) tx_hash: TxHash,
    /// The receiver for the notification.
    // TODO: send a receipt?
    pub(crate) rx: oneshot::Receiver<Result<(), WatchTxError>>,
}

impl fmt::Debug for PendingTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingTransaction").field("tx_hash", &self.tx_hash).finish()
    }
}

impl PendingTransaction {
    /// Creates a ready pending transaction.
    pub fn ready(tx_hash: TxHash) -> Self {
        let (tx, rx) = oneshot::channel();
        tx.send(Ok(())).ok(); // Make sure that the receiver is notified already.
        Self { tx_hash, rx }
    }

    /// Returns this transaction's hash.
    #[doc(alias = "transaction_hash")]
    pub const fn tx_hash(&self) -> &TxHash {
        &self.tx_hash
    }
}

impl Future for PendingTransaction {
    type Output = Result<TxHash, PendingTransactionError>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.rx.poll_unpin(cx).map(|res| {
            res??;
            Ok(self.tx_hash)
        })
    }
}

/// A handle to the heartbeat task.
#[derive(Clone, Debug)]
pub(crate) struct HeartbeatHandle<N: Network> {
    tx: mpsc::Sender<TxWatcher>,
    latest: watch::Receiver<Option<N::BlockResponse>>,
}

impl<N: Network> HeartbeatHandle<N> {
    /// Watch for a transaction to be confirmed with the given config.
    #[doc(alias = "watch_transaction")]
    pub(crate) async fn watch_tx(
        &self,
        config: PendingTransactionConfig,
        received_at_block: Option<u64>,
    ) -> Result<PendingTransaction, PendingTransactionConfig> {
        let (tx, rx) = oneshot::channel();
        let tx_hash = config.tx_hash;
        match self.tx.send(TxWatcher { config, received_at_block, tx }).await {
            Ok(()) => Ok(PendingTransaction { tx_hash, rx }),
            Err(e) => Err(e.0.config),
        }
    }

    /// Returns a watcher that always sees the latest block.
    #[allow(dead_code)]
    pub(crate) const fn latest(&self) -> &watch::Receiver<Option<N::BlockResponse>> {
        &self.latest
    }
}

// TODO: Parameterize with `Network`
/// A heartbeat task that receives blocks and watches for transactions.
pub(crate) struct Heartbeat<N, S> {
    /// The stream of incoming blocks to watch.
    stream: futures::stream::Fuse<S>,

    /// Lookbehind blocks in form of mapping block number -> vector of transaction hashes.
    past_blocks: VecDeque<(u64, B256HashSet)>,

    /// Transactions to watch for.
    unconfirmed: B256HashMap<TxWatcher>,

    /// Ordered map of transactions waiting for confirmations.
    waiting_confs: BTreeMap<u64, Vec<TxWatcher>>,

    /// Ordered map of transactions to reap at a certain time.
    reap_at: BTreeMap<Instant, B256>,

    _network: std::marker::PhantomData<N>,
}

impl<N: Network, S: Stream<Item = N::BlockResponse> + Unpin + 'static> Heartbeat<N, S> {
    /// Create a new heartbeat task.
    pub(crate) fn new(stream: S) -> Self {
        Self {
            stream: stream.fuse(),
            past_blocks: Default::default(),
            unconfirmed: Default::default(),
            waiting_confs: Default::default(),
            reap_at: Default::default(),
            _network: Default::default(),
        }
    }

    /// Check if any transactions have enough confirmations to notify.
    fn check_confirmations(&mut self, current_height: u64) {
        let to_keep = self.waiting_confs.split_off(&(current_height + 1));
        let to_notify = std::mem::replace(&mut self.waiting_confs, to_keep);
        for watcher in to_notify.into_values().flatten() {
            watcher.notify(Ok(()));
        }
    }

    /// Get the next time to reap a transaction. If no reaps, this is a very
    /// long time from now (i.e. will not be woken).
    fn next_reap(&self) -> Instant {
        self.reap_at
            .first_key_value()
            .map(|(k, _)| *k)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(60_000))
    }

    /// Reap any timeout
    fn reap_timeouts(&mut self) {
        let now = Instant::now();
        let to_keep = self.reap_at.split_off(&now);
        let to_reap = std::mem::replace(&mut self.reap_at, to_keep);

        for tx_hash in to_reap.values() {
            if let Some(watcher) = self.unconfirmed.remove(tx_hash) {
                debug!(tx=%tx_hash, "reaped");
                watcher.notify(Err(WatchTxError::Timeout));
            }
        }
    }

    /// Reap transactions overridden by the reorg.
    /// Accepts new chain height as an argument, and drops any subscriptions
    /// that were received in blocks affected by the reorg (e.g. >= new_height).
    fn move_reorg_to_unconfirmed(&mut self, new_height: u64) {
        for waiters in self.waiting_confs.values_mut() {
            *waiters = std::mem::take(waiters).into_iter().filter_map(|watcher| {
                if let Some(received_at_block) = watcher.received_at_block {
                    // All blocks after and _including_ the new height are reaped.
                    if received_at_block >= new_height {
                        let hash = watcher.config.tx_hash;
                        debug!(tx=%hash, %received_at_block, %new_height, "return to unconfirmed due to reorg");
                        self.unconfirmed.insert(hash, watcher);
                        return None;
                    }
                }
                Some(watcher)
            }).collect();
        }
    }

    /// Handle a watch instruction by adding it to the watch list, and
    /// potentially adding it to our `reap_at` list.
    fn handle_watch_ix(&mut self, to_watch: TxWatcher) {
        // Start watching for the transaction.
        debug!(tx=%to_watch.config.tx_hash, "watching");
        trace!(?to_watch.config, ?to_watch.received_at_block);
        if let Some(received_at_block) = to_watch.received_at_block {
            // Transaction is already confirmed, we just need to wait for the required
            // confirmations.
            let current_block =
                self.past_blocks.back().map(|(h, _)| *h).unwrap_or(received_at_block);
            self.add_to_waiting_list(to_watch, current_block);
            return;
        }

        if let Some(timeout) = to_watch.config.timeout {
            self.reap_at.insert(Instant::now() + timeout, to_watch.config.tx_hash);
        }
        // Transaction may be confirmed already, check the lookbehind history first.
        // If so, insert it into the waiting list.
        for (block_height, txs) in self.past_blocks.iter().rev() {
            if txs.contains(&to_watch.config.tx_hash) {
                let confirmations = to_watch.config.required_confirmations;
                let confirmed_at = *block_height + confirmations - 1;
                let current_height = self.past_blocks.back().map(|(h, _)| *h).unwrap();

                if confirmed_at <= current_height {
                    to_watch.notify(Ok(()));
                } else {
                    debug!(tx=%to_watch.config.tx_hash, %block_height, confirmations, "adding to waiting list");
                    self.waiting_confs.entry(confirmed_at).or_default().push(to_watch);
                }
                return;
            }
        }

        self.unconfirmed.insert(to_watch.config.tx_hash, to_watch);
    }

    fn add_to_waiting_list(&mut self, watcher: TxWatcher, block_height: u64) {
        let confirmations = watcher.config.required_confirmations;
        debug!(tx=%watcher.config.tx_hash, %block_height, confirmations, "adding to waiting list");
        self.waiting_confs.entry(block_height + confirmations - 1).or_default().push(watcher);
    }

    /// Handle a new block by checking if any of the transactions we're
    /// watching are in it, and if so, notifying the watcher. Also updates
    /// the latest block.
    fn handle_new_block(
        &mut self,
        block: N::BlockResponse,
        latest: &watch::Sender<Option<N::BlockResponse>>,
    ) {
        // Blocks without numbers are ignored, as they're not part of the chain.
        let block_height = block.header().as_ref().number();

        // Add the block the lookbehind.
        // The value is chosen arbitrarily to not have a huge memory footprint but still
        // catch most cases where user subscribes for an already mined transaction.
        // Note that we expect provider to check whether transaction is already mined
        // before subscribing, so here we only need to consider time before sending a notification
        // and processing it.
        const MAX_BLOCKS_TO_RETAIN: usize = 10;
        if self.past_blocks.len() >= MAX_BLOCKS_TO_RETAIN {
            self.past_blocks.pop_front();
        }
        if let Some((last_height, _)) = self.past_blocks.back().as_ref() {
            // Check that the chain is continuous.
            if *last_height + 1 != block_height {
                // Move all the transactions that were reset by the reorg to the unconfirmed list.
                warn!(%block_height, last_height, "reorg detected");
                self.move_reorg_to_unconfirmed(block_height);
                // Remove past blocks that are now invalid.
                self.past_blocks.retain(|(h, _)| *h < block_height);
            }
        }
        self.past_blocks.push_back((block_height, block.transactions().hashes().collect()));

        // Check if we are watching for any of the transactions in this block.
        let to_check: Vec<_> = block
            .transactions()
            .hashes()
            .filter_map(|tx_hash| self.unconfirmed.remove(&tx_hash))
            .collect();
        for mut watcher in to_check {
            // If `confirmations` is not more than 1 we can notify the watcher immediately.
            let confirmations = watcher.config.required_confirmations;
            if confirmations <= 1 {
                watcher.notify(Ok(()));
                continue;
            }
            // Otherwise add it to the waiting list.

            // Set the block at which the transaction was received.
            if let Some(set_block) = watcher.received_at_block {
                warn!(tx=%watcher.config.tx_hash, set_block=%set_block, new_block=%block_height, "received_at_block already set");
                // We don't override the set value.
            } else {
                watcher.received_at_block = Some(block_height);
            }
            self.add_to_waiting_list(watcher, block_height);
        }

        self.check_confirmations(block_height);

        // Update the latest block. We use `send_replace` here to ensure the
        // latest block is always up to date, even if no receivers exist.
        // C.f. https://docs.rs/tokio/latest/tokio/sync/watch/struct.Sender.html#method.send
        debug!(%block_height, "updating latest block");
        let _ = latest.send_replace(Some(block));
    }
}

#[cfg(target_arch = "wasm32")]
impl<N: Network, S: Stream<Item = N::BlockResponse> + Unpin + 'static> Heartbeat<N, S> {
    /// Spawn the heartbeat task, returning a [`HeartbeatHandle`].
    pub(crate) fn spawn(self) -> HeartbeatHandle<N> {
        let (task, handle) = self.consume();
        task.spawn_task();
        handle
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<N: Network, S: Stream<Item = N::BlockResponse> + Unpin + Send + 'static> Heartbeat<N, S> {
    /// Spawn the heartbeat task, returning a [`HeartbeatHandle`].
    pub(crate) fn spawn(self) -> HeartbeatHandle<N> {
        let (task, handle) = self.consume();
        task.spawn_task();
        handle
    }
}

impl<N: Network, S: Stream<Item = N::BlockResponse> + Unpin + 'static> Heartbeat<N, S> {
    fn consume(self) -> (impl Future<Output = ()>, HeartbeatHandle<N>) {
        let (latest, latest_rx) = watch::channel(None::<N::BlockResponse>);
        let (ix_tx, ixns) = mpsc::channel(16);
        (self.into_future(latest, ixns), HeartbeatHandle { tx: ix_tx, latest: latest_rx })
    }

    async fn into_future(
        mut self,
        latest: watch::Sender<Option<N::BlockResponse>>,
        mut ixns: mpsc::Receiver<TxWatcher>,
    ) {
        'shutdown: loop {
            {
                let next_reap = self.next_reap();
                let sleep = std::pin::pin!(sleep_until(next_reap.into()));

                // We bias the select so that we always handle new messages
                // before checking blocks, and reap timeouts are last.
                select! {
                    biased;

                    // Watch for new transactions.
                    ix_opt = ixns.recv() => match ix_opt {
                        Some(to_watch) => self.handle_watch_ix(to_watch),
                        None => break 'shutdown, // ix channel is closed
                    },

                    // Wake up to handle new blocks.
                    Some(block) = self.stream.next() => {
                        self.handle_new_block(block, &latest);
                    },

                    // This arm ensures we always wake up to reap timeouts,
                    // even if there are no other events.
                    _ = sleep => {},
                }
            }

            // Always reap timeouts
            self.reap_timeouts();
        }
    }
}
