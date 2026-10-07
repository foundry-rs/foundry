//! The `debug_*` and `trace_*` methods anvil answers differently from reth.
//!
//! The fork block and the blocks before it are not stored locally, so their traces come from the
//! fork endpoint, as on anvil. The other differences are error codes and parameter checks.

use crate::fork::ForkInfo;
use alloy_consensus::BlockHeader;
use alloy_eips::BlockId;
use alloy_primitives::{Address, B256, Bytes, map::HashSet};
use alloy_rpc_types::trace::{
    filter::TraceFilter,
    geth::{GethDebugTracingOptions, GethTrace},
    opcode::BlockOpcodeGas,
    parity::{LocalizedTransactionTrace, TraceResults, TraceResultsWithTransactionHash, TraceType},
};
use alloy_rpc_types_eth::{AccountInfo, Index};
use jsonrpsee::{
    core::{RpcResult, async_trait},
    proc_macros::rpc,
    types::{ErrorObjectOwned, error::INVALID_PARAMS_CODE},
};
use reth_ethereum::{
    PooledTransactionVariant,
    rpc::{
        DebugApi, TraceApi,
        api::{DebugApiServer, TraceApiServer},
        eth::{EthApiError, TransactionSource, utils::recover_raw_transaction},
    },
    storage::StateProvider,
};
use reth_rpc_eth_api::{
    RpcNodeCore, RpcTxReq,
    helpers::{EthTransactions, LoadBlock, LoadTransaction, TraceExt},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{fmt, sync::Arc};

/// The error code anvil reports for tracing an unknown transaction.
const TRANSACTION_NOT_FOUND_CODE: i32 = -32001;

/// The error code anvil reports for a raw transaction from a sender with code.
const SENDER_NOT_EOA_CODE: i32 = -32003;

/// The error code anvil reports for a replay without the state of the parent block.
const HISTORICAL_STATE_CODE: i32 = -32000;

/// The `debug_*` methods with anvil's error codes and fork routing.
#[rpc(server, namespace = "debug")]
pub trait AnvilDebugApi {
    /// Traces the transaction with the given hash. An unknown hash is error `-32001`, as in anvil.
    #[method(name = "traceTransaction")]
    async fn debug_trace_transaction(
        &self,
        hash: B256,
        opts: Option<GethDebugTracingOptions>,
    ) -> RpcResult<GethTrace>;

    /// Returns the account after the transaction at the given index of the block. The fork
    /// endpoint answers for the fork block and the blocks before it.
    #[method(name = "accountInfoAt")]
    async fn debug_account_info_at(
        &self,
        block: BlockId,
        index: Index,
        address: Address,
    ) -> RpcResult<Option<AccountInfo>>;
}

/// The `trace_*` methods with anvil's handling of the pending block, the fork, and the
/// `trace_get` indices.
#[rpc(server, namespace = "trace")]
pub trait AnvilTraceApi {
    /// Returns the traces of the block's transactions. The pending block is rejected, as in anvil.
    #[method(name = "block")]
    async fn trace_block(
        &self,
        block: BlockId,
    ) -> RpcResult<Option<Vec<LocalizedTransactionTrace>>>;

    /// Replays the block's transactions with the given trace types. The pending block is
    /// rejected, as in anvil.
    #[method(name = "replayBlockTransactions")]
    async fn trace_replay_block_transactions(
        &self,
        block: BlockId,
        trace_types: HashSet<TraceType>,
    ) -> RpcResult<Option<Vec<TraceResultsWithTransactionHash>>>;

    /// Returns the opcode gas of the block's transactions.
    #[method(name = "blockOpcodeGas")]
    async fn trace_block_opcode_gas(&self, block: BlockId) -> RpcResult<Option<BlockOpcodeGas>>;

    /// Returns the traces of the transaction, from the fork endpoint when the local chain does
    /// not know the transaction.
    #[method(name = "transaction")]
    async fn trace_transaction(
        &self,
        hash: B256,
    ) -> RpcResult<Option<Vec<LocalizedTransactionTrace>>>;

    /// Returns the trace at the given trace address. The indices are quantity strings, as in
    /// anvil; integers are rejected.
    #[method(name = "get")]
    async fn trace_get(
        &self,
        hash: B256,
        indices: Vec<Value>,
    ) -> RpcResult<Option<LocalizedTransactionTrace>>;

    /// Returns the traces matching the filter, with the fork endpoint's traces for the blocks it
    /// serves.
    #[method(name = "filter")]
    async fn trace_filter(&self, filter: TraceFilter) -> RpcResult<Vec<LocalizedTransactionTrace>>;

    /// Traces a signed transaction. A sender with code is rejected, as in anvil; an EIP-7702
    /// delegation keeps the sender an EOA.
    #[method(name = "rawTransaction")]
    async fn trace_raw_transaction(
        &self,
        data: Bytes,
        trace_types: HashSet<TraceType>,
        block: Option<BlockId>,
    ) -> RpcResult<TraceResults>;
}

/// Rejects the pending block, which has no traces.
fn ensure_mined(block: BlockId) -> RpcResult<()> {
    if block.is_pending() {
        return Err(ErrorObjectOwned::owned(
            INVALID_PARAMS_CODE,
            "the pending block cannot be traced",
            None::<()>,
        ));
    }
    Ok(())
}

/// Where the traces of a block come from.
enum Route {
    /// The fork endpoint serves the block, under this id.
    Remote(BlockId),
    /// The node replays the block, with this number when it knows the block.
    Local(Option<u64>),
}

/// Routes a block to the fork endpoint when it serves the block, which the local chain does not
/// store. A hash keeps selecting the endpoint's block.
async fn route<Eth: LoadBlock>(
    eth: &Eth,
    fork: Option<&Arc<dyn ForkInfo>>,
    block: BlockId,
) -> RpcResult<Route> {
    let Some(fork) = fork else { return Ok(Route::Local(None)) };
    let Some(base) = eth.recovered_block(block).await.map_err(Into::into)? else {
        return Ok(match block {
            BlockId::Hash(_) if fork.has_remote() => Route::Remote(block),
            _ => Route::Local(None),
        });
    };
    let number = base.header().number();
    if number > fork.block_number() || !fork.remote_serves(number) {
        return Ok(Route::Local(Some(number)));
    }
    Ok(Route::Remote(match block {
        BlockId::Hash(hash) => BlockId::Hash(hash),
        _ => BlockId::number(number),
    }))
}

/// Returns the block id to ask the fork endpoint for, when it serves the block.
async fn remote_block<Eth: LoadBlock>(
    eth: &Eth,
    fork: Option<&Arc<dyn ForkInfo>>,
    block: BlockId,
) -> RpcResult<Option<BlockId>> {
    Ok(match route(eth, fork, block).await? {
        Route::Remote(id) => Some(id),
        Route::Local(_) => None,
    })
}

/// Rejects the replay of a block whose parent state the node does not have: a state dump
/// restored without its historical states, as anvil reports it.
fn ensure_replayable(fork: Option<&Arc<dyn ForkInfo>>, number: u64) -> RpcResult<()> {
    if let Some(fork) = fork
        && let Some(parent) = number.checked_sub(1)
        && !fork.has_state_at(parent)
    {
        return Err(ErrorObjectOwned::owned(
            HISTORICAL_STATE_CODE,
            format!("historical state needed to replay block {number} is not available"),
            None::<()>,
        ));
    }
    Ok(())
}

/// Rejects the replay of a mined transaction whose parent block state the node does not have.
async fn ensure_transaction_replayable<Eth: LoadTransaction>(
    eth: &Eth,
    fork: Option<&Arc<dyn ForkInfo>>,
    hash: B256,
) -> RpcResult<()> {
    if let Some(TransactionSource::Block { block_number, .. }) =
        LoadTransaction::transaction_by_hash(eth, hash).await.map_err(Into::into)?
    {
        ensure_replayable(fork, block_number)?;
    }
    Ok(())
}

/// Reth's `debug` API with anvil's error codes and fork routing.
#[derive(Clone)]
pub struct AnvilDebugApi<Eth: RpcNodeCore> {
    inner: DebugApi<Eth>,
    fork: Option<Arc<dyn ForkInfo>>,
}

impl<Eth: RpcNodeCore> AnvilDebugApi<Eth> {
    /// Wraps reth's `debug` API.
    pub const fn new(inner: DebugApi<Eth>, fork: Option<Arc<dyn ForkInfo>>) -> Self {
        Self { inner, fork }
    }
}

impl<Eth: RpcNodeCore> fmt::Debug for AnvilDebugApi<Eth> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilDebugApi").field("fork", &self.fork).finish_non_exhaustive()
    }
}

