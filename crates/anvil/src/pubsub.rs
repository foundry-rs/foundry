use crate::{
    eth::{backend::notifications::NewBlockNotifications, error::to_rpc_result},
    StorageInfo,
};
use alloy_primitives::{TxHash, B256, U256, U64};
use alloy_rpc_types::{pubsub::SubscriptionResult, FilteredParams, Log as AlloyLog};
use anvil_core::eth::{
    block::Block,
    receipt::{EIP658Receipt, Log, TypedReceipt},
    subscription::SubscriptionId,
};
use anvil_rpc::{request::Version, response::ResponseResult};
use foundry_utils::types::ToAlloy;
use futures::{channel::mpsc::Receiver, ready, Stream, StreamExt};
use serde::Serialize;
use std::{
    collections::VecDeque,
    pin::Pin,
    task::{Context, Poll},
};

/// Listens for new blocks and matching logs emitted in that block
#[derive(Debug)]
pub struct LogsSubscription {
    pub blocks: NewBlockNotifications,
    pub storage: StorageInfo,
    pub filter: FilteredParams,
    pub queued: VecDeque<AlloyLog>,
    pub id: SubscriptionId,
}

// === impl LogsSubscription ===

impl LogsSubscription {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Option<EthSubscriptionResponse>> {
        loop {
            if let Some(log) = self.queued.pop_front() {
                let params = EthSubscriptionParams {
                    subscription: self.id.clone(),
                    result: to_rpc_result(log),
                };
                return Poll::Ready(Some(EthSubscriptionResponse::new(params)));
            }

            if let Some(block) = ready!(self.blocks.poll_next_unpin(cx)) {
                let b = self.storage.block(block.hash.to_alloy());
                let receipts = self.storage.receipts(block.hash.to_alloy());
                if let (Some(receipts), Some(block)) = (receipts, b) {
                    let logs = filter_logs(block, receipts, &self.filter);
                    if logs.is_empty() {
                        // this ensures we poll the receiver until it is pending, in which case the
                        // underlying `UnboundedReceiver` will register the new waker, see
                        // [`futures::channel::mpsc::UnboundedReceiver::poll_next()`]
                        continue;
                    }
                    self.queued.extend(logs)
                }
            } else {
                return Poll::Ready(None);
            }

            if self.queued.is_empty() {
                return Poll::Pending;
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize)]
pub struct EthSubscriptionResponse {
    jsonrpc: Version,
    method: &'static str,
    params: EthSubscriptionParams,
}

// === impl EthSubscriptionResponse ===

impl EthSubscriptionResponse {
    pub fn new(params: EthSubscriptionParams) -> Self {
        Self { jsonrpc: Version::V2, method: "eth_subscription", params }
    }
}

/// Represents the `params` field of an `eth_subscription` event
#[derive(Debug, PartialEq, Eq, Clone, Serialize)]
pub struct EthSubscriptionParams {
    subscription: SubscriptionId,
    #[serde(flatten)]
    result: ResponseResult,
}

/// Represents an ethereum Websocket subscription
#[derive(Debug)]
pub enum EthSubscription {
    Logs(Box<LogsSubscription>),
    Header(NewBlockNotifications, StorageInfo, SubscriptionId),
    PendingTransactions(Receiver<TxHash>, SubscriptionId),
}

// === impl EthSubscription ===

impl EthSubscription {
    fn poll_response(&mut self, cx: &mut Context<'_>) -> Poll<Option<EthSubscriptionResponse>> {
        match self {
            EthSubscription::Logs(listener) => listener.poll(cx),
            EthSubscription::Header(blocks, storage, id) => {
                // this loop ensures we poll the receiver until it is pending, in which case the
                // underlying `UnboundedReceiver` will register the new waker, see
                // [`futures::channel::mpsc::UnboundedReceiver::poll_next()`]
                loop {
                    if let Some(block) = ready!(blocks.poll_next_unpin(cx)) {
                        if let Some(block) = storage.eth_block(block.hash.to_alloy()) {
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
            EthSubscription::PendingTransactions(tx, id) => {
                let res = ready!(tx.poll_next_unpin(cx))
                    .map(SubscriptionResult::TransactionHash)
                    .map(to_rpc_result)
                    .map(|result| {
                        let params = EthSubscriptionParams { subscription: id.clone(), result };
                        EthSubscriptionResponse::new(params)
                    });
                Poll::Ready(res)
            }
        }
    }
}

impl Stream for EthSubscription {
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
pub fn filter_logs(
    block: Block,
    receipts: Vec<TypedReceipt>,
    filter: &FilteredParams,
) -> Vec<AlloyLog> {
    /// Determines whether to add this log
    fn add_log(block_hash: B256, l: &Log, block: &Block, params: &FilteredParams) -> bool {
        let log = AlloyLog {
            address: l.address.to_alloy(),
            topics: l.topics.clone().into_iter().map(|t| t.to_alloy()).collect::<Vec<_>>(),
            data: l.data.clone().0.into(),
            block_hash: None,
            block_number: None,
            transaction_hash: None,
            transaction_index: None,
            log_index: None,
            removed: false,
        };
        if params.filter.is_some() {
            let block_number = block.header.number.as_u64();
            if !params.filter_block_range(block_number)
                || !params.filter_block_hash(block_hash)
                || !params.filter_address(&log)
                || !params.filter_topics(&log)
            {
                return false;
            }
        }
        true
    }

    let block_hash = block.header.hash();
    let mut logs = vec![];
    let mut log_index: u32 = 0;
    for (receipt_index, receipt) in receipts.into_iter().enumerate() {
        let receipt: EIP658Receipt = receipt.into();
        let receipt_logs = receipt.logs;
        let transaction_hash: Option<B256> = if !receipt_logs.is_empty() {
            Some(block.transactions[receipt_index].hash().to_alloy())
        } else {
            None
        };
        for (transaction_log_index, log) in receipt_logs.into_iter().enumerate() {
            if add_log(block_hash.to_alloy(), &log, &block, filter) {
                logs.push(AlloyLog {
                    address: log.address.to_alloy(),
                    topics: log.topics.into_iter().map(|t| t.to_alloy()).collect::<Vec<_>>(),
                    data: log.data.0.into(),
                    block_hash: Some(block_hash.to_alloy()),
                    block_number: Some(U256::from(block.header.number.as_u64())),
                    transaction_hash,
                    transaction_index: Some(U256::from(receipt_index)),
                    log_index: Some(U256::from(log_index)),
                    removed: false,
                });
            }
            log_index += 1;
        }
    }
    logs
}
