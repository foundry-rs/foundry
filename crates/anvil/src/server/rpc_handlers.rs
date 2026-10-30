//! Contains RPC handlers
use crate::{
    EthApi,
    eth::{
        error::{ToRpcResponseResult, to_rpc_result},
        macros::node_info,
    },
    pubsub::{EthSubscription, LogsSubscription},
};
use alloy_rpc_types::{
    FilteredParams,
    pubsub::{Params, SubscriptionKind},
};
use anvil_core::eth::{EthPubSub, EthRequest, EthRpcCall, subscription::SubscriptionId};
use anvil_rpc::{error::RpcError, request::Id, response::ResponseResult};
use anvil_server::{PubSubContext, PubSubRpcHandler, RpcHandler};
use foundry_primitives::FoundryNetwork;

/// A `RpcHandler` that expects `EthRequest` rpc calls via http
#[derive(Clone)]
pub struct HttpEthRpcHandler {
    /// Access to the node
    api: EthApi<FoundryNetwork>,
}

impl HttpEthRpcHandler {
    /// Creates a new instance of the handler using the given `EthApi`
    pub const fn new(api: EthApi<FoundryNetwork>) -> Self {
        Self { api }
    }
}

impl RpcHandler for HttpEthRpcHandler {
    type Request = EthRequest;

    fn on_request(&self, request: Self::Request) -> impl Future<Output = ResponseResult> + Send {
        self.api.execute(request)
    }

    fn on_request_error(
        &self,
        method: &str,
        id: &Id,
        params: &serde_json::Value,
        error: &RpcError,
    ) {
        log_request_error(method, id, params, error);
    }
}

/// A `RpcHandler` that expects `EthRequest` rpc calls and `EthPubSub` via pubsub connection
#[derive(Clone)]
pub struct PubSubEthRpcHandler {
    /// Access to the node
    api: EthApi<FoundryNetwork>,
}

impl PubSubEthRpcHandler {
    /// Creates a new instance of the handler using the given `EthApi`
    pub const fn new(api: EthApi<FoundryNetwork>) -> Self {
        Self { api }
    }

