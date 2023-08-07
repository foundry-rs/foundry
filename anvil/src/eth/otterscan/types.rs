use ethers::types::{
    Action, Address, Block, Bytes, Call, CallType, Create, CreateResult, Res, Suicide, Trace,
    Transaction, TransactionReceipt, H256, U256,
};
use futures::future::join_all;
use serde::{de::DeserializeOwned, Serialize};

use crate::eth::{backend::mem::Backend, error::Result};

/// Patched Block struct, to include the additional `transactionCount` field expected by Otterscan
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", bound = "TX: Serialize + DeserializeOwned")]
pub struct OtsBlock<TX> {
    #[serde(flatten)]
    pub block: Block<TX>,
    pub transaction_count: usize,
}

/// Block structure with additional details regarding fees and issuance
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OtsBlockDetails {
    pub block: OtsBlock<Transaction>,
    pub total_fees: U256,
    pub issuance: Issuance,
}

/// Issuance information for a block. Expected by Otterscan in ots_getBlockDetails calls
#[derive(Debug, Serialize, Default)]
pub struct Issuance {
    block_reward: U256,
    uncle_reward: U256,
    issuance: U256,
}

/// Holds both transactions and receipts for a block
#[derive(Serialize, Debug)]
pub struct OtsBlockTransactions {
    pub fullblock: OtsBlock<Transaction>,
    pub receipts: Vec<TransactionReceipt>,
}

/// Patched Receipt struct, to include the additional `timestamp` field expected by Otterscan
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtsTransactionReceipt {
    #[serde(flatten)]
    receipt: TransactionReceipt,
    timestamp: u64,
}

/// Information about the creator address and transaction for a contract
#[derive(Serialize, Debug)]
pub struct OtsContractCreator {
    pub hash: H256,
    pub creator: Address,
}

/// Paginated search results of an account's history
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OtsSearchTransactions {
    pub txs: Vec<Transaction>,
    pub receipts: Vec<OtsTransactionReceipt>,
    pub first_page: bool,
    pub last_page: bool,
}

/// Otterscan format for listing relevant internal operations
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OtsInternalOperation {
    pub r#type: OtsInternalOperationType,
    pub from: Address,
    pub to: Address,
    pub value: U256,
}

/// Types of internal operations recognized by Otterscan
#[derive(Serialize, Debug, PartialEq)]
pub enum OtsInternalOperationType {
    Transfer = 0,
    SelfDestruct = 1,
    Create = 2,
    // The spec asks for a Create2 entry as well, but we don't have that info
}

