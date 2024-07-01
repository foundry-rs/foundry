use crate::eth::{
    backend::mem::{storage::MinedTransaction, Backend},
    error::{BlockchainError, Result},
};
use alloy_primitives::{Address, Bytes, FixedBytes, B256, U256};
use alloy_rpc_types::{
    trace::parity::{
        Action, CallAction, CallType, CreateAction, CreateOutput, LocalizedTransactionTrace,
        RewardAction, TraceOutput,
    },
    Block, BlockTransactions, Transaction,
};
use alloy_serde::WithOtherFields;
use anvil_core::eth::transaction::ReceiptResponse;
use foundry_evm::traces::CallKind;
use futures::future::join_all;
use serde::Serialize;
use serde_repr::Serialize_repr;

/// Patched Block struct, to include the additional `transactionCount` field expected by Otterscan
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtsBlock {
    #[serde(flatten)]
    pub block: Block,
    pub transaction_count: usize,
}

/// Block structure with additional details regarding fees and issuance
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtsBlockDetails {
    pub block: OtsBlock,
    pub total_fees: U256,
    pub issuance: Issuance,
}

/// Issuance information for a block. Expected by Otterscan in ots_getBlockDetails calls
#[derive(Debug, Default, Serialize)]
pub struct Issuance {
    block_reward: U256,
    uncle_reward: U256,
    issuance: U256,
}

/// Holds both transactions and receipts for a block
#[derive(Clone, Serialize, Debug)]
pub struct OtsBlockTransactions {
    pub fullblock: OtsBlock,
    pub receipts: Vec<ReceiptResponse>,
}

/// Patched Receipt struct, to include the additional `timestamp` field expected by Otterscan
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtsTransactionReceipt {
    #[serde(flatten)]
    receipt: ReceiptResponse,
    timestamp: u64,
}

/// Information about the creator address and transaction for a contract
#[derive(Debug, Serialize)]
pub struct OtsContractCreator {
    pub hash: B256,
    pub creator: Address,
}

/// Paginated search results of an account's history
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtsSearchTransactions {
    pub txs: Vec<WithOtherFields<Transaction>>,
    pub receipts: Vec<OtsTransactionReceipt>,
    pub first_page: bool,
    pub last_page: bool,
}

/// Otterscan format for listing relevant internal operations.
///
/// Ref: <https://github.com/otterscan/otterscan/blob/5adf4e3eead05eddb7746ee45b689161aaea7a7a/src/types.ts#L98>
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtsInternalOperation {
    pub r#type: OtsInternalOperationType,
    pub from: Address,
    pub to: Address,
    pub value: U256,
}

/// Types of internal operations recognized by Otterscan.
///
/// Ref: <https://github.com/otterscan/otterscan/blob/5adf4e3eead05eddb7746ee45b689161aaea7a7a/src/types.ts#L91>
#[derive(Debug, PartialEq, Eq, Serialize_repr)]
#[repr(u8)]
pub enum OtsInternalOperationType {
    Transfer = 0,
    SelfDestruct = 1,
    Create = 2,
    Create2 = 3,
}

/// Otterscan's representation of a trace
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct OtsTrace {
    pub r#type: OtsTraceType,
    pub depth: usize,
    pub from: Address,
    pub to: Address,
    pub value: U256,
    pub input: Bytes,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Bytes>,
}

/// The type of call being described by an Otterscan trace. Only CALL, STATICCALL and DELEGATECALL
/// are represented
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OtsTraceType {
    Call,
    StaticCall,
    DelegateCall,
}

