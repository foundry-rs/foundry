//! The `ots_*` methods anvil answers differently from reth: the internal operations of a
//! transaction include its top-level operation, block transactions page from the first
//! transaction, and the address history search, which reth does not implement.
//!
//! The methods read through the node's own RPC module, so they get anvil's answers for the fork
//! and for every network.

use alloy_eips::eip1898::LenientBlockNumberOrTag;
use alloy_network::{AnyRpcBlock, AnyTransactionReceipt, ReceiptResponse};
use alloy_primitives::{Address, B256, U64};
use alloy_rpc_types::{
    BlockTransactions, TransactionReceipt,
    trace::{
        otterscan::{
            InternalOperation, OperationType, OtsBlock, OtsBlockTransactions, OtsReceipt,
            OtsTransactionReceipt, TransactionsWithReceipts,
        },
        parity::{Action, CallType, CreationMethod, LocalizedTransactionTrace, TraceOutput},
    },
};
use jsonrpsee::{
    MethodsError, RpcModule,
    core::{RpcResult, async_trait, params::ArrayParams},
    proc_macros::rpc,
    types::{ErrorObjectOwned, error::INTERNAL_ERROR_CODE},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::{Arc, OnceLock};

/// The `ots` methods anvil answers differently from reth.
#[rpc(server, namespace = "ots")]
pub trait AnvilOtsApi {
    /// Returns the value transfers, contract creations, and self-destructs of a transaction,
    /// its top-level operation included.
    #[method(name = "getInternalOperations")]
    async fn get_internal_operations(&self, tx_hash: B256) -> RpcResult<Vec<InternalOperation>>;

    /// Returns one page of the transactions of a block, with their receipts. Pages count from
    /// the first transaction.
    #[method(name = "getBlockTransactions")]
    async fn get_block_transactions(
        &self,
        block: LenientBlockNumberOrTag,
        page: usize,
        page_size: usize,
    ) -> RpcResult<OtsBlockTransactions<Value, Value>>;

    /// Returns a page of the transactions that touch an address, before a block, newest first.
    #[method(name = "searchTransactionsBefore")]
    async fn search_transactions_before(
        &self,
        address: Address,
        block: LenientBlockNumberOrTag,
        page_size: usize,
    ) -> RpcResult<TransactionsWithReceipts<Value>>;

    /// Returns a page of the transactions that touch an address, after a block, newest first.
    #[method(name = "searchTransactionsAfter")]
    async fn search_transactions_after(
        &self,
        address: Address,
        block: LenientBlockNumberOrTag,
        page_size: usize,
    ) -> RpcResult<TransactionsWithReceipts<Value>>;
}

/// The node's RPC module, set once the node has built it.
pub type NodeRpcModule = Arc<OnceLock<RpcModule<()>>>;

/// The `ots` methods anvil answers differently from reth.
#[derive(Clone, Debug)]
pub struct AnvilOts {
    module: NodeRpcModule,
    /// The first block the address history search reads: the first block after the fork, or
    /// after genesis.
    first_block: u64,
}

impl AnvilOts {
    /// Creates the methods over the node's RPC module.
    pub const fn new(module: NodeRpcModule, first_block: u64) -> Self {
        Self { module, first_block }
    }

    /// Calls a method of the node.
    async fn call<T: DeserializeOwned + Clone>(
        &self,
        method: &str,
        params: ArrayParams,
    ) -> RpcResult<T> {
        let module = self.module.get().ok_or_else(|| internal_error("the node is starting"))?;
        module.call(method, params).await.map_err(|error| match error {
            MethodsError::JsonRpc(error) => error,
            error => internal_error(error),
        })
    }

    /// Returns the number of the latest block.
    async fn best_number(&self) -> RpcResult<u64> {
        let number: U64 = self.call("eth_blockNumber", ArrayParams::new()).await?;
        Ok(number.to())
    }

    /// Returns the hashes of the transactions of a block that touch the address, last first.
    async fn touching(&self, address: Address, number: u64) -> RpcResult<Vec<B256>> {
        let traces: Option<Vec<LocalizedTransactionTrace>> =
            self.call("trace_block", params([json!(number)])).await?;
        let mut hashes = Vec::new();
        for trace in traces.unwrap_or_default().into_iter().rev() {
            if trace.trace.contains_address(address)
                && let Some(hash) = trace.transaction_hash
                && !hashes.contains(&hash)
            {
                hashes.push(hash);
            }
        }
        Ok(hashes)
    }

    /// Returns the transactions with the given hashes and their receipts.
    async fn with_receipts(
        &self,
        hashes: Vec<B256>,
        first_page: bool,
        last_page: bool,
    ) -> RpcResult<TransactionsWithReceipts<Value>> {
        let mut txs = Vec::with_capacity(hashes.len());
        let mut receipts = Vec::with_capacity(hashes.len());
        for hash in hashes {
            let tx: Value = self.call("eth_getTransactionByHash", params([json!(hash)])).await?;
            let receipt: Option<AnyTransactionReceipt> =
                self.call("eth_getTransactionReceipt", params([json!(hash)])).await?;
            let receipt = receipt.ok_or_else(|| internal_error("missing receipt"))?;
            let timestamp = match receipt.block_number() {
                Some(number) => {
                    let block: Option<AnyRpcBlock> = self
                        .call(
                            "eth_getBlockByNumber",
                            params([json!(U64::from(number)), false.into()]),
                        )
                        .await?;
                    block.map(|block| block.header.timestamp)
                }
                None => None,
            };
            let ty =
                tx.get("type").and_then(Value::as_str).and_then(parse_quantity).unwrap_or_default();
            txs.push(tx);
            receipts.push(ots_receipt(&receipt, ty as u8, timestamp));
        }
        Ok(TransactionsWithReceipts { txs, receipts, first_page, last_page })
    }
}

#[async_trait]
impl AnvilOtsApiServer for AnvilOts {
    async fn get_internal_operations(&self, tx_hash: B256) -> RpcResult<Vec<InternalOperation>> {
        let traces: Option<Vec<LocalizedTransactionTrace>> =
            self.call("trace_transaction", params([json!(tx_hash)])).await?;
        Ok(traces.unwrap_or_default().iter().filter_map(internal_operation).collect())
    }

    async fn get_block_transactions(
        &self,
        block: LenientBlockNumberOrTag,
        page: usize,
        page_size: usize,
    ) -> RpcResult<OtsBlockTransactions<Value, Value>> {
        let block = json!(block.into_inner());
        let full: Option<AnyRpcBlock> =
            self.call("eth_getBlockByNumber", params([block.clone(), true.into()])).await?;
        let full = full.ok_or_else(|| internal_error("block not found"))?;
        let receipts: Option<Vec<AnyTransactionReceipt>> =
            self.call("eth_getBlockReceipts", params([block])).await?;
        let receipts = receipts.unwrap_or_default();

        let timestamp = full.header.timestamp;
        let transaction_count = full.transactions.len();
        let BlockTransactions::Full(transactions) = &full.transactions else {
            return Err(internal_error("block is not full"));
        };
        let range = page.saturating_mul(page_size)..;
        let transactions: Vec<Value> = transactions
            .iter()
            .skip(range.start)
            .take(page_size)
            .map(|tx| serde_json::to_value(tx).map_err(internal_error))
            .collect::<RpcResult<_>>()?;
        let receipts = receipts
            .iter()
            .skip(range.start)
            .take(page_size)
            .zip(&transactions)
            .map(|(receipt, tx)| {
                let ty = tx.get("type").and_then(Value::as_str).and_then(parse_quantity);
                ots_receipt(receipt, ty.unwrap_or_default() as u8, Some(timestamp))
            })
            .collect();

        let header = serde_json::to_value(&full.header).map_err(internal_error)?;
        let block = alloy_rpc_types::Block {
            header,
            uncles: full.uncles.clone(),
            transactions: BlockTransactions::Full(transactions),
            withdrawals: full.withdrawals.clone(),
        };
        Ok(OtsBlockTransactions { fullblock: OtsBlock { block, transaction_count }, receipts })
    }

    async fn search_transactions_before(
        &self,
        address: Address,
        block: LenientBlockNumberOrTag,
        page_size: usize,
    ) -> RpcResult<TransactionsWithReceipts<Value>> {
        let best = self.best_number().await?;
        let number = block.into_inner().as_number().unwrap_or_default();
        // From the given block, the latest by default, down to the first block.
        let from = if number == 0 { best } else { number - 1 };
        let first_page = from >= best;
        let mut last_page = false;
        let mut hashes = Vec::new();
        for number in (self.first_block..=from).rev() {
            if hashes.len() >= page_size {
                break;
            }
            hashes.extend(self.touching(address, number).await?);
            if number == self.first_block {
                last_page = true;
            }
        }
        self.with_receipts(hashes, first_page, last_page).await
    }

    async fn search_transactions_after(
        &self,
        address: Address,
        block: LenientBlockNumberOrTag,
        page_size: usize,
    ) -> RpcResult<TransactionsWithReceipts<Value>> {
        let best = self.best_number().await?;
        let number = block.into_inner().as_number().unwrap_or_default();
        // From the given block, the first block by default, up to the latest block.
        let from = if number == 0 { self.first_block } else { number + 1 };
        let mut first_page = from >= best;
        let mut last_page = false;
        let mut hashes = Vec::new();
        for number in from..=best {
            if number == self.first_block {
                last_page = true;
            }
            if hashes.len() >= page_size {
                break;
            }
            hashes.extend(self.touching(address, number).await?);
            if number == best {
                first_page = true;
            }
        }
        // The results are newest first, as Otterscan expects.
        hashes.reverse();
        self.with_receipts(hashes, first_page, last_page).await
    }
}

/// Returns the internal operation of a trace: a value transfer, a contract creation, or a
/// self-destruct.
fn internal_operation(trace: &LocalizedTransactionTrace) -> Option<InternalOperation> {
    match &trace.trace.action {
        Action::Call(call) if call.call_type == CallType::Call && !call.value.is_zero() => {
            Some(InternalOperation {
                r#type: OperationType::OpTransfer,
                from: call.from,
                to: call.to,
                value: call.value,
            })
        }
        Action::Create(create) => Some(InternalOperation {
            r#type: match create.creation_method {
                CreationMethod::Create2 => OperationType::OpCreate2,
                _ => OperationType::OpCreate,
            },
            from: create.from,
            to: match &trace.trace.result {
                Some(TraceOutput::Create(output)) => output.address,
                _ => Address::ZERO,
            },
            value: create.value,
        }),
        Action::Selfdestruct(selfdestruct) => Some(InternalOperation {
            r#type: OperationType::OpSelfDestruct,
            from: selfdestruct.address,
            to: selfdestruct.refund_address,
            value: selfdestruct.balance,
        }),
        _ => None,
    }
}