#[async_trait]
impl<Eth> AnvilDebugApiServer for AnvilDebugApi<Eth>
where
    Eth: EthTransactions + TraceExt + 'static,
{
    async fn debug_trace_transaction(
        &self,
        hash: B256,
        opts: Option<GethDebugTracingOptions>,
    ) -> RpcResult<GethTrace> {
        ensure_transaction_replayable(self.inner.eth_api(), self.fork.as_ref(), hash).await?;
        <DebugApi<Eth> as DebugApiServer<RpcTxReq<Eth::NetworkTypes>>>::debug_trace_transaction(
            &self.inner,
            hash,
            opts,
        )
        .await
        .map_err(|error| {
            if error.message() == "transaction not found" {
                ErrorObjectOwned::owned(TRANSACTION_NOT_FOUND_CODE, error.message(), None::<()>)
            } else {
                error
            }
        })
    }

    async fn debug_account_info_at(
        &self,
        block: BlockId,
        index: Index,
        address: Address,
    ) -> RpcResult<Option<AccountInfo>> {
        if let Some(remote) = remote_block(self.inner.eth_api(), self.fork.as_ref(), block).await?
            && let Some(fork) = &self.fork
        {
            return fork.forward_json("debug_accountInfoAt", json!([remote, index, address]));
        }
        <DebugApi<Eth> as DebugApiServer<RpcTxReq<Eth::NetworkTypes>>>::debug_account_info_at(
            &self.inner,
            block,
            index,
            address,
        )
        .await
    }
}