impl OtsBlockDetails {
    /// The response for ots_getBlockDetails includes an `issuance` object that requires computing
    /// the total gas spent in a given block.
    ///
    /// The only way to do this with the existing API is to explicitly fetch all receipts, to get
    /// their `gas_used`. This would be extremely inefficient in a real blockchain RPC, but we can
    /// get away with that in this context.
    ///
    /// The [original spec](https://github.com/otterscan/otterscan/blob/develop/docs/custom-jsonrpc.md#ots_getblockdetails)
    /// also mentions we can hardcode `transactions` and `logsBloom` to an empty array to save
    /// bandwidth, because fields weren't intended to be used in the Otterscan UI at this point.
    ///
    /// This has two problems though:
    ///   - It makes the endpoint too specific to Otterscan's implementation
    ///   - It breaks the abstraction built in `OtsBlock<TX>` which computes `transaction_count`
    ///     based on the existing list.
    ///
    /// Therefore we keep it simple by keeping the data in the response
    pub async fn build(block: Block, backend: &Backend) -> Result<Self> {
        if block.transactions.is_uncle() {
            return Err(BlockchainError::DataUnavailable);
        }
        let receipts_futs = block
            .transactions
            .hashes()
            .map(|hash| async { backend.transaction_receipt(*hash).await });

        // fetch all receipts
        let receipts = join_all(receipts_futs)
            .await
            .into_iter()
            .map(|r| match r {
                Ok(Some(r)) => Ok(r),
                _ => Err(BlockchainError::DataUnavailable),
            })
            .collect::<Result<Vec<_>>>()?;

        let total_fees = receipts
            .iter()
            .fold(0, |acc, receipt| acc + receipt.gas_used * receipt.effective_gas_price);

        Ok(Self {
            block: block.into(),
            total_fees: U256::from(total_fees),
            // issuance has no meaningful value in anvil's backend. just default to 0
            issuance: Default::default(),
        })
    }
}

/// Converts a regular block into the patched OtsBlock format
/// which includes the `transaction_count` field
impl From<Block> for OtsBlock {
    fn from(block: Block) -> Self {
        Self { transaction_count: block.transactions.len(), block }
    }
}

impl OtsBlockTransactions {
    /// Fetches all receipts for the blocks's transactions, as required by the
    /// [`ots_getBlockTransactions`] endpoint spec, and returns the final response object.
    ///
    /// [`ots_getBlockTransactions`]: https://github.com/otterscan/otterscan/blob/develop/docs/custom-jsonrpc.md#ots_getblockdetails
    pub async fn build(
        mut block: Block,
        backend: &Backend,
        page: usize,
        page_size: usize,
    ) -> Result<Self> {
        if block.transactions.is_uncle() {
            return Err(BlockchainError::DataUnavailable);
        }

        block.transactions = match block.transactions {
            BlockTransactions::Full(txs) => BlockTransactions::Full(
                txs.into_iter().skip(page * page_size).take(page_size).collect(),
            ),
            BlockTransactions::Hashes(txs) => BlockTransactions::Hashes(
                txs.into_iter().skip(page * page_size).take(page_size).collect(),
            ),
            BlockTransactions::Uncle => unreachable!(),
        };

        let receipt_futs =
            block.transactions.hashes().map(|hash| backend.transaction_receipt(*hash));

        let receipts = join_all(receipt_futs)
            .await
            .into_iter()
            .map(|r| match r {
                Ok(Some(r)) => Ok(r),
                _ => Err(BlockchainError::DataUnavailable),
            })
            .collect::<Result<_>>()?;

        let fullblock: OtsBlock = block.into();

        Ok(Self { fullblock, receipts })
    }
}

impl OtsSearchTransactions {
    /// Constructs the final response object for both [`ots_searchTransactionsBefore` and
    /// `ots_searchTransactionsAfter`](lrequires not only the transactions, but also the
    /// corresponding receipts, which are fetched here before constructing the final)
    pub async fn build(
        hashes: Vec<B256>,
        backend: &Backend,
        first_page: bool,
        last_page: bool,
    ) -> Result<Self> {
        let txs_futs = hashes.iter().map(|hash| async { backend.transaction_by_hash(*hash).await });

        let txs: Vec<_> = join_all(txs_futs)
            .await
            .into_iter()
            .map(|t| match t {
                Ok(Some(t)) => Ok(t),
                _ => Err(BlockchainError::DataUnavailable),
            })
            .collect::<Result<_>>()?;

        join_all(hashes.iter().map(|hash| async {
            match backend.transaction_receipt(*hash).await {
                Ok(Some(receipt)) => {
                    let timestamp =
                        backend.get_block(receipt.block_number.unwrap()).unwrap().header.timestamp;
                    Ok(OtsTransactionReceipt { receipt, timestamp })
                }
                Ok(None) => Err(BlockchainError::DataUnavailable),
                Err(e) => Err(e),
            }
        }))
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()
        .map(|receipts| Self { txs, receipts, first_page, last_page })
    }

