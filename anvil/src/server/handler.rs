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
use anvil_rpc::{error::RpcError, request::Version, response::ResponseResult};
use anvil_server::{RpcHandler, WsContext, WsRpcHandler};
use ethers::types::{Log as EthersLog, TxHash, H256, U256, U64};
use futures::{channel::mpsc::Receiver, ready, Stream, StreamExt};
use serde::Serialize;
use std::{
    collections::VecDeque,
    pin::Pin,
    task::{Context, Poll},
};
use tracing::trace;

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
        let id = SubscriptionId::random_hex();
        trace!(target: "rpc::ws", "received pubsub request {:?}", pubsub);
        match pubsub {
            EthPubSub::EthUnSubscribe(id) => {
                trace!(target: "rpc::ws", "canceling subscription {:?}", id);
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
                        trace!(target: "rpc::ws", "received logs subscription {:?}", params);
                        let blocks = self.api.new_block_notifications();
                        let storage = self.api.storage_info();
                        EthSubscription::Logs(Box::new(LogsListener {
                            blocks,
                            storage,
                            filter: params,
                            queued: Default::default(),
                            id: id.clone(),
                        }))
                    }
                    SubscriptionKind::NewHeads => {
                        trace!(target: "rpc::ws", "received header subscription");
                        let blocks = self.api.new_block_notifications();
                        let storage = self.api.storage_info();
                        EthSubscription::Header(blocks, storage, id.clone())
                    }
                    SubscriptionKind::NewPendingTransactions => {
                        trace!(target: "rpc::ws", "received pending transactions subscription");
                        EthSubscription::PendingTransactions(
                            self.api.new_ready_transactions(),
                            id.clone(),
                        )
                    }
                    SubscriptionKind::Syncing => {
                        return RpcError::internal_error_with("Not implemented").into()
                    }
                };

                cx.add_subscription(id.clone(), subscription);

                trace!(target: "rpc::ws", "created new subscription: {:?}", id);
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
        trace!(target: "rpc::ws", "received ws request {:?}", request);
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
    id: SubscriptionId,
}

// === impl SubscriptionListener ===

impl LogsListener {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Option<EthSubscriptionResponse>> {
        loop {
            if let Some(log) = self.queued.pop_front() {
                let params = EthSubscriptionParams {
                    subscription: self.id.clone(),
                    result: to_rpc_result(log),
                };
                return Poll::Ready(Some(EthSubscriptionResponse::new(params)))
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

#[derive(Debug, PartialEq, Clone, Serialize)]
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

#[derive(Debug, PartialEq, Clone, Serialize)]
pub struct EthSubscriptionParams {
    subscription: SubscriptionId,
    #[serde(flatten)]
    result: ResponseResult,
}

/// Represents an ethereum subscription
#[derive(Debug)]
pub enum EthSubscription {
    Logs(Box<LogsListener>),
    Header(NewBlockNotifications, StorageInfo, SubscriptionId),
    PendingTransactions(Receiver<TxHash>, SubscriptionId),
}

// === impl EthSubscription ===

impl EthSubscription {
    fn poll_response(&mut self, cx: &mut Context<'_>) -> Poll<Option<EthSubscriptionResponse>> {
        match self {
            EthSubscription::Logs(listener) => listener.poll(cx),
            EthSubscription::Header(blocks, storage, id) => {
                if let Some(block) = ready!(blocks.poll_next_unpin(cx)) {
                    if let Some(block) = storage.eth_block(block.hash) {
                        let params = EthSubscriptionParams {
                            subscription: id.clone(),
                            result: to_rpc_result(block),
                        };
                        Poll::Ready(Some(EthSubscriptionResponse::new(params)))
                    } else {
                        Poll::Pending
                    }
                } else {
                    Poll::Ready(None)
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