/// Reth's `trace` API with anvil's handling of the pending block, the fork, and the `trace_get`
/// indices.
#[derive(Clone)]
pub struct AnvilTraceApi<Eth> {
    inner: TraceApi<Eth>,
    fork: Option<Arc<dyn ForkInfo>>,
}

impl<Eth> AnvilTraceApi<Eth> {
    /// Wraps reth's `trace` API.
    pub const fn new(inner: TraceApi<Eth>, fork: Option<Arc<dyn ForkInfo>>) -> Self {
        Self { inner, fork }
    }
}

impl<Eth> fmt::Debug for AnvilTraceApi<Eth> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilTraceApi").field("fork", &self.fork).finish_non_exhaustive()
    }
}

impl<Eth: TraceExt + 'static> AnvilTraceApi<Eth> {
    /// Returns the fork endpoint's answer when it serves the block.
    async fn forwarded<T: DeserializeOwned>(
        &self,
        method: &str,
        block: BlockId,
        params: impl FnOnce(BlockId) -> Value,
    ) -> RpcResult<Option<T>> {
        ensure_mined(block)?;
        match route(self.inner.eth_api(), self.fork.as_ref(), block).await? {
            Route::Remote(remote) => {
                let fork = self.fork.as_ref().expect("a remote route has a fork");
                fork.forward_json(method, params(remote)).map(Some)
            }
            Route::Local(Some(number)) => {
                ensure_replayable(self.fork.as_ref(), number)?;
                Ok(None)
            }
            Route::Local(None) => Ok(None),
        }
    }

    /// Reth's answer to `trace_filter`.
    async fn local_filter(&self, filter: TraceFilter) -> RpcResult<Vec<LocalizedTransactionTrace>> {
        <TraceApi<Eth> as TraceApiServer<RpcTxReq<Eth::NetworkTypes>>>::trace_filter(
            &self.inner,
            filter,
        )
        .await
    }
}