    pub fn mentions_address(
        trace: LocalizedTransactionTrace,
        address: Address,
    ) -> Option<FixedBytes<32>> {
        match (trace.trace.action, trace.trace.result) {
            (Action::Call(CallAction { from, to, .. }), _) if from == address || to == address => {
                trace.transaction_hash
            }
            (_, Some(TraceOutput::Create(CreateOutput { address: created_address, .. })))
                if created_address == address =>
            {
                trace.transaction_hash
            }
            (Action::Create(CreateAction { from, .. }), _) if from == address => {
                trace.transaction_hash
            }
            (Action::Reward(RewardAction { author, .. }), _) if author == address => {
                trace.transaction_hash
            }
            _ => None,
        }
    }
}

impl OtsInternalOperation {
    /// Converts a batch of traces into a batch of internal operations, to comply with the spec for
    /// [`ots_getInternalOperations`](https://github.com/otterscan/otterscan/blob/develop/docs/custom-jsonrpc.md#ots_getinternaloperations)
    pub fn batch_build(traces: MinedTransaction) -> Vec<Self> {
        traces
            .info
            .traces
            .iter()
            .filter_map(|node| {
                let r#type = match node.trace.kind {
                    _ if node.is_selfdestruct() => OtsInternalOperationType::SelfDestruct,
                    CallKind::Call if !node.trace.value.is_zero() => {
                        OtsInternalOperationType::Transfer
                    }
                    CallKind::Create => OtsInternalOperationType::Create,
                    CallKind::Create2 => OtsInternalOperationType::Create2,
                    _ => return None,
                };
                let mut from = node.trace.caller;
                let mut to = node.trace.address;
                let mut value = node.trace.value;
                if node.is_selfdestruct() {
                    from = node.trace.address;
                    to = node.trace.selfdestruct_refund_target.unwrap_or_default();
                    value = node.trace.selfdestruct_transferred_value.unwrap_or_default();
                }
                Some(Self { r#type, from, to, value })
            })
            .collect()
    }
}

impl OtsTrace {
    /// Converts the list of traces for a transaction into the expected Otterscan format, as
    /// specified in the [`ots_traceTransaction`](https://github.com/otterscan/otterscan/blob/develop/docs/custom-jsonrpc.md#ots_tracetransaction) spec
    pub fn batch_build(traces: Vec<LocalizedTransactionTrace>) -> Vec<Self> {
        traces
            .into_iter()
            .filter_map(|trace| match trace.trace.action {
                Action::Call(call) => {
                    if let Ok(ots_type) = call.call_type.try_into() {
                        Some(Self {
                            r#type: ots_type,
                            depth: trace.trace.trace_address.len(),
                            from: call.from,
                            to: call.to,
                            value: call.value,
                            input: call.input.0.into(),
                            output: None,
                        })
                    } else {
                        None
                    }
                }
                Action::Create(_) | Action::Selfdestruct(_) | Action::Reward(_) => None,
            })
            .collect()
    }
}

impl TryFrom<CallType> for OtsTraceType {
    type Error = ();

    fn try_from(value: CallType) -> std::result::Result<Self, Self::Error> {
        match value {
            CallType::Call => Ok(Self::Call),
            CallType::StaticCall => Ok(Self::StaticCall),
            CallType::DelegateCall => Ok(Self::DelegateCall),
            _ => Err(()),
        }
    }
}
