//! Anvil-style stdout logging: the RPC methods served, and the blocks and transactions mined.
//!
//! `--silent` disables the output at startup; `anvil_setLoggingEnabled` toggles it at runtime.

use alloy_consensus::{BlockHeader, Transaction, TxReceipt, transaction::TxHashRef};
use alloy_primitives::TxKind;
use chrono::{DateTime, Datelike, Utc};
use jsonrpsee::{
    MethodResponse,
    core::middleware::{Batch, Notification},
    server::middleware::rpc::RpcServiceT,
    types::Request,
};
use reth_ethereum::{
    primitives::{BlockBody, NodePrimitives, SignerRecoverable},
    provider::CanonStateNotifications,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::broadcast::error::RecvError;
use tower::Layer;

/// Whether the node prints to stdout.
#[derive(Clone, Debug)]
pub struct LoggingState {
    enabled: Arc<AtomicBool>,
}

impl LoggingState {
    /// Creates the state with logging on or off.
    pub fn new(enabled: bool) -> Self {
        Self { enabled: Arc::new(AtomicBool::new(enabled)) }
    }

    /// Returns whether logging is on.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Turns logging on or off.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }
}

/// Prints a line to stdout when logging is on.
fn node_info(logging: &LoggingState, line: impl AsRef<str>) {
    if logging.is_enabled() {
        let _ = foundry_common::sh_println!("{}", line.as_ref());
    }
}

/// Prints every mined block and its transactions, as anvil does.
pub async fn log_mined_blocks<N>(
    mut notifications: CanonStateNotifications<N>,
    logging: LoggingState,
) where
    N: NodePrimitives<SignedTx: Transaction + SignerRecoverable + TxHashRef, Receipt: TxReceipt>,
{
    loop {
        match notifications.recv().await {
            Ok(notification) => {
                let committed = notification.committed();
                for (block, receipts) in committed.blocks_and_receipts() {
                    if !logging.is_enabled() {
                        continue;
                    }
                    node_info(&logging, "");
                    let mut previous_gas = 0u64;
                    for (tx, receipt) in block.body().transactions().iter().zip(receipts) {
                        node_info(&logging, format!("    Transaction: {:?}", tx.tx_hash()));
                        if tx.kind() == TxKind::Create
                            && let Ok(sender) = tx.recover_signer_unchecked()
                        {
                            let contract = sender.create(tx.nonce());
                            node_info(&logging, format!("    Contract created: {contract}"));
                        }
                        let gas_used = receipt.cumulative_gas_used().saturating_sub(previous_gas);
                        previous_gas = receipt.cumulative_gas_used();
                        node_info(&logging, format!("    Gas used: {gas_used}"));
                        if !receipt.status() {
                            node_info(&logging, "    Error: reverted");
                        }
                        node_info(&logging, "");
                    }
                    node_info(&logging, format!("    Block Number: {}", block.number()));
                    node_info(&logging, format!("    Block Hash: {:?}", block.hash()));
                    let timestamp = DateTime::<Utc>::from_timestamp(block.timestamp() as i64, 0)
                        .unwrap_or(DateTime::<Utc>::MAX_UTC);
                    let time = if timestamp.year() > 9999 {
                        timestamp.to_rfc3339()
                    } else {
                        timestamp.to_rfc2822()
                    };
                    node_info(&logging, format!("    Block Time: {time:?}\n"));
                }
            }
            Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
    }
}

/// RPC middleware that prints the name of every method served.
#[derive(Clone, Debug)]
pub struct NodeInfoLayer {
    logging: LoggingState,
}

impl NodeInfoLayer {
    /// Creates the layer.
    pub const fn new(logging: LoggingState) -> Self {
        Self { logging }
    }
}

impl<S> Layer<S> for NodeInfoLayer {
    type Service = NodeInfoService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        NodeInfoService { inner, logging: self.logging.clone() }
    }
}

/// The service produced by [`NodeInfoLayer`].
#[derive(Clone, Debug)]
pub struct NodeInfoService<S> {
    inner: S,
    logging: LoggingState,
}

impl<S> RpcServiceT for NodeInfoService<S>
where
    S: RpcServiceT<MethodResponse = MethodResponse> + Send + Sync + Clone + 'static,
{
    type MethodResponse = S::MethodResponse;
    type NotificationResponse = S::NotificationResponse;
    type BatchResponse = S::BatchResponse;

    fn call<'a>(
        &self,
        request: Request<'a>,
    ) -> impl Future<Output = Self::MethodResponse> + Send + 'a {
        node_info(&self.logging, request.method_name());
        self.inner.call(request)
    }

    fn batch<'a>(
        &self,
        requests: Batch<'a>,
    ) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        for entry in requests.iter().flatten() {
            node_info(&self.logging, entry.method_name());
        }
        self.inner.batch(requests)
    }

    fn notification<'a>(
        &self,
        notification: Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        self.inner.notification(notification)
    }
}
