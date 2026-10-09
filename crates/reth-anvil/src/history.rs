//! `--prune-history`: anvil keeps the states of the last blocks in memory and answers a state
//! read at an older block with an error. Reth keeps the full history on disk, so this RPC
//! middleware rejects the reads anvil would have pruned, with anvil's error.

use crate::server::SharedModule;
use alloy_primitives::U64;
use jsonrpsee::{
    MethodResponse,
    core::{
        middleware::{Batch, Notification},
        params::ArrayParams,
    },
    server::middleware::rpc::RpcServiceT,
    types::{ErrorObjectOwned, Request, error::INVALID_PARAMS_CODE},
};
use serde_json::Value;
use tower::Layer;

/// The state reads that name a block, and the position of the block among their parameters.
const STATE_READS: [(&str, usize); 12] = [
    ("eth_getBalance", 1),
    ("eth_getCode", 1),
    ("eth_getTransactionCount", 1),
    ("eth_getStorageAt", 2),
    ("eth_getProof", 2),
    ("eth_getAccount", 1),
    ("eth_getAccountInfo", 1),
    ("eth_call", 1),
    ("eth_estimateGas", 1),
    ("eth_createAccessList", 1),
    ("debug_traceCall", 1),
    ("trace_call", 2),
];

/// Rejects state reads at blocks whose state anvil would have pruned.
#[derive(Clone, Debug)]
pub struct PruneHistoryLayer {
    /// The number of past states kept besides the latest one, when pruning.
    keep: Option<u64>,
    module: SharedModule,
}

impl PruneHistoryLayer {
    /// Creates the layer for `--prune-history`: with a limit, the states of that many past
    /// blocks stay; without one, only the latest state does. Without the flag, no state is
    /// pruned.
    pub fn new(prune_history: Option<Option<usize>>, module: SharedModule) -> Self {
        Self { keep: prune_history.map(|keep| keep.unwrap_or_default() as u64), module }
    }
}

impl<S> Layer<S> for PruneHistoryLayer {
    type Service = PruneHistoryService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PruneHistoryService { inner, keep: self.keep, module: self.module.clone() }
    }
}

/// The service of [`PruneHistoryLayer`].
#[derive(Clone, Debug)]
pub struct PruneHistoryService<S> {
    inner: S,
    keep: Option<u64>,
    module: SharedModule,
}

impl<S> PruneHistoryService<S> {
    /// Returns anvil's error when the request reads the state of a pruned block.
    async fn pruned(&self, request: &Request<'_>) -> Option<ErrorObjectOwned> {
        let keep = self.keep?;
        let (_, position) =
            STATE_READS.iter().find(|(method, _)| *method == request.method_name())?;
        let params = request.params().parse::<Vec<Value>>().ok()?;
        let number = block_number(params.get(*position)?)?;
        let module = self.module.read().clone();
        let best: U64 = module.call("eth_blockNumber", ArrayParams::new()).await.ok()?;
        let best = best.to::<u64>();
        if number > best || number >= best.saturating_sub(keep) {
            return None;
        }
        // The blocks up to the fork block are the remote chain's, which keeps their states.
        let metadata: Value = module.call("anvil_metadata", ArrayParams::new()).await.ok()?;
        let fork_block = metadata["forkedNetwork"]["forkBlockNumber"].as_u64();
        if fork_block.is_some_and(|fork_block| number <= fork_block) {
            return None;
        }
        Some(ErrorObjectOwned::owned(
            INVALID_PARAMS_CODE,
            format!("BlockOutOfRangeError: block height is {best} but requested was {number}"),
            None::<()>,
        ))
    }
}

impl<S> RpcServiceT for PruneHistoryService<S>
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
        let this = self.clone();
        async move {
            if let Some(error) = this.pruned(&request).await {
                return MethodResponse::error(request.id, error);
            }
            this.inner.call(request).await
        }
    }

    fn batch<'a>(
        &self,
        requests: Batch<'a>,
    ) -> impl Future<Output = Self::BatchResponse> + Send + 'a {
        self.inner.batch(requests)
    }

    fn notification<'a>(
        &self,
        notification: Notification<'a>,
    ) -> impl Future<Output = Self::NotificationResponse> + Send + 'a {
        self.inner.notification(notification)
    }
}

/// Returns the number of a block parameter given by number.
fn block_number(block: &Value) -> Option<u64> {
    let number = match block {
        Value::Object(block) => block.get("blockNumber")?,
        number => number,
    };
    serde_json::from_value::<U64>(number.clone()).ok().map(|number| number.to())
}
