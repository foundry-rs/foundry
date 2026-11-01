use crate::{
    StorageInfo,
    eth::{
        backend::notifications::{ChainNotification, ChainNotifications},
        error::to_rpc_result,
        tempo_consensus::TempoConsensusEvent,
    },
};
use alloy_consensus::{BlockHeader, TxReceipt};
use alloy_network::{AnyRpcTransaction, Network};
use alloy_primitives::{B256, TxHash};
use alloy_rpc_types::{FilteredParams, Log, Transaction, pubsub::SubscriptionResult};
use anvil_core::eth::{block::Block, subscription::SubscriptionId};
use anvil_rpc::{request::Version, response::ResponseResult};
use foundry_primitives::FoundryTxReceipt;
use futures::{Stream, StreamExt, channel::mpsc::Receiver, ready};
use serde::Serialize;
use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::{broadcast, mpsc::UnboundedReceiver};

/// Listens for new blocks and matching logs emitted in that block
pub struct LogsSubscription<N: Network> {
    pub blocks: ChainNotifications,
    pub storage: StorageInfo<N>,
    pub filter: FilteredParams,
    pub queued: VecDeque<Log>,
    pub id: SubscriptionId,
}

impl<N: Network> std::fmt::Debug for LogsSubscription<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogsSubscription")
            .field("filter", &self.filter)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl<N: Network> LogsSubscription<N>
