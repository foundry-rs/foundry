//! Contains RPC handlers
use crate::{
    eth::{backend::notifications::NewBlockNotifications, error::to_rpc_result},
    EthApi, StorageInfo,
};
use anvil_core::eth::{
    block::Block,
    filter::FilteredParams,
    receipt::{EIP658Receipt, Log, TypedReceipt},
    subscription::{SubscriptionId, SubscriptionKind, SubscriptionParams, SubscriptionResult},
    EthPubSub, EthRequest, EthRpcCall,
};
use anvil_rpc::{error::RpcError, response::ResponseResult};
use anvil_server::{RpcHandler, WsContext, WsRpcHandler};
use ethers::types::{Log as EthersLog, TxHash, H256, U256, U64};
use futures::{channel::mpsc::Receiver, ready, Stream, StreamExt};
use std::{
    collections::VecDeque,
    pin::Pin,
    task::{Context, Poll},
};

/// A `RpcHandler` that expects `EthRequest` rpc calls via http
#[derive(Clone)]
pub struct HttpEthRpcHandler {
    /// Access to the node
    api: EthApi,
}

// === impl WsEthRpcHandler ===

impl HttpEthRpcHandler {
    /// Creates a new instance of the handler using the given `EthApi`
    pub fn new(api: EthApi) -> Self {
        Self { api }
    }
}

#[async_trait::async_trait]
impl RpcHandler for HttpEthRpcHandler {
    type Request = EthRequest;

    async fn on_request(&self, request: Self::Request) -> ResponseResult {
        self.api.execute(request).await
    }
}

/// A `RpcHandler` that expects `EthRequest` rpc calls and `EthPubSub` via websocket
#[derive(Clone)]
pub struct WsEthRpcHandler {
    /// Access to the node
    api: EthApi,
}

impl WsEthRpcHandler {
    /// Creates a new instance of the handler using the given `EthApi`
    pub fn new(api: EthApi) -> Self {
        Self { api }
    }

    /// Invoked for an ethereum pubsub rpc call
    async fn on_pub_sub(&self, pubsub: EthPubSub, cx: WsContext<Self>) -> ResponseResult {
        match pubsub {
            EthPubSub::EthUnSubscribe(id) => {
                let canceled = cx.remove_subscription(&id).is_some();
                ResponseResult::Success(canceled.into())
            }
            EthPubSub::EthSubscribe(kind, params) => {
                let params = match params {
                    SubscriptionParams::Logs(filter) => FilteredParams::new(Some(filter)),
                    _ => FilteredParams::default(),
                };

                let subscription = match kind {
                    SubscriptionKind::Logs => {
                        let blocks = self.api.new_block_notifications();
                        let storage = self.api.storage_info();
                        EthSubscription::Logs(Box::new(LogsListener {
                            blocks,
                            storage,
                            filter: params,
                            queued: Default::default(),
                        }))
                    }
                    SubscriptionKind::NewHeads => {
                        let blocks = self.api.new_block_notifications();
                        let storage = self.api.storage_info();
                        EthSubscription::Header(blocks, storage)
                    }
                    SubscriptionKind::NewPendingTransactions => {
                        EthSubscription::PendingTransactions(self.api.new_ready_transactions())
                    }
                    SubscriptionKind::Syncing => {
                        return RpcError::internal_error_with("Not implemented").into()
                    }
                };

                let id = SubscriptionId::random_hex();
                cx.add_subscription(id.clone(), subscription);

                to_rpc_result(id)
            }
        }
    }
}

#[async_trait::async_trait]
impl WsRpcHandler for WsEthRpcHandler {
    type Request = EthRpcCall;
    type SubscriptionId = SubscriptionId;
    type Subscription = EthSubscription;

    async fn on_request(&self, request: Self::Request, cx: WsContext<Self>) -> ResponseResult {
        match request {
            EthRpcCall::Request(request) => self.api.execute(request).await,
            EthRpcCall::PubSub(pubsub) => self.on_pub_sub(pubsub, cx).await,
        }
    }
}

