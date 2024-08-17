//! Support for polling based filters
use crate::{
    eth::{backend::notifications::NewBlockNotifications, error::ToRpcResponseResult},
    pubsub::filter_logs,
    StorageInfo,
};
use alloy_primitives::TxHash;
use alloy_rpc_types::{Filter, FilteredParams, Log};
use anvil_core::eth::subscription::SubscriptionId;
use anvil_rpc::response::ResponseResult;
use futures::{channel::mpsc::Receiver, Stream, StreamExt};
use std::{
    collections::HashMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

/// Type alias for filters identified by their id and their expiration timestamp
type FilterMap = Arc<Mutex<HashMap<String, (EthFilter, Instant)>>>;

/// timeout after which to remove an active filter if it wasn't polled since then
pub const ACTIVE_FILTER_TIMEOUT_SECS: u64 = 60 * 5;

/// Contains all registered filters
#[derive(Clone, Debug)]
pub struct Filters {
    /// all currently active filters
    active_filters: FilterMap,
    /// How long we keep a live the filter after the last poll
    keepalive: Duration,
}

impl Filters {
    /// Adds a new `EthFilter` to the set
    pub async fn add_filter(&self, filter: EthFilter) -> String {
        let id = new_id();
        trace!(target: "node::filter", "Adding new filter id {}", id);
        let mut filters = self.active_filters.lock().await;
        filters.insert(id.clone(), (filter, self.next_deadline()));
        id
    }

    pub async fn get_filter_changes(&self, id: &str) -> ResponseResult {
        {
            let mut filters = self.active_filters.lock().await;
            if let Some((filter, deadline)) = filters.get_mut(id) {
                let resp = filter
                    .next()
                    .await
                    .unwrap_or_else(|| ResponseResult::success(Vec::<()>::new()));
                *deadline = self.next_deadline();
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

    /// Returns the timestamp after which a filter should expire
    fn next_deadline(&self) -> Instant {
        Instant::now() + self.keep_alive()
    }

    /// Evict all filters that weren't updated and reached there deadline
    pub async fn evict(&self) {
        trace!(target: "node::filter", "Evicting stale filters");
        let now = Instant::now();
        let mut active_filters = self.active_filters.lock().await;
        active_filters.retain(|id, (_, deadline)| {
            if now > *deadline {
                trace!(target: "node::filter",?id, "Evicting stale filter");
                return false
            }
            true
        });
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

impl Stream for EthFilter {
    type Item = ResponseResult;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();
        match pin {
            Self::Logs(logs) => Poll::Ready(Some(Ok(logs.poll(cx)).to_rpc_result())),
            Self::Blocks(blocks) => {
                let mut new_blocks = Vec::new();
                while let Poll::Ready(Some(block)) = blocks.poll_next_unpin(cx) {
                    new_blocks.push(block.hash);
                }
                Poll::Ready(Some(Ok(new_blocks).to_rpc_result()))
            }
            Self::PendingTransactions(tx) => {
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
    pub historic: Option<Vec<Log>>,
}

impl LogsFilter {
    /// Returns all the logs since the last time this filter was polled
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Vec<Log> {
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