#[derive(Serialize, Debug, PartialEq)]
pub struct OtsTrace {
    pub r#type: OtsTraceType,
    pub depth: usize,
    pub from: Address,
    pub to: Address,
    pub value: U256,
    pub input: Bytes,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum OtsTraceType {
    Call,
    StaticCall,
    DelegateCall,
}

impl OtsBlockDetails {
    pub async fn build(mut block: Block<Transaction>, backend: &Backend) -> Result<Self> {
        // TODO: avoid unwrapping
        let receipts: Vec<TransactionReceipt> = join_all(
            block
                .transactions
                .iter()
                .map(|tx| async { backend.transaction_receipt(tx.hash).await.unwrap().unwrap() }),
        )
        .await;

        let total_fees = receipts.iter().fold(U256::zero(), |acc, receipt| {
            acc + receipt.gas_used.unwrap() * (receipt.effective_gas_price.unwrap())
        });

        // Otterscan doesn't need logsBloom, so we can save some bandwidth
        // it also doesn't need transactions, but we can't really empty that, since it would cause
        // `transaction_count` to also end up as 0
        block.logs_bloom = None;

        Ok(Self {
            block: block.into(),
            total_fees,
            // issuance has no meaningful value in anvil's backend. just default to 0
            issuance: Default::default(),
        })
    }
}

impl<TX> From<Block<TX>> for OtsBlock<TX> {
    fn from(block: Block<TX>) -> Self {
        let transaction_count = block.transactions.len();

        Self { block, transaction_count }
    }
}

impl OtsBlockTransactions {
    pub async fn build(
        mut block: Block<Transaction>,
        backend: &Backend,
        page: usize,
        page_size: usize,
    ) -> Result<Self> {
        block.transactions =
            block.transactions.into_iter().skip(page * page_size).take(page_size).collect();
        // TODO: avoid unwrapping
        let receipts: Vec<TransactionReceipt> = join_all(
            block
                .transactions
                .iter()
                .map(|tx| async { backend.transaction_receipt(tx.hash).await.unwrap().unwrap() }),
        )
        .await;

        let fullblock: OtsBlock<_> = block.into();

        Ok(Self { fullblock, receipts })
    }
}

impl OtsSearchTransactions {
    pub async fn build(
        hashes: Vec<H256>,
        backend: &Backend,
        first_page: bool,
        last_page: bool,
    ) -> Result<Self> {
        let txs: Vec<Transaction> = join_all(
            hashes
                .iter()
                .map(|hash| async { backend.transaction_by_hash(*hash).await.unwrap().unwrap() }),
        )
        .await;

        let receipts: Vec<OtsTransactionReceipt> = join_all(hashes.iter().map(|hash| async {
            let receipt = backend.transaction_receipt(*hash).await.unwrap().unwrap();
            let timestamp =
                backend.get_block(receipt.block_number.unwrap()).unwrap().header.timestamp;
            OtsTransactionReceipt { receipt, timestamp }
        }))
        .await;

        Ok(Self { txs, receipts, first_page, last_page })
    }
}

impl OtsInternalOperation {
    pub fn batch_build(traces: Vec<Trace>) -> Vec<OtsInternalOperation> {
        traces
            .iter()
            .filter_map(|trace| {
                match (trace.action.clone(), trace.result.clone()) {
                    (Action::Call(Call { from, to, value, .. }), _) if !value.is_zero() => {
                        Some(Self { r#type: OtsInternalOperationType::Transfer, from, to, value })
                    }
                    (
                        Action::Create(Create { from, value, .. }),
                        Some(Res::Create(CreateResult { address, .. })),
                    ) => Some(Self {
                        r#type: OtsInternalOperationType::Create,
                        from,
                        to: address,
                        value,
                    }),
                    (Action::Suicide(Suicide { address, .. }), _) => {
                        // can we correctly assume that any suicide has a parent trace?
                        let (from, value) =
                            Self::find_suicide_caller(&traces, &trace.trace_address).unwrap();

                        Some(Self {
                            r#type: OtsInternalOperationType::SelfDestruct,
                            from,
                            to: address,
                            value,
                        })
                    }
                    _ => None,
                }
            })
            .collect()
    }

    // finds the trace that parents a given trace_address
    fn find_suicide_caller(
        traces: &Vec<Trace>,
        suicide_address: &Vec<usize>,
    ) -> Option<(Address, U256)> {
        traces.iter().find(|t| t.trace_address == suicide_address[..suicide_address.len() - 1]).map(
            |t| match t.action {
                Action::Call(Call { from, value, .. }) => (from, value),

                Action::Create(Create { from, value, .. }) => (from, value),

                // TODO can a suicide trace be parented by another suicide?
                Action::Suicide(_) => Self::find_suicide_caller(traces, &t.trace_address).unwrap(),

                Action::Reward(_) => unreachable!(),
            },
        )
    }
}

impl OtsTrace {
    pub fn batch_build(traces: Vec<Trace>) -> Vec<Self> {
        traces
            .into_iter()
            .filter_map(|trace| match trace.action {
                Action::Call(call) => {
                    if let Ok(ots_type) = call.call_type.try_into() {
                        Some(OtsTrace {
                            r#type: ots_type,
                            depth: trace.trace_address.len(),
                            from: call.from,
                            to: call.to,
                            value: call.value,
                            input: call.input,
                        })
                    } else {
                        None
                    }
                }
                Action::Create(_) => None,
                Action::Suicide(_) => None,
                Action::Reward(_) => None,
            })
            .collect()
    }
}

impl TryFrom<CallType> for OtsTraceType {
    type Error = ();

    fn try_from(value: CallType) -> std::result::Result<Self, Self::Error> {
        match value {
            CallType::Call => Ok(OtsTraceType::Call),
            CallType::StaticCall => Ok(OtsTraceType::StaticCall),
            CallType::DelegateCall => Ok(OtsTraceType::DelegateCall),
            _ => Err(()),
        }
    }
}
