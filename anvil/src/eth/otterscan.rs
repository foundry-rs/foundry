use ethers::types::{
    Action, Address, Block, Call, Create, CreateResult, Res, Suicide, Trace, Transaction,
    TransactionReceipt, H256, U256,
};
use futures::future::join_all;
use serde::{de::DeserializeOwned, Serialize};

use super::{backend::mem::Backend, error::Result};

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
#[serde(rename_all = "camelCase", bound = "TX: Serialize + DeserializeOwned")]
pub struct OtsBlockDetails<TX> {
    block: OtsBlock<TX>,
    total_fees: U256,
    issuance: Issuance,
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
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OtsInternalOperation {
    r#type: OtsInternalOperationType,
    from: Address,
    to: Address,
    value: U256,
}

/// Types of internal operations recognized by Otterscan
#[derive(Serialize, Debug)]
pub enum OtsInternalOperationType {
    Transfer = 0,
    SelfDestruct = 1,
    Create = 2,
    // The spec asks for a Create2 entry as well, but we don't have that info
}

impl<TX> From<Block<TX>> for OtsBlockDetails<TX> {
    fn from(block: Block<TX>) -> Self {
        Self {
            block: block.into(),
            total_fees: U256::zero(),     // TODO:
            issuance: Default::default(), // TODO: fill block_reward
        }
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
            .map(|trace| {
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
            .flatten()
            .collect()
    }

    // finds the trace that parents a given trace_address
    fn find_suicide_caller(
        traces: &Vec<Trace>,
        suicide_address: &Vec<usize>,
    ) -> Option<(Address, U256)> {
        traces
            .iter()
            .find(|t| t.trace_address == &suicide_address[..suicide_address.len() - 1])
            .map(|t| match t.action {
                Action::Call(Call { from, value, .. }) => (from, value),

                Action::Create(Create { from, value, .. }) => (from, value),

                // TODO can a suicide trace be parented by another suicide?
                Action::Suicide(_) => Self::find_suicide_caller(traces, &t.trace_address).unwrap(),

                Action::Reward(_) => unreachable!(),
            })
    }
}