/// Converts a receipt into Otterscan's receipt, without logs.
fn ots_receipt(
    receipt: &AnyTransactionReceipt,
    ty: u8,
    timestamp: Option<u64>,
) -> OtsTransactionReceipt {
    let inner = OtsReceipt {
        status: receipt.status(),
        cumulative_gas_used: receipt.cumulative_gas_used(),
        logs: None,
        logs_bloom: None,
        r#type: ty,
    };
    let receipt = TransactionReceipt {
        inner,
        transaction_hash: receipt.transaction_hash(),
        transaction_index: receipt.transaction_index(),
        block_hash: receipt.block_hash(),
        block_number: receipt.block_number(),
        gas_used: receipt.gas_used(),
        effective_gas_price: receipt.effective_gas_price(),
        blob_gas_used: receipt.blob_gas_used(),
        blob_gas_price: receipt.blob_gas_price(),
        from: receipt.from(),
        to: receipt.to(),
        contract_address: receipt.contract_address(),
    };
    OtsTransactionReceipt { receipt, timestamp }
}

/// Builds positional parameters.
fn params<const N: usize>(values: [Value; N]) -> ArrayParams {
    let mut params = ArrayParams::new();
    for value in values {
        params.insert(value).expect("a JSON value serializes");
    }
    params
}

/// Parses a hex quantity.
fn parse_quantity(value: &str) -> Option<u64> {
    u64::from_str_radix(value.trim_start_matches("0x"), 16).ok()
}

/// An internal JSON-RPC error.
fn internal_error(message: impl std::fmt::Display) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, message.to_string(), None::<()>)
}
