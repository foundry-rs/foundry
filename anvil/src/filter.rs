//! Support for polling based filters
use crate::{
    eth::{backend::notifications::NewBlockNotifications, error::ToRpcResponseResult},
    pubsub::filter_logs,
    StorageInfo,
};
use anvil_core::eth::{filter::FilteredParams, subscription::SubscriptionId};
use anvil_rpc::response::ResponseResult;
use ethers::prelude::{Log as EthersLog, H256 as TxHash};
use futures::{channel::mpsc::Receiver, Stream, StreamExt};

use anvil_core::eth::filter::Filter;
use std::{
    collections::HashMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tracing::{trace, warn};

type FilterMap = Arc<Mutex<HashMap<String, (EthFilter, Instant)>>>;

/// timeout after which to remove an active filter if it wasn't polled since then
pub const ACTIVE_FILTER_TIMEOUT_SECS: u64 = 60 * 5;

/// Contains all registered filters
#[derive(Debug, Clone)]
pub struct Filters {
    /// all currently active filters
    active_filters: FilterMap,
    /// How long we keep a live the filter after the last poll
    keepalive: Duration,
}

// === impl Filters ===

impl Filters {
    /// Adds a new `EthFilter` to the set
    pub async fn add_filter(&self, filter: EthFilter) -> String {
        let id = new_id();
        trace!(target: "node::filter", "Adding new filter id {}", id);
        let mut filters = self.active_filters.lock().await;
        filters.insert(id.clone(), (filter, Instant::now()));
        id
    }

    pub async fn get_filter_changes(&self, id: &str) -> ResponseResult {
        {
            let mut filters = self.active_filters.lock().await;
            if let Some((filter, timestamp)) = filters.get_mut(id) {
                let resp = filter
                    .next()
                    .await
                    .unwrap_or_else(|| ResponseResult::success(Vec::<()>::new()));
                *timestamp = Instant::now();
                return resp
            }
        }
        warn!(target: "node::filter", "No filter found for {}", id);
        ResponseResult::success(Vec::<()>::new())
    }

    /// Returns the original `Filter` of an `eth_newFilter`
    pub async fn get_log_filter(&self, id: &str) -> Option<Filter> {
        let filters = self.active_filters.lock().await;
        if let Some((EthFilter::Logs(ref log), _)) = filters.get(id) {
            return log.filter.filter.clone()
        }
        None
    }

    /// Removes the filter identified with the `id`
    pub async fn uninstall_filter(&self, id: &str) -> Option<EthFilter> {
        trace!(target: "node::filter", "Uninstalling filter id {}", id);
        self.active_filters.lock().await.remove(id).map(|(f, _)| f)
    }

    /// The duration how long to keep alive stale filters
    pub fn keep_alive(&self) -> Duration {
        self.keepalive
    }

    pub async fn evict(&self) {
        trace!(target: "node::filter", "Evicting stale filters");
        let deadline = Instant::now() - self.keepalive;
        let mut active_filters = self.active_filters.lock().await;
        active_filters.retain(|_, (_, timestamp)| *timestamp > deadline);
    }
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            active_filters: Arc::new(Default::default()),
            keepalive: Duration::from_secs(ACTIVE_FILTER_TIMEOUT_SECS),
        }
    }
}

/// returns a new random hex id
fn new_id() -> String {
    SubscriptionId::random_hex().to_string()
}

/// Represents a poll based filter
#[derive(Debug)]
pub enum EthFilter {
    Logs(Box<LogsFilter>),
    Blocks(NewBlockNotifications),
    PendingTransactions(Receiver<TxHash>),
}

// === impl EthFilter ===

impl Stream for EthFilter {
    type Item = ResponseResult;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();
        match pin {
            EthFilter::Logs(logs) => Poll::Ready(Some(Ok(logs.poll(cx)).to_rpc_result())),
            EthFilter::Blocks(blocks) => {
                let mut new_blocks = Vec::new();
                while let Poll::Ready(Some(block)) = blocks.poll_next_unpin(cx) {
                    new_blocks.push(block.hash);
                }
                Poll::Ready(Some(Ok(new_blocks).to_rpc_result()))
            }
            EthFilter::PendingTransactions(tx) => {
                let mut new_txs = Vec::new();
                while let Poll::Ready(Some(tx_hash)) = tx.poll_next_unpin(cx) {
                    new_txs.push(tx_hash);
                }
                Poll::Ready(Some(Ok(new_txs).to_rpc_result()))
            }
        }
    }
}

/// Listens for new blocks and matching logs emitted in that block
#[derive(Debug)]
pub struct LogsFilter {
    /// listener for new blocks
    pub blocks: NewBlockNotifications,
    /// accessor for block storage
    pub storage: StorageInfo,
    /// matcher with all provided filter params
    pub filter: FilteredParams,
    /// existing logs that matched the filter when the listener was installed
    ///
    /// They'll be returned on the first pill
    pub historic: Option<Vec<EthersLog>>,
}

// === impl LogsFilter ===

impl LogsFilter {
    /// Returns all the logs since the last time this filter was polled
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Vec<EthersLog> {
        let mut logs = self.historic.take().unwrap_or_default();
        while let Poll::Ready(Some(block)) = self.blocks.poll_next_unpin(cx) {
            let b = self.storage.block(block.hash);
            let receipts = self.storage.receipts(block.hash);
            if let (Some(receipts), Some(block)) = (receipts, b) {
                logs.extend(filter_logs(block, receipts, &self.filter))
            }
        }
        logs
    }
}