#[async_trait]
impl<Eth> AnvilTraceApiServer for AnvilTraceApi<Eth>
where
    Eth: TraceExt + 'static,
{
    async fn trace_block(
        &self,
        block: BlockId,
    ) -> RpcResult<Option<Vec<LocalizedTransactionTrace>>> {
        if let Some(traces) = self.forwarded("trace_block", block, |id| json!([id])).await? {
            return Ok(traces);
        }
        <TraceApi<Eth> as TraceApiServer<RpcTxReq<Eth::NetworkTypes>>>::trace_block(
            &self.inner,
            block,
        )
        .await
    }

    async fn trace_replay_block_transactions(
        &self,
        block: BlockId,
        trace_types: HashSet<TraceType>,
    ) -> RpcResult<Option<Vec<TraceResultsWithTransactionHash>>> {
        if let Some(traces) = self
            .forwarded("trace_replayBlockTransactions", block, |id| json!([id, trace_types]))
            .await?
        {
            return Ok(traces);
        }
        <TraceApi<Eth> as TraceApiServer<RpcTxReq<Eth::NetworkTypes>>>::replay_block_transactions(
            &self.inner,
            block,
            trace_types,
        )
        .await
    }

    async fn trace_block_opcode_gas(&self, block: BlockId) -> RpcResult<Option<BlockOpcodeGas>> {
        if let Some(gas) = self.forwarded("trace_blockOpcodeGas", block, |id| json!([id])).await? {
            return Ok(gas);
        }
        <TraceApi<Eth> as TraceApiServer<RpcTxReq<Eth::NetworkTypes>>>::trace_block_opcode_gas(
            &self.inner,
            block,
        )
        .await
    }

    async fn trace_transaction(
        &self,
        hash: B256,
    ) -> RpcResult<Option<Vec<LocalizedTransactionTrace>>> {
        ensure_transaction_replayable(self.inner.eth_api(), self.fork.as_ref(), hash).await?;
        let traces =
            <TraceApi<Eth> as TraceApiServer<RpcTxReq<Eth::NetworkTypes>>>::trace_transaction(
                &self.inner,
                hash,
            )
            .await?;
        match (&traces, &self.fork) {
            (None, Some(fork)) => fork.forward_json("trace_transaction", json!([hash])),
            _ => Ok(traces),
        }
    }

    async fn trace_get(
        &self,
        hash: B256,
        indices: Vec<Value>,
    ) -> RpcResult<Option<LocalizedTransactionTrace>> {
        let indices = indices
            .into_iter()
            .map(|index| {
                if !index.is_string() {
                    return Err(ErrorObjectOwned::owned(
                        INVALID_PARAMS_CODE,
                        "trace indices must be quantity strings",
                        None::<()>,
                    ));
                }
                serde_json::from_value::<Index>(index).map_err(|error| {
                    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, error.to_string(), None::<()>)
                })
            })
            .collect::<RpcResult<Vec<_>>>()?;
        ensure_transaction_replayable(self.inner.eth_api(), self.fork.as_ref(), hash).await?;
        <TraceApi<Eth> as TraceApiServer<RpcTxReq<Eth::NetworkTypes>>>::trace_get(
            &self.inner,
            hash,
            indices,
        )
        .await
    }

    async fn trace_filter(&self, filter: TraceFilter) -> RpcResult<Vec<LocalizedTransactionTrace>> {
        // A range without a start begins at the latest block, which is local.
        let (Some(fork), Some(from)) = (&self.fork, filter.from_block) else {
            return self.local_filter(filter).await;
        };
        if from > fork.block_number() {
            return self.local_filter(filter).await;
        }
        let TraceFilter { to_block, after, count, .. } = filter;
        let remote = TraceFilter {
            to_block: Some(to_block.map_or(fork.block_number(), |to| to.min(fork.block_number()))),
            after: None,
            count: None,
            ..filter.clone()
        };
        let mut traces: Vec<LocalizedTransactionTrace> =
            fork.forward_json("trace_filter", json!([remote]))?;
        if to_block.is_none_or(|to| to > fork.block_number()) {
            let local = TraceFilter {
                from_block: Some(fork.block_number() + 1),
                after: None,
                count: None,
                ..filter
            };
            traces.extend(self.local_filter(local).await?);
        }
        let skip = after.unwrap_or_default() as usize;
        let take = count.map_or(usize::MAX, |count| count as usize);
        Ok(traces.into_iter().skip(skip).take(take).collect())
    }

    async fn trace_raw_transaction(
        &self,
        data: Bytes,
        trace_types: HashSet<TraceType>,
        block: Option<BlockId>,
    ) -> RpcResult<TraceResults> {
        // EIP-3607 stays off for calls and mining, but a signed transaction has a real sender.
        let sender = recover_raw_transaction::<PooledTransactionVariant>(&data)
            .map_err(ErrorObjectOwned::from)?
            .signer();
        let state = self
            .inner
            .eth_api()
            .state_at_block_id(block.unwrap_or_default())
            .await
            .map_err(Into::into)?;
        let code = state
            .account_code(&sender)
            .map_err(EthApiError::from)
            .map_err(ErrorObjectOwned::from)?
            .map(|code| code.original_bytes())
            .unwrap_or_default();
        if !code.is_empty() && !code.starts_with(&[0xef, 0x01, 0x00]) {
            return Err(ErrorObjectOwned::owned(
                SENDER_NOT_EOA_CODE,
                "sender not an eoa",
                None::<()>,
            ));
        }
        <TraceApi<Eth> as TraceApiServer<RpcTxReq<Eth::NetworkTypes>>>::trace_raw_transaction(
            &self.inner,
            data,
            trace_types,
            block,
        )
        .await
    }
}