where
    N::ReceiptEnvelope: TxReceipt<Log = alloy_primitives::Log>,
{
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Option<EthSubscriptionResponse>> {
        loop {
            if let Some(log) = self.queued.pop_front() {
                let params = EthSubscriptionParams {
                    subscription: self.id.clone(),
                    result: to_rpc_result(log),
                };
                return Poll::Ready(Some(EthSubscriptionResponse::new(params)));
            }

            if let Some(notification) = ready!(self.blocks.poll_next_unpin(cx)) {
                let logs = match notification {
                    ChainNotification::Block(block) => {
                        // Rule out nonmatching blocks before requiring data that may be pruned.
                        if !self.filter.filter_block_range(block.header.number)
                            || !self.filter.filter_block_hash(block.hash)
                            || block.header.logs_bloom.is_zero()
                            || self.filter.filter.as_ref().is_some_and(|filter| {
                                !filter.matches_bloom(block.header.logs_bloom)
                            })
                        {
                            continue;
                        }
                        let hash = block.hash;
                        let (Some(block), Some(receipts)) =
                            (self.storage.block(hash), self.storage.receipts(hash))
                        else {
                            tracing::warn!(target: "rpc::ws", ?hash, "Closing logs subscription because block data is unavailable");
                            return Poll::Ready(None);
                        };
                        if block.body.transactions.len() != receipts.len()
                            || (block.body.transactions.is_empty()
                                && block.header.transactions_root()
                                    != alloy_consensus::constants::EMPTY_ROOT_HASH)
                        {
                            tracing::warn!(target: "rpc::ws", ?hash, "Closing logs subscription because block data was pruned");
                            return Poll::Ready(None);
                        }
                        filter_logs(block, receipts, &self.filter)
                    }
                    ChainNotification::RemovedLogs(logs) => {
                        filter_removed_logs(&logs, &self.filter)
                    }
                };
                if logs.is_empty() {
                    // this ensures we poll the receiver until it is pending, in which case the
                    // underlying `UnboundedReceiver` will register the new waker, see
                    // [`futures::channel::mpsc::UnboundedReceiver::poll_next()`]
                    continue;
                }
                self.queued.extend(logs)
            } else {
                return Poll::Ready(None);
            }

            if self.queued.is_empty() {
                return Poll::Pending;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EthSubscriptionResponse {
    jsonrpc: Version,
    method: &'static str,
    params: EthSubscriptionParams,
}

impl EthSubscriptionResponse {
    pub const fn new(params: EthSubscriptionParams) -> Self {
        Self { jsonrpc: Version::V2, method: "eth_subscription", params }
    }
}

/// Represents the `params` field of an `eth_subscription` event
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EthSubscriptionParams {
    subscription: SubscriptionId,
    #[serde(flatten)]
    result: ResponseResult,
}

/// Represents an ethereum Websocket subscription
pub enum EthSubscription<N: Network> {
    Logs(Box<LogsSubscription<N>>),
    Header(ChainNotifications, StorageInfo<N>, SubscriptionId),
    PendingTransactions(Receiver<TxHash>, SubscriptionId),
    FullPendingTransactions(UnboundedReceiver<AnyRpcTransaction>, SubscriptionId),
    Syncing(Option<SubscriptionId>),
    TransactionReceipts(UnboundedReceiver<Vec<FoundryTxReceipt>>, SubscriptionId),
    TempoConsensus(
        Pin<Box<dyn Stream<Item = Arc<TempoConsensusEvent>> + Send + Sync>>,
        SubscriptionId,
    ),
}

impl<N: Network> std::fmt::Debug for EthSubscription<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Logs(_) => f.debug_tuple("Logs").finish(),
            Self::Header(..) => f.debug_tuple("Header").finish(),
            Self::PendingTransactions(..) => f.debug_tuple("PendingTransactions").finish(),
            Self::FullPendingTransactions(..) => f.debug_tuple("FullPendingTransactions").finish(),
            Self::Syncing(_) => f.debug_tuple("Syncing").finish(),
            Self::TransactionReceipts(..) => f.debug_tuple("TransactionReceipts").finish(),
            Self::TempoConsensus(..) => f.debug_tuple("TempoConsensus").finish(),
        }
    }
}

impl<N: Network> EthSubscription<N>
where
    N::ReceiptEnvelope: TxReceipt<Log = alloy_primitives::Log>,
{
    fn poll_response(&mut self, cx: &mut Context<'_>) -> Poll<Option<EthSubscriptionResponse>> {
        match self {
            Self::Logs(listener) => listener.poll(cx),
            Self::Header(blocks, storage, id) => {
                // this loop ensures we poll the receiver until it is pending, in which case the
                // underlying `UnboundedReceiver` will register the new waker, see
                // [`futures::channel::mpsc::UnboundedReceiver::poll_next()`]
                loop {
                    if let Some(notification) = ready!(blocks.poll_next_unpin(cx)) {
                        if let Some(block) = notification.as_new_block()
                            && let Some(block) = storage.eth_block(block.hash)
                        {
                            let params = EthSubscriptionParams {
                                subscription: id.clone(),
                                result: to_rpc_result(block),
                            };
                            return Poll::Ready(Some(EthSubscriptionResponse::new(params)));
                        }
                    } else {
                        return Poll::Ready(None);
                    }
                }
            }
            Self::PendingTransactions(tx, id) => {
                let res = ready!(tx.poll_next_unpin(cx))
                    .map(SubscriptionResult::<Transaction>::TransactionHash)
                    .map(to_rpc_result)
                    .map(|result| {
                        let params = EthSubscriptionParams { subscription: id.clone(), result };
                        EthSubscriptionResponse::new(params)
                    });
                Poll::Ready(res)
            }
            Self::FullPendingTransactions(tx, id) => {
                let res = ready!(tx.poll_recv(cx)).map(to_rpc_result).map(|result| {
                    let params = EthSubscriptionParams { subscription: id.clone(), result };
                    EthSubscriptionResponse::new(params)
                });
                Poll::Ready(res)
            }
            Self::Syncing(id) => {
                if let Some(id) = id.take() {
                    let params =
                        EthSubscriptionParams { subscription: id, result: to_rpc_result(false) };
                    return Poll::Ready(Some(EthSubscriptionResponse::new(params)));
                }
                Poll::Pending
            }
            Self::TransactionReceipts(receipts, id) => {
                let res = ready!(receipts.poll_recv(cx)).map(to_rpc_result).map(|result| {
                    let params = EthSubscriptionParams { subscription: id.clone(), result };
                    EthSubscriptionResponse::new(params)
                });
                Poll::Ready(res)
            }
            Self::TempoConsensus(events, id) => {
                let res = ready!(events.poll_next_unpin(cx)).map(|event| {
                    let params = EthSubscriptionParams {
                        subscription: id.clone(),
                        result: to_rpc_result(&*event),
                    };
                    EthSubscriptionResponse {
                        jsonrpc: Version::V2,
                        method: "consensus_event",
                        params,
                    }
                });
                Poll::Ready(res)
            }
        }
    }

    /// Owns the receiver directly so unsubscribe and disconnect release it without another event.
    pub fn tempo_consensus(
        events: broadcast::Receiver<Arc<TempoConsensusEvent>>,
        id: SubscriptionId,
    ) -> Self {
        let events = futures::stream::unfold((events, id.clone()), |(mut events, id)| async move {
            loop {
                match events.recv().await {
                    Ok(event) => return Some((event, (events, id))),
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(target: "rpc", %id, skipped, "Tempo consensus subscription lagged; recover retained blocks with consensus_getFinalization");
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        });
        Self::TempoConsensus(Box::pin(events), id)
    }
}

impl<N: Network> Stream for EthSubscription<N>
where
    N::ReceiptEnvelope: TxReceipt<Log = alloy_primitives::Log>,
{
    type Item = serde_json::Value;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();
        match ready!(pin.poll_response(cx)) {
            None => Poll::Ready(None),
            Some(res) => Poll::Ready(Some(serde_json::to_value(res).expect("can't fail;"))),
        }
    }
}

/// Returns all the logs that match the given filter
pub fn filter_logs<R>(block: Block, receipts: Vec<R>, filter: &FilteredParams) -> Vec<Log>
where
    R: TxReceipt<Log = alloy_primitives::Log>,
{
    /// Determines whether to add this log
    fn add_log(
        block_hash: B256,
        l: &alloy_primitives::Log,
        block: &Block,
        params: &FilteredParams,
    ) -> bool {
        if params.filter.is_some() {
            let block_number = block.header.number();
            if !params.filter_block_range(block_number)
                || !params.filter_block_hash(block_hash)
                || !params.filter_address(&l.address)
                || !params.filter_topics(l.topics())
            {
                return false;
            }
        }
        true
    }

    let block_hash = block.header.hash_slow();
    let mut logs = vec![];
    let mut log_index: u32 = 0;
    for (receipt_index, receipt) in receipts.into_iter().enumerate() {
        let transaction_hash = block.body.transactions[receipt_index].hash();
        for log in receipt.logs() {
            if add_log(block_hash, log, &block, filter) {
                logs.push(Log {
                    inner: log.clone(),
                    block_hash: Some(block_hash),
                    block_number: Some(block.header.number()),
                    transaction_hash: Some(transaction_hash),
                    transaction_index: Some(receipt_index as u64),
                    log_index: Some(log_index as u64),
                    removed: false,
                    block_timestamp: Some(block.header.timestamp()),
                });
            }
            log_index += 1;
        }
    }
    logs
}

/// Returns all logs of reorged out blocks that match the given filter.
///
/// The logs are expected to already be marked as `removed` and carry the metadata of the block
/// they were originally included in, so the filter is applied against that metadata.
pub fn filter_removed_logs(logs: &[Log], filter: &FilteredParams) -> Vec<Log> {
    logs.iter()
        .filter(|log| {
            filter.filter.is_none()
                || (filter.filter_block_range(log.block_number.unwrap_or_default())
                    && filter.filter_block_hash(log.block_hash.unwrap_or_default())
                    && filter.filter_address(&log.inner.address)
                    && filter.filter_topics(log.inner.topics()))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eth::{
        backend::tempo_consensus::TempoFinalization, tempo_consensus::TempoCertifiedBlock,
    };
    use foundry_primitives::FoundryNetwork;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn tempo_subscription_wire_format_lag_and_cleanup() {
        let events = broadcast::Sender::new(1);
        let id = SubscriptionId::String("0x1234".to_owned());
        let mut subscription =
            EthSubscription::<FoundryNetwork>::tempo_consensus(events.subscribe(), id.clone());
        assert_eq!(events.receiver_count(), 1);

        let mut latest = None;
        for view in 1..=2 {
            let event = Arc::new(TempoConsensusEvent::Finalized {
                block: TempoCertifiedBlock {
                    finalization: TempoFinalization {
                        epoch: 0,
                        view,
                        digest: B256::ZERO,
                        certificate: Default::default(),
                    },
                    block: Default::default(),
                },
                seen: 1234,
            });
            events.send(event.clone()).unwrap();
            latest = Some(event);
        }

        let expected_response = |event: &TempoConsensusEvent| {
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "consensus_event",
                "params": { "subscription": id, "result": event },
            })
        };
        let response = timeout(Duration::from_secs(1), subscription.next()).await.unwrap().unwrap();
        assert_eq!(response, expected_response(latest.as_ref().unwrap()));
        let mut next = subscription.next();
        assert!(futures::poll!(&mut next).is_pending());

        let mut event = latest.unwrap();
        let TempoConsensusEvent::Finalized { block, .. } = Arc::make_mut(&mut event);
        block.finalization.view = 3;
        events.send(event.clone()).unwrap();

        let response = timeout(Duration::from_secs(1), next).await.unwrap().unwrap();
        assert_eq!(response, expected_response(&event));
        drop(subscription);
        assert_eq!(events.receiver_count(), 0);

        let mut subscription =
            EthSubscription::<FoundryNetwork>::tempo_consensus(events.subscribe(), id);
        drop(events);
        assert!(timeout(Duration::from_secs(1), subscription.next()).await.unwrap().is_none());
    }
}