/// Listens for new blocks and matching logs emitted in that block
#[derive(Debug)]
pub struct LogsListener {
    blocks: NewBlockNotifications,
    storage: StorageInfo,
    filter: FilteredParams,
    queued: VecDeque<EthersLog>,
}

// === impl SubscriptionListener ===

impl LogsListener {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Option<ResponseResult>> {
        loop {
            if let Some(log) = self.queued.pop_front() {
                return Poll::Ready(Some(to_rpc_result(log)))
            }

            if let Some(block) = ready!(self.blocks.poll_next_unpin(cx)) {
                let b = self.storage.block(block.hash);
                let receipts = self.storage.receipts(block.hash);
                if let (Some(receipts), Some(block)) = (receipts, b) {
                    self.queued.extend(logs(block, receipts, &self.filter))
                }
            } else {
                return Poll::Ready(None)
            }

            if self.queued.is_empty() {
                return Poll::Pending
            }
        }
    }
}

/// Represents an ethereum subscription
#[derive(Debug)]
pub enum EthSubscription {
    Logs(Box<LogsListener>),
    Header(NewBlockNotifications, StorageInfo),
    PendingTransactions(Receiver<TxHash>),
}

impl Stream for EthSubscription {
    type Item = ResponseResult;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();
        match pin {
            EthSubscription::Logs(listener) => listener.poll(cx),
            EthSubscription::Header(blocks, storage) => {
                if let Some(block) = ready!(blocks.poll_next_unpin(cx)) {
                    if let Some(block) = storage.eth_block(block.hash) {
                        Poll::Ready(Some(to_rpc_result(block)))
                    } else {
                        Poll::Pending
                    }
                } else {
                    Poll::Ready(None)
                }
            }
            EthSubscription::PendingTransactions(tx) => {
                let res = ready!(tx.poll_next_unpin(cx))
                    .map(SubscriptionResult::TransactionHash)
                    .map(to_rpc_result);
                Poll::Ready(res)
            }
        }
    }
}

/// Returns all the logs that match the given filter
fn logs(block: Block, receipts: Vec<TypedReceipt>, filter: &FilteredParams) -> Vec<EthersLog> {
    /// Determines whether to add this log
    fn add_log(block_hash: H256, l: &Log, block: &Block, params: &FilteredParams) -> bool {
        let log = EthersLog {
            address: l.address,
            topics: l.topics.clone(),
            data: l.data.clone(),
            block_hash: None,
            block_number: None,
            transaction_hash: None,
            transaction_index: None,
            log_index: None,
            transaction_log_index: None,
            log_type: None,
            removed: Some(false),
        };
        if params.filter.is_some() {
            let block_number = block.header.number.as_u64();
            if !params.filter_block_range(block_number) ||
                !params.filter_block_hash(block_hash) ||
                !params.filter_address(&log) ||
                !params.filter_topics(&log)
            {
                return false
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
        let transaction_hash: Option<H256> = if !receipt_logs.is_empty() {
            Some(block.transactions[receipt_index as usize].hash())
        } else {
            None
        };
        for (transaction_log_index, log) in receipt_logs.into_iter().enumerate() {
            if add_log(block_hash, &log, &block, filter) {
                logs.push(EthersLog {
                    address: log.address,
                    topics: log.topics,
                    data: log.data,
                    block_hash: Some(block_hash),
                    block_number: Some(block.header.number.as_u64().into()),
                    transaction_hash,
                    transaction_index: Some(U64::from(receipt_index)),
                    log_index: Some(U256::from(log_index)),
                    transaction_log_index: Some(U256::from(transaction_log_index)),
                    log_type: None,
                    removed: Some(false),
                });
            }
            log_index += 1;
        }
    }
    logs
}
