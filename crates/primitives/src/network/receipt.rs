use alloy_network::ReceiptResponse;
use alloy_primitives::{Address, B256, BlockHash, TxHash};
use alloy_rpc_types::TransactionReceipt;
use op_alloy_rpc_types::L1BlockInfo;
use serde::{Deserialize, Serialize};

use crate::FoundryReceiptEnvelope;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundryTxReceipt {
    /// Regular eth transaction receipt including deposit receipts
    #[serde(flatten)]
    pub inner: TransactionReceipt<FoundryReceiptEnvelope<alloy_rpc_types_eth::Log>>,
    /// L1 block info of the transaction.
    #[serde(flatten)]
    pub l1_block_info: L1BlockInfo,
}

impl ReceiptResponse for FoundryTxReceipt {
    fn contract_address(&self) -> Option<Address> {
        self.inner.contract_address
    }

    fn status(&self) -> bool {
        self.inner.inner.status()
    }

    fn block_hash(&self) -> Option<BlockHash> {
        self.inner.block_hash
    }

    fn block_number(&self) -> Option<u64> {
        self.inner.block_number
    }

    fn transaction_hash(&self) -> TxHash {
        self.inner.transaction_hash
    }

    fn transaction_index(&self) -> Option<u64> {
        self.inner.transaction_index()
    }

    fn gas_used(&self) -> u64 {
        self.inner.gas_used()
    }

    fn effective_gas_price(&self) -> u128 {
        self.inner.effective_gas_price()
    }

    fn blob_gas_used(&self) -> Option<u64> {
        self.inner.blob_gas_used()
    }

    fn blob_gas_price(&self) -> Option<u128> {
        self.inner.blob_gas_price()
    }

    fn from(&self) -> Address {
        self.inner.from()
    }

    fn to(&self) -> Option<Address> {
        self.inner.to()
    }

    fn cumulative_gas_used(&self) -> u64 {
        self.inner.cumulative_gas_used()
    }

    fn state_root(&self) -> Option<B256> {
        self.inner.state_root()
    }
}