    /// Invoked for an ethereum pubsub rpc call
    async fn on_pub_sub(&self, pubsub: EthPubSub, cx: PubSubContext<Self>) -> ResponseResult {
        let id = SubscriptionId::random_hex();
        trace!(target: "rpc::ws", "received pubsub request {:?}", pubsub);
        match pubsub {
            EthPubSub::EthUnSubscribe(id) => {
                trace!(target: "rpc::ws", "canceling subscription {:?}", id);
                let canceled = cx.remove_subscription(&PubSubSubscriptionId::Eth(id)).is_some();
                ResponseResult::success(canceled)
            }
            EthPubSub::EthSubscribe(kind, raw_params) => {
                let filter = match &*raw_params {
                    Params::None => None,
                    Params::Logs(filter) => Some(filter.clone()),
                    Params::Bool(_) => None,
                    Params::TransactionReceipts(_) => None,
                };
                let params = FilteredParams::new(filter.map(|b| *b));

                let subscription = match kind {
                    SubscriptionKind::Logs => {
                        if raw_params.is_bool() {
                            return ResponseResult::Error(RpcError::invalid_params(
                                "Expected params for logs subscription",
                            ));
                        }

                        trace!(target: "rpc::ws", "received logs subscription {:?}", params);
                        let blocks = self.api.new_block_notifications();
                        let storage = self.api.storage_info();
                        EthSubscription::Logs(Box::new(LogsSubscription {
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
                        match *raw_params {
                            Params::Bool(true) => EthSubscription::FullPendingTransactions(
                                self.api.full_pending_transactions(),
                                id.clone(),
                            ),
                            Params::Bool(false) | Params::None => {
                                EthSubscription::PendingTransactions(
                                    self.api.new_ready_transaction_subscription(),
                                    id.clone(),
                                )
                            }
                            _ => {
                                return ResponseResult::Error(RpcError::invalid_params(
                                    "Expected boolean parameter for newPendingTransactions",
                                ));
                            }
                        }
                    }
                    SubscriptionKind::TransactionReceipts => {
                        trace!(target: "rpc::ws", "received transaction receipts subscription");
                        match *raw_params {
                            Params::None => EthSubscription::TransactionReceipts(
                                self.api.transaction_receipts_subscription(Default::default()),
                                id.clone(),
                            ),
                            Params::TransactionReceipts(filter) => {
                                EthSubscription::TransactionReceipts(
                                    self.api.transaction_receipts_subscription(filter),
                                    id.clone(),
                                )
                            }
                            _ => {
                                return ResponseResult::Error(RpcError::invalid_params(
                                    "Expected transactionReceipts params",
                                ));
                            }
                        }
                    }
                    SubscriptionKind::Syncing => {
                        trace!(target: "rpc::ws", "received syncing subscription");
                        EthSubscription::Syncing(Some(id.clone()))
                    }
                };

                cx.add_subscription(PubSubSubscriptionId::Eth(id.clone()), subscription);

                trace!(target: "rpc::ws", "created new subscription: {:?}", id);
                to_rpc_result(id)
            }
            EthPubSub::ConsensusSubscribe(()) => {
                let events = match self.api.backend.tempo_consensus_notifications().await {
                    Ok(events) => events,
                    Err(error) => return Err::<(), _>(error).to_rpc_result(),
                };
                let subscription = EthSubscription::tempo_consensus(events, id.clone());
                cx.add_subscription(PubSubSubscriptionId::Consensus(id.clone()), subscription);
                to_rpc_result(id)
            }
            EthPubSub::ConsensusUnsubscribe(id) => {
                let canceled =
                    cx.remove_subscription(&PubSubSubscriptionId::Consensus(id)).is_some();
                ResponseResult::success(canceled)
            }
        }
    }
}

impl PubSubRpcHandler for PubSubEthRpcHandler {
    type Request = EthRpcCall;
    type SubscriptionId = PubSubSubscriptionId;
    type Subscription = EthSubscription<FoundryNetwork>;

    async fn on_request(&self, request: Self::Request, cx: PubSubContext<Self>) -> ResponseResult {
        match request {
            EthRpcCall::Request(request) => self.api.execute(*request).await,
            EthRpcCall::PubSub(pubsub) => self.on_pub_sub(pubsub, cx).await,
        }
    }

    fn close_on_subscription_end(&self, subscription: &Self::Subscription) -> bool {
        matches!(
            subscription,
            EthSubscription::Logs(_)
                | EthSubscription::PendingTransactions(..)
                | EthSubscription::FullPendingTransactions(..)
        )
    }

    fn on_request_error(
        &self,
        method: &str,
        id: &Id,
        params: &serde_json::Value,
        error: &RpcError,
    ) {
        log_request_error(method, id, params, error);
    }
}

fn log_request_error(method: &str, id: &Id, params: &serde_json::Value, error: &RpcError) {
    node_info!(method, ?id, %params, error = %error, "RPC request failed");
}

/// Namespaced connection-local keys preserve each subscription API's cancellation scope.
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum PubSubSubscriptionId {
    /// An Ethereum subscription.
    Eth(SubscriptionId),
    /// A Tempo consensus subscription.
    Consensus(SubscriptionId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeConfig, spawn};
    use futures::StreamExt;
    use tokio::sync::broadcast;

    #[tokio::test(flavor = "multi_thread")]
    async fn closes_connection_when_loss_sensitive_subscription_ends() {
        let (api, _handle) = spawn(NodeConfig::test()).await;
        let handler = PubSubEthRpcHandler::new(api.clone());
        let id = SubscriptionId::Number(1);
        let logs = EthSubscription::Logs(Box::new(LogsSubscription {
            blocks: api.new_block_notifications(),
            storage: api.storage_info(),
            filter: FilteredParams::new(None),
            queued: Default::default(),
            id: id.clone(),
        }));
        assert!(handler.close_on_subscription_end(&logs));

        let (_, hashes) = futures::channel::mpsc::channel(1);
        let pending = EthSubscription::<FoundryNetwork>::PendingTransactions(hashes, id.clone());
        assert!(handler.close_on_subscription_end(&pending));

        let (_, full_transactions) = tokio::sync::mpsc::unbounded_channel();
        let full = EthSubscription::<FoundryNetwork>::FullPendingTransactions(
            full_transactions,
            id.clone(),
        );
        assert!(handler.close_on_subscription_end(&full));
        assert!(!handler.close_on_subscription_end(&EthSubscription::Syncing(None)));

        let events = broadcast::Sender::new(1);
        let consensus = EthSubscription::tempo_consensus(events.subscribe(), id);
        assert!(!handler.close_on_subscription_end(&consensus));
    }

    #[tokio::test]
    async fn tempo_unsubscribe_preserves_other_namespaces_and_drops_receiver() {
        let (api, _handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
        let events = broadcast::Sender::new(1);
        let id = SubscriptionId::String("0x1234".to_owned());
        let context = PubSubContext::<PubSubEthRpcHandler>::default();
        let handler = PubSubEthRpcHandler::new(api);
        let mut subscription = EthSubscription::tempo_consensus(events.subscribe(), id.clone());
        assert!(futures::poll!(subscription.next()).is_pending());
        context.add_subscription(PubSubSubscriptionId::Consensus(id.clone()), subscription);
        context.add_subscription(
            PubSubSubscriptionId::Eth(id.clone()),
            EthSubscription::Syncing(Some(id.clone())),
        );
        assert_eq!(events.receiver_count(), 1);

        assert_eq!(
            handler.on_pub_sub(EthPubSub::EthUnSubscribe(id.clone()), context.clone()).await,
            ResponseResult::success(true),
        );
        assert_eq!(events.receiver_count(), 1);
        context.add_subscription(
            PubSubSubscriptionId::Eth(id.clone()),
            EthSubscription::Syncing(Some(id.clone())),
        );
        assert_eq!(
            handler.on_pub_sub(EthPubSub::ConsensusUnsubscribe(id.clone()), context.clone()).await,
            ResponseResult::success(true),
        );
        assert_eq!(events.receiver_count(), 0);
        assert_eq!(
            handler.on_pub_sub(EthPubSub::ConsensusUnsubscribe(id.clone()), context.clone()).await,
            ResponseResult::success(false),
        );
        assert!(context.remove_subscription(&PubSubSubscriptionId::Eth(id)).is_some());
    }

    #[tokio::test]
    async fn tempo_disconnect_drops_idle_receiver() {
        let events = broadcast::Sender::new(1);
        let id = SubscriptionId::String("0x1234".to_owned());
        let context = PubSubContext::<PubSubEthRpcHandler>::default();
        let mut subscription = EthSubscription::tempo_consensus(events.subscribe(), id.clone());
        assert!(futures::poll!(subscription.next()).is_pending());
        context.add_subscription(PubSubSubscriptionId::Consensus(id), subscription);
        assert_eq!(events.receiver_count(), 1);
        drop(context);
        assert_eq!(events.receiver_count(), 0);
    }
}
