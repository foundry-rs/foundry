//! Helper trait and functions to format Ethereum types.

use alloy_consensus::{AnyReceiptEnvelope, Eip658Value, Receipt, ReceiptWithBloom, TxType};
use alloy_primitives::{hex, Address, Bloom, Bytes, FixedBytes, Uint, B256, I256, U256, U64};
use alloy_rpc_types::{
    AccessListItem, AnyTransactionReceipt, Block, BlockTransactions, Log, Transaction,
    TransactionReceipt,
};
use alloy_serde::OtherFields;
use serde::Deserialize;

/// length of the name column for pretty formatting `{:>20}{value}`
const NAME_COLUMN_LEN: usize = 20usize;

/// Helper trait to format Ethereum types.
///
/// # Examples
///
/// ```
/// use foundry_common_fmt::UIfmt;
///
/// let boolean: bool = true;
/// let string = boolean.pretty();
/// ```
pub trait UIfmt {
    /// Return a prettified string version of the value
    fn pretty(&self) -> String;
}

impl<T: UIfmt> UIfmt for &T {
    fn pretty(&self) -> String {
        (*self).pretty()
    }
}

impl<T: UIfmt> UIfmt for Option<T> {
    fn pretty(&self) -> String {
        if let Some(ref inner) = self {
            inner.pretty()
        } else {
            String::new()
        }
    }
}

impl<T: UIfmt> UIfmt for [T] {
    fn pretty(&self) -> String {
        if !self.is_empty() {
            let mut s = String::with_capacity(self.len() * 64);
            s.push_str("[\n");
            for item in self {
                for line in item.pretty().lines() {
                    s.push('\t');
                    s.push_str(line);
                    s.push('\n');
                }
            }
            s.push(']');
            s
        } else {
            "[]".to_string()
        }
    }
}

impl UIfmt for String {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for u64 {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for u128 {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for bool {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl<const BITS: usize, const LIMBS: usize> UIfmt for Uint<BITS, LIMBS> {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for I256 {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for Address {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for Bloom {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for TxType {
    fn pretty(&self) -> String {
        (*self as u8).to_string()
    }
}

impl UIfmt for Vec<u8> {
    fn pretty(&self) -> String {
        self[..].pretty()
    }
}

impl UIfmt for Bytes {
    fn pretty(&self) -> String {
        self[..].pretty()
    }
}

impl<const N: usize> UIfmt for [u8; N] {
    fn pretty(&self) -> String {
        self[..].pretty()
    }
}

impl<const N: usize> UIfmt for FixedBytes<N> {
    fn pretty(&self) -> String {
        self[..].pretty()
    }
}

impl UIfmt for [u8] {
    fn pretty(&self) -> String {
        hex::encode_prefixed(self)
    }
}

impl UIfmt for Eip658Value {
    fn pretty(&self) -> String {
        match self {
            Self::Eip658(status) => if *status { "1 (success)" } else { "0 (failed)" }.to_string(),
            Self::PostState(state) => state.pretty(),
        }
    }
}

impl UIfmt for AnyTransactionReceipt {
    fn pretty(&self) -> String {
        let Self {
            inner:
                TransactionReceipt {
                    transaction_hash,
                    transaction_index,
                    block_hash,
                    block_number,
                    from,
                    to,
                    gas_used,
                    contract_address,
                    state_root,
                    effective_gas_price,
                    inner:
                        AnyReceiptEnvelope {
                            r#type: transaction_type,
                            inner:
                                ReceiptWithBloom {
                                    receipt: Receipt { status, cumulative_gas_used, logs },
                                    logs_bloom,
                                },
                        },
                    blob_gas_price,
                    blob_gas_used,
                    authorization_list,
                },
            other,
        } = self;

        let mut pretty = format!(
            "
blockHash               {}
blockNumber             {}
contractAddress         {}
cumulativeGasUsed       {}
effectiveGasPrice       {}
from                    {}
gasUsed                 {}
logs                    {}
logsBloom               {}
root                    {}
status                  {}
transactionHash         {}
transactionIndex        {}
type                    {}
blobGasPrice            {}
blobGasUsed             {}
authorizationList       {}",
            block_hash.pretty(),
            block_number.pretty(),
            contract_address.pretty(),
            cumulative_gas_used.pretty(),
            effective_gas_price.pretty(),
            from.pretty(),
            gas_used.pretty(),
            serde_json::to_string(&logs).unwrap(),
            logs_bloom.pretty(),
            state_root.pretty(),
            status.pretty(),
            transaction_hash.pretty(),
            transaction_index.pretty(),
            transaction_type,
            blob_gas_price.pretty(),
            blob_gas_used.pretty(),
            authorization_list
                .as_ref()
                .map(|l| serde_json::to_string(&l).unwrap())
                .unwrap_or_default(),
        );

        if let Some(to) = to {
            pretty.push_str(&format!("\nto                      {}", to.pretty()));
        }

        // additional captured fields
        for (key, val) in other.iter() {
            pretty.push_str(&format!("\n{key}             {val}"));
        }

        pretty
    }
}

impl UIfmt for Log {
    fn pretty(&self) -> String {
        format!(
            "
address: {}
blockHash: {}
blockNumber: {}
data: {}
logIndex: {}
removed: {}
topics: {}
transactionHash: {}
transactionIndex: {}",
            self.address().pretty(),
            self.block_hash.pretty(),
            self.block_number.pretty(),
            self.data().data.pretty(),
            self.log_index.pretty(),
            self.removed.pretty(),
            self.topics().pretty(),
            self.transaction_hash.pretty(),
            self.transaction_index.pretty(),
        )
    }
}

impl UIfmt for Block {
    fn pretty(&self) -> String {
        format!(
            "
{}
transactions:        {}",
            pretty_block_basics(self),
            self.transactions.pretty()
        )
    }
}

impl<T: UIfmt> UIfmt for BlockTransactions<T> {
    fn pretty(&self) -> String {
        match self {
            Self::Hashes(hashes) => hashes.pretty(),
            Self::Full(transactions) => transactions.pretty(),
            Self::Uncle => String::new(),
        }
    }
}

impl UIfmt for OtherFields {
    fn pretty(&self) -> String {
        let mut s = String::with_capacity(self.len() * 30);
        if !self.is_empty() {
            s.push('\n');
        }
        for (key, value) in self.iter() {
            let val = EthValue::from(value.clone()).pretty();
            let offset = NAME_COLUMN_LEN.saturating_sub(key.len());
            s.push_str(key);
            s.extend(std::iter::repeat(' ').take(offset + 1));
            s.push_str(&val);
            s.push('\n');
        }
        s
    }
}

impl UIfmt for AccessListItem {
    fn pretty(&self) -> String {
        let mut s = String::with_capacity(42 + self.storage_keys.len() * 66);
        s.push_str(self.address.pretty().as_str());
        s.push_str(" => ");
        s.push_str(self.storage_keys.pretty().as_str());
        s
    }
}

impl UIfmt for Transaction {
    fn pretty(&self) -> String {
        match self.transaction_type {
            Some(1) => format!(
                "
accessList           {}
blockHash            {}
blockNumber          {}
chainId              {}
from                 {}
gasLimit             {}
gasPrice             {}
hash                 {}
input                {}
nonce                {}
r                    {}
s                    {}
to                   {}
transactionIndex     {}
type                 {}
value                {}
yParity              {}{}",
                self.access_list.as_deref().map(Vec::as_slice).unwrap_or(&[]).pretty(),
                self.block_hash.pretty(),
                self.block_number.pretty(),
                self.chain_id.pretty(),
                self.from.pretty(),
                self.gas.pretty(),
                self.gas_price.pretty(),
                self.hash.pretty(),
                self.input.pretty(),
                self.nonce.pretty(),
                self.signature.map(|s| s.r.to_be_bytes_vec()).pretty(),
                self.signature.map(|s| s.s.to_be_bytes_vec()).pretty(),
                self.to.pretty(),
                self.transaction_index.pretty(),
                self.transaction_type.unwrap(),
                self.value.pretty(),
                self.signature.map(|s| s.v).pretty(),
                self.other.pretty()
            ),
            Some(2) => format!(
                "
accessList           {}
blockHash            {}
blockNumber          {}
chainId              {}
from                 {}
gasLimit             {}
hash                 {}
input                {}
maxFeePerGas         {}
maxPriorityFeePerGas {}
nonce                {}
r                    {}
s                    {}
to                   {}
transactionIndex     {}
type                 {}
value                {}
yParity              {}{}",
                self.access_list.as_deref().map(Vec::as_slice).unwrap_or(&[]).pretty(),
                self.block_hash.pretty(),
                self.block_number.pretty(),
                self.chain_id.pretty(),
                self.from.pretty(),
                self.gas.pretty(),
                self.hash.pretty(),
                self.input.pretty(),
                self.max_fee_per_gas.pretty(),
                self.max_priority_fee_per_gas.pretty(),
                self.nonce.pretty(),
                self.signature.map(|s| s.r.to_be_bytes_vec()).pretty(),
                self.signature.map(|s| s.s.to_be_bytes_vec()).pretty(),
                self.to.pretty(),
                self.transaction_index.pretty(),
                self.transaction_type.unwrap(),
                self.value.pretty(),
                self.signature.map(|s| s.v).pretty(),
                self.other.pretty()
            ),
            Some(3) => format!(
                "
accessList           {}
blobVersionedHashes  {}
blockHash            {}
blockNumber          {}
chainId              {}
from                 {}
gasLimit             {}
hash                 {}
input                {}
maxFeePerBlobGas     {}
maxFeePerGas         {}
maxPriorityFeePerGas {}
nonce                {}
r                    {}
s                    {}
to                   {}
transactionIndex     {}
type                 {}
value                {}
yParity              {}{}",
                self.access_list.as_deref().map(Vec::as_slice).unwrap_or(&[]).pretty(),
                self.blob_versioned_hashes.as_deref().unwrap_or(&[]).pretty(),
                self.block_hash.pretty(),
                self.block_number.pretty(),
                self.chain_id.pretty(),
                self.from.pretty(),
                self.gas.pretty(),
                self.hash.pretty(),
                self.input.pretty(),
                self.max_fee_per_blob_gas.pretty(),
                self.max_fee_per_gas.pretty(),
                self.max_priority_fee_per_gas.pretty(),
                self.nonce.pretty(),
                self.signature.map(|s| s.r.to_be_bytes_vec()).pretty(),
                self.signature.map(|s| s.s.to_be_bytes_vec()).pretty(),
                self.to.pretty(),
                self.transaction_index.pretty(),
                self.transaction_type.unwrap(),
                self.value.pretty(),
                self.signature.map(|s| s.v).pretty(),
                self.other.pretty()
            ),
            Some(4) => format!(
                "
accessList           {}
authorizationList    {}
blockHash            {}
blockNumber          {}
chainId              {}
from                 {}
gasLimit             {}
hash                 {}
input                {}
maxFeePerGas         {}
maxPriorityFeePerGas {}
nonce                {}
r                    {}
s                    {}
to                   {}
transactionIndex     {}
type                 {}
value                {}
yParity              {}{}",
                self.access_list.as_deref().map(Vec::as_slice).unwrap_or(&[]).pretty(),
                self.authorization_list
                    .as_ref()
                    .map(|l| serde_json::to_string(&l).unwrap())
                    .unwrap_or_default(),
                self.block_hash.pretty(),
                self.block_number.pretty(),
                self.chain_id.pretty(),
                self.from.pretty(),
                self.gas.pretty(),
                self.hash.pretty(),
                self.input.pretty(),
                self.max_fee_per_gas.pretty(),
                self.max_priority_fee_per_gas.pretty(),
                self.nonce.pretty(),
                self.signature.map(|s| s.r.to_be_bytes_vec()).pretty(),
                self.signature.map(|s| s.s.to_be_bytes_vec()).pretty(),
                self.to.pretty(),
                self.transaction_index.pretty(),
                self.transaction_type.unwrap(),
                self.value.pretty(),
                self.signature.map(|s| s.v).pretty(),
                self.other.pretty()
            ),
            _ => format!(
                "
blockHash            {}
blockNumber          {}
from                 {}
gas                  {}
gasPrice             {}
hash                 {}
input                {}
nonce                {}
r                    {}
s                    {}
to                   {}
transactionIndex     {}
v                    {}
value                {}{}",
                self.block_hash.pretty(),
                self.block_number.pretty(),
                self.from.pretty(),
                self.gas.pretty(),
                self.gas_price.pretty(),
                self.hash.pretty(),
                self.input.pretty(),
                self.nonce,
                self.signature.map(|s| s.r.to_be_bytes_vec()).pretty(),
                self.signature.map(|s| s.s.to_be_bytes_vec()).pretty(),
                self.to.pretty(),
                self.transaction_index.pretty(),
                self.signature.map(|s| s.v).pretty(),
                self.value.pretty(),
                self.other.pretty()
            ),
        }
    }
}

/// Various numerical ethereum types used for pretty printing
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
#[allow(missing_docs)]
pub enum EthValue {
    U64(U64),
    U256(U256),
    U64Array(Vec<U64>),
    U256Array(Vec<U256>),
    Other(serde_json::Value),
}

impl From<serde_json::Value> for EthValue {
    fn from(val: serde_json::Value) -> Self {
        serde_json::from_value(val).expect("infallible")
    }
}

impl UIfmt for EthValue {
    fn pretty(&self) -> String {
        match self {
            Self::U64(num) => num.pretty(),
            Self::U256(num) => num.pretty(),
            Self::U64Array(arr) => arr.pretty(),
            Self::U256Array(arr) => arr.pretty(),
            Self::Other(val) => val.to_string().trim_matches('"').to_string(),
        }
    }
}

/// Returns the `UiFmt::pretty()` formatted attribute of the transactions
pub fn get_pretty_tx_attr(transaction: &Transaction, attr: &str) -> Option<String> {
    match attr {
        "blockHash" | "block_hash" => Some(transaction.block_hash.pretty()),
        "blockNumber" | "block_number" => Some(transaction.block_number.pretty()),
        "from" => Some(transaction.from.pretty()),
        "gas" => Some(transaction.gas.pretty()),
        "gasPrice" | "gas_price" => Some(transaction.gas_price.pretty()),
        "hash" => Some(transaction.hash.pretty()),
        "input" => Some(transaction.input.pretty()),
        "nonce" => Some(transaction.nonce.to_string()),
        "s" => transaction.signature.map(|s| B256::from(s.s).pretty()),
        "r" => transaction.signature.map(|s| B256::from(s.r).pretty()),
        "to" => Some(transaction.to.pretty()),
        "transactionIndex" | "transaction_index" => Some(transaction.transaction_index.pretty()),
        "v" => transaction.signature.map(|s| s.v.pretty()),
        "value" => Some(transaction.value.pretty()),
        other => {
            if let Some(value) = transaction.other.get(other) {
                return Some(value.to_string().trim_matches('"').to_string())
            }
            None
        }
    }
}

/// Returns the `UiFmt::pretty()` formatted attribute of the given block
pub fn get_pretty_block_attr(block: &Block, attr: &str) -> Option<String> {
    match attr {
        "baseFeePerGas" | "base_fee_per_gas" => Some(block.header.base_fee_per_gas.pretty()),
        "difficulty" => Some(block.header.difficulty.pretty()),
        "extraData" | "extra_data" => Some(block.header.extra_data.pretty()),
        "gasLimit" | "gas_limit" => Some(block.header.gas_limit.pretty()),
        "gasUsed" | "gas_used" => Some(block.header.gas_used.pretty()),
        "hash" => Some(block.header.hash.pretty()),
        "logsBloom" | "logs_bloom" => Some(block.header.logs_bloom.pretty()),
        "miner" | "author" => Some(block.header.miner.pretty()),
        "mixHash" | "mix_hash" => Some(block.header.mix_hash.pretty()),
        "nonce" => Some(block.header.nonce.pretty()),
        "number" => Some(block.header.number.pretty()),
        "parentHash" | "parent_hash" => Some(block.header.parent_hash.pretty()),
        "transactionsRoot" | "transactions_root" => Some(block.header.transactions_root.pretty()),
        "receiptsRoot" | "receipts_root" => Some(block.header.receipts_root.pretty()),
        "sha3Uncles" | "sha_3_uncles" => Some(block.header.uncles_hash.pretty()),
        "size" => Some(block.size.pretty()),
        "stateRoot" | "state_root" => Some(block.header.state_root.pretty()),
        "timestamp" => Some(block.header.timestamp.pretty()),
        "totalDifficulty" | "total_difficult" => Some(block.header.total_difficulty.pretty()),
        other => {
            if let Some(value) = block.other.get(other) {
                let val = EthValue::from(value.clone());
                return Some(val.pretty())
            }
            None
        }
    }
}

fn pretty_block_basics(block: &Block) -> String {
    format!(
        "
baseFeePerGas        {}
difficulty           {}
extraData            {}
gasLimit             {}
gasUsed              {}
hash                 {}
logsBloom            {}
miner                {}
mixHash              {}
nonce                {}
number               {}
parentHash           {}
transactionsRoot     {}
receiptsRoot         {}
sha3Uncles           {}
size                 {}
stateRoot            {}
timestamp            {} ({})
withdrawalsRoot      {}
totalDifficulty      {}{}",
        block.header.base_fee_per_gas.pretty(),
        block.header.difficulty.pretty(),
        block.header.extra_data.pretty(),
        block.header.gas_limit.pretty(),
        block.header.gas_used.pretty(),
        block.header.hash.pretty(),
        block.header.logs_bloom.pretty(),
        block.header.miner.pretty(),
        block.header.mix_hash.pretty(),
        block.header.nonce.pretty(),
        block.header.number.pretty(),
        block.header.parent_hash.pretty(),
        block.header.transactions_root.pretty(),
        block.header.receipts_root.pretty(),
        block.header.uncles_hash.pretty(),
        block.size.pretty(),
        block.header.state_root.pretty(),
        block.header.timestamp.pretty(),
        chrono::DateTime::from_timestamp(block.header.timestamp as i64, 0)
            .expect("block timestamp in range")
            .to_rfc2822(),
        block.header.withdrawals_root.pretty(),
        block.header.total_difficulty.pretty(),
        block.other.pretty()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar_asserts::assert_eq;
    use std::str::FromStr;

    #[test]
    fn can_format_bytes32() {
        let val = hex::decode("7465737400000000000000000000000000000000000000000000000000000000")
            .unwrap();
        let mut b32 = [0u8; 32];
        b32.copy_from_slice(&val);

        assert_eq!(
            b32.pretty(),
            "0x7465737400000000000000000000000000000000000000000000000000000000"
        );
        let b: Bytes = val.into();
        assert_eq!(b.pretty(), b32.pretty());
    }

    #[test]
    fn can_pretty_print_optimism_tx() {
        let s = r#"
        {
        "blockHash": "0x02b853cf50bc1c335b70790f93d5a390a35a166bea9c895e685cc866e4961cae",
        "blockNumber": "0x1b4",
        "from": "0x3b179DcfC5fAa677044c27dCe958e4BC0ad696A6",
        "gas": "0x11cbbdc",
        "gasPrice": "0x0",
        "hash": "0x2642e960d3150244e298d52b5b0f024782253e6d0b2c9a01dd4858f7b4665a3f",
        "input": "0xd294f093",
        "nonce": "0xa2",
        "to": "0x4a16A42407AA491564643E1dfc1fd50af29794eF",
        "transactionIndex": "0x0",
        "value": "0x0",
        "v": "0x38",
        "r": "0x6fca94073a0cf3381978662d46cf890602d3e9ccf6a31e4b69e8ecbd995e2bee",
        "s": "0xe804161a2b56a37ca1f6f4c4b8bce926587afa0d9b1acc5165e6556c959d583",
        "queueOrigin": "sequencer",
        "txType": "",
        "l1TxOrigin": null,
        "l1BlockNumber": "0xc1a65c",
        "l1Timestamp": "0x60d34b60",
        "index": "0x1b3",
        "queueIndex": null,
        "rawTransaction": "0xf86681a28084011cbbdc944a16a42407aa491564643e1dfc1fd50af29794ef8084d294f09338a06fca94073a0cf3381978662d46cf890602d3e9ccf6a31e4b69e8ecbd995e2beea00e804161a2b56a37ca1f6f4c4b8bce926587afa0d9b1acc5165e6556c959d583"
    }
        "#;

        let tx: Transaction = serde_json::from_str(s).unwrap();
        assert_eq!(tx.pretty().trim(),
                   r"
blockHash            0x02b853cf50bc1c335b70790f93d5a390a35a166bea9c895e685cc866e4961cae
blockNumber          436
from                 0x3b179DcfC5fAa677044c27dCe958e4BC0ad696A6
gas                  18660316
gasPrice             0
hash                 0x2642e960d3150244e298d52b5b0f024782253e6d0b2c9a01dd4858f7b4665a3f
input                0xd294f093
nonce                162
r                    0x6fca94073a0cf3381978662d46cf890602d3e9ccf6a31e4b69e8ecbd995e2bee
s                    0x0e804161a2b56a37ca1f6f4c4b8bce926587afa0d9b1acc5165e6556c959d583
to                   0x4a16A42407AA491564643E1dfc1fd50af29794eF
transactionIndex     0
v                    56
value                0
index                435
l1BlockNumber        12691036
l1Timestamp          1624460128
l1TxOrigin           null
queueIndex           null
queueOrigin          sequencer
rawTransaction       0xf86681a28084011cbbdc944a16a42407aa491564643e1dfc1fd50af29794ef8084d294f09338a06fca94073a0cf3381978662d46cf890602d3e9ccf6a31e4b69e8ecbd995e2beea00e804161a2b56a37ca1f6f4c4b8bce926587afa0d9b1acc5165e6556c959d583
txType               0
".trim()
        );
    }

    #[test]
    fn can_pretty_print_eip2930() {
        let s = r#"{
        "type": "0x1",
        "blockHash": "0x2b27fe2bbc8ce01ac7ae8bf74f793a197cf7edbe82727588811fa9a2c4776f81",
        "blockNumber": "0x12b1d",
        "from": "0x2b371c0262ceab27face32fbb5270ddc6aa01ba4",
        "gas": "0x6bdf",
        "gasPrice": "0x3b9aca00",
        "hash": "0xbddbb685774d8a3df036ed9fb920b48f876090a57e9e90ee60921e0510ef7090",
        "input": "0x9c0e3f7a0000000000000000000000000000000000000000000000000000000000000078000000000000000000000000000000000000000000000000000000000000002a",
        "nonce": "0x1c",
        "to": "0x8e730df7c70d33118d9e5f79ab81aed0be6f6635",
        "transactionIndex": "0x2",
        "value": "0x0",
        "v": "0x1",
        "r": "0x2a98c51c2782f664d3ce571fef0491b48f5ebbc5845fa513192e6e6b24ecdaa1",
        "s": "0x29b8e0c67aa9c11327e16556c591dc84a7aac2f6fc57c7f93901be8ee867aebc",
		"chainId": "0x66a",
		"accessList": [
			{ "address": "0x2b371c0262ceab27face32fbb5270ddc6aa01ba4", "storageKeys": ["0x1122334455667788990011223344556677889900112233445566778899001122", "0x0000000000000000000000000000000000000000000000000000000000000000"] },
			{ "address": "0x8e730df7c70d33118d9e5f79ab81aed0be6f6635", "storageKeys": [] }
		]
      }
        "#;

        let tx: Transaction = serde_json::from_str(s).unwrap();
        assert_eq!(tx.pretty().trim(),
                   r"
accessList           [
	0x2b371c0262CEAb27fAcE32FBB5270dDc6Aa01ba4 => [
		0x1122334455667788990011223344556677889900112233445566778899001122
		0x0000000000000000000000000000000000000000000000000000000000000000
	]
	0x8E730Df7C70D33118D9e5F79ab81aEd0bE6F6635 => []
]
blockHash            0x2b27fe2bbc8ce01ac7ae8bf74f793a197cf7edbe82727588811fa9a2c4776f81
blockNumber          76573
chainId              1642
from                 0x2b371c0262CEAb27fAcE32FBB5270dDc6Aa01ba4
gasLimit             27615
gasPrice             1000000000
hash                 0xbddbb685774d8a3df036ed9fb920b48f876090a57e9e90ee60921e0510ef7090
input                0x9c0e3f7a0000000000000000000000000000000000000000000000000000000000000078000000000000000000000000000000000000000000000000000000000000002a
nonce                28
r                    0x2a98c51c2782f664d3ce571fef0491b48f5ebbc5845fa513192e6e6b24ecdaa1
s                    0x29b8e0c67aa9c11327e16556c591dc84a7aac2f6fc57c7f93901be8ee867aebc
to                   0x8E730Df7C70D33118D9e5F79ab81aEd0bE6F6635
transactionIndex     2
type                 1
value                0
yParity              1
".trim()
        );
    }

    #[test]
    fn can_pretty_print_eip1559() {
        let s = r#"{
        "type": "0x2",
        "blockHash": "0x61abbe5e22738de0462046f5a5d6c4cd6bc1f3a6398e4457d5e293590e721125",
        "blockNumber": "0x7647",
        "from": "0xbaadf00d42264eeb3fafe6799d0b56cf55df0f00",
        "gas": "0x186a0",
        "hash": "0xa7231d4da0576fade5d3b9481f4cd52459ec59b9bbdbf4f60d6cd726b2a3a244",
        "input": "0x48600055323160015500",
        "nonce": "0x12c",
        "to": null,
        "transactionIndex": "0x41",
        "value": "0x0",
        "v": "0x1",
        "yParity": "0x1",
        "r": "0x396864e5f9132327defdb1449504252e1fa6bce73feb8cd6f348a342b198af34",
        "s": "0x44dbba72e6d3304104848277143252ee43627c82f02d1ef8e404e1bf97c70158",
        "gasPrice": "0x4a817c800",
        "maxFeePerGas": "0x4a817c800",
        "maxPriorityFeePerGas": "0x4a817c800",
        "chainId": "0x66a",
        "accessList": [
          {
            "address": "0xc141a9a7463e6c4716d9fc0c056c054f46bb2993",
            "storageKeys": [
              "0x0000000000000000000000000000000000000000000000000000000000000000"
            ]
          }
        ]
      }
"#;
        let tx: Transaction = serde_json::from_str(s).unwrap();
        assert_eq!(
            tx.pretty().trim(),
            r"
accessList           [
	0xC141a9A7463e6C4716d9FC0C056C054F46Bb2993 => [
		0x0000000000000000000000000000000000000000000000000000000000000000
	]
]
blockHash            0x61abbe5e22738de0462046f5a5d6c4cd6bc1f3a6398e4457d5e293590e721125
blockNumber          30279
chainId              1642
from                 0xBaaDF00d42264eEb3FAFe6799d0b56cf55DF0F00
gasLimit             100000
hash                 0xa7231d4da0576fade5d3b9481f4cd52459ec59b9bbdbf4f60d6cd726b2a3a244
input                0x48600055323160015500
maxFeePerGas         20000000000
maxPriorityFeePerGas 20000000000
nonce                300
r                    0x396864e5f9132327defdb1449504252e1fa6bce73feb8cd6f348a342b198af34
s                    0x44dbba72e6d3304104848277143252ee43627c82f02d1ef8e404e1bf97c70158
to                   
transactionIndex     65
type                 2
value                0
yParity              1
"
            .trim()
        );
    }

    #[test]
    fn can_pretty_print_eip4884() {
        let s = r#"{
		"blockHash": "0xfc2715ff196e23ae613ed6f837abd9035329a720a1f4e8dce3b0694c867ba052",
		"blockNumber": "0x2a1cb",
		"from": "0xad01b55d7c3448b8899862eb335fbb17075d8de2",
		"gas": "0x5208",
		"gasPrice": "0x1d1a94a201c",
		"maxFeePerGas": "0x1d1a94a201c",
		"maxPriorityFeePerGas": "0x1d1a94a201c",
		"maxFeePerBlobGas": "0x3e8",
		"hash": "0x5ceec39b631763ae0b45a8fb55c373f38b8fab308336ca1dc90ecd2b3cf06d00",
		"input": "0x",
		"nonce": "0x1b483",
		"to": "0x000000000000000000000000000000000000f1c1",
		"transactionIndex": "0x0",
		"value": "0x0",
		"type": "0x3",
		"accessList": [],
		"chainId": "0x1a1f0ff42",
		"blobVersionedHashes": [
		  "0x01a128c46fc61395706686d6284f83c6c86dfc15769b9363171ea9d8566e6e76"
		],
		"v": "0x0",
		"r": "0x343c6239323a81ef61293cb4a4d37b6df47fbf68114adb5dd41581151a077da1",
		"s": "0x48c21f6872feaf181d37cc4f9bbb356d3f10b352ceb38d1c3b190d749f95a11b",
		"yParity": "0x0"
	  }
"#;
        let tx: Transaction = serde_json::from_str(s).unwrap();
        assert_eq!(
            tx.pretty().trim(),
            r"
accessList           []
blobVersionedHashes  [
	0x01a128c46fc61395706686d6284f83c6c86dfc15769b9363171ea9d8566e6e76
]
blockHash            0xfc2715ff196e23ae613ed6f837abd9035329a720a1f4e8dce3b0694c867ba052
blockNumber          172491
chainId              7011893058
from                 0xAD01b55d7c3448B8899862eb335FBb17075d8DE2
gasLimit             21000
hash                 0x5ceec39b631763ae0b45a8fb55c373f38b8fab308336ca1dc90ecd2b3cf06d00
input                0x
maxFeePerBlobGas     1000
maxFeePerGas         2000000000028
maxPriorityFeePerGas 2000000000028
nonce                111747
r                    0x343c6239323a81ef61293cb4a4d37b6df47fbf68114adb5dd41581151a077da1
s                    0x48c21f6872feaf181d37cc4f9bbb356d3f10b352ceb38d1c3b190d749f95a11b
to                   0x000000000000000000000000000000000000f1C1
transactionIndex     0
type                 3
value                0
yParity              0
"
            .trim()
        );
    }

    #[test]
    fn print_block_w_txs() {
        let block = r#"{"number":"0x3","hash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","parentHash":"0x689c70c080ca22bc0e681694fa803c1aba16a69c8b6368fed5311d279eb9de90","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","transactionsRoot":"0x7270c1c4440180f2bd5215809ee3d545df042b67329499e1ab97eb759d31610d","stateRoot":"0x29f32984517a7d25607da485b23cefabfd443751422ca7e603395e1de9bc8a4b","receiptsRoot":"0x056b23fbba480696b65fe5a59b8f2148a1299103c4f57df839233af2cf4ca2d2","miner":"0x0000000000000000000000000000000000000000","difficulty":"0x0","totalDifficulty":"0x0","extraData":"0x","size":"0x3e8","gasLimit":"0x6691b7","gasUsed":"0x5208","timestamp":"0x5ecedbb9","transactions":[{"hash":"0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067","nonce":"0x2","blockHash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","blockNumber":"0x3","transactionIndex":"0x0","from":"0xfdcedc3bfca10ecb0890337fbdd1977aba84807a","to":"0xdca8ce283150ab773bcbeb8d38289bdb5661de1e","value":"0x0","gas":"0x15f90","gasPrice":"0x4a817c800","input":"0x","v":"0x25","r":"0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88","s":"0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e"}],"uncles":[]}"#;
        let block: Block = serde_json::from_str(block).unwrap();
        let output ="\nblockHash            0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972
blockNumber          3
from                 0xFdCeDC3bFca10eCb0890337fbdD1977aba84807a
gas                  90000
gasPrice             20000000000
hash                 0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067
input                0x
nonce                2
r                    0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88
s                    0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e
to                   0xdca8ce283150AB773BCbeB8d38289bdB5661dE1e
transactionIndex     0
v                    37
value                0".to_string();
        let txs = match block.transactions {
            BlockTransactions::Full(txs) => txs,
            _ => panic!("not full transactions"),
        };
        let generated = txs[0].pretty();
        assert_eq!(generated.as_str(), output.as_str());
    }

    #[test]
    fn uifmt_option_u64() {
        assert_eq!(None::<U64>.pretty(), "");
        assert_eq!(U64::from(100).pretty(), "100");
        assert_eq!(Some(U64::from(100)).pretty(), "100");
    }

    #[test]
    fn uifmt_option_h64() {
        assert_eq!(None::<B256>.pretty(), "");
        assert_eq!(
            B256::with_last_byte(100).pretty(),
            "0x0000000000000000000000000000000000000000000000000000000000000064",
        );
        assert_eq!(
            Some(B256::with_last_byte(100)).pretty(),
            "0x0000000000000000000000000000000000000000000000000000000000000064",
        );
    }

    #[test]
    fn uifmt_option_bytes() {
        assert_eq!(None::<Bytes>.pretty(), "");
        assert_eq!(
            Bytes::from_str("0x0000000000000000000000000000000000000000000000000000000000000064")
                .unwrap()
                .pretty(),
            "0x0000000000000000000000000000000000000000000000000000000000000064",
        );
        assert_eq!(
            Some(
                Bytes::from_str(
                    "0x0000000000000000000000000000000000000000000000000000000000000064"
                )
                .unwrap()
            )
            .pretty(),
            "0x0000000000000000000000000000000000000000000000000000000000000064",
        );
    }

    #[test]
    fn test_pretty_tx_attr() {
        let block = r#"{"number":"0x3","hash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","parentHash":"0x689c70c080ca22bc0e681694fa803c1aba16a69c8b6368fed5311d279eb9de90","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","transactionsRoot":"0x7270c1c4440180f2bd5215809ee3d545df042b67329499e1ab97eb759d31610d","stateRoot":"0x29f32984517a7d25607da485b23cefabfd443751422ca7e603395e1de9bc8a4b","receiptsRoot":"0x056b23fbba480696b65fe5a59b8f2148a1299103c4f57df839233af2cf4ca2d2","miner":"0x0000000000000000000000000000000000000000","difficulty":"0x0","totalDifficulty":"0x0","extraData":"0x","size":"0x3e8","gasLimit":"0x6691b7","gasUsed":"0x5208","timestamp":"0x5ecedbb9","transactions":[{"hash":"0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067","nonce":"0x2","blockHash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","blockNumber":"0x3","transactionIndex":"0x0","from":"0xfdcedc3bfca10ecb0890337fbdd1977aba84807a","to":"0xdca8ce283150ab773bcbeb8d38289bdb5661de1e","value":"0x0","gas":"0x15f90","gasPrice":"0x4a817c800","input":"0x","v":"0x25","r":"0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88","s":"0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e"}],"uncles":[]}"#;
        let block: Block = serde_json::from_str(block).unwrap();
        let txs = match block.transactions {
            BlockTransactions::Full(txes) => txes,
            _ => panic!("not full transactions"),
        };
        assert_eq!(None, get_pretty_tx_attr(&txs[0], ""));
        assert_eq!(Some("3".to_string()), get_pretty_tx_attr(&txs[0], "blockNumber"));
        assert_eq!(
            Some("0xFdCeDC3bFca10eCb0890337fbdD1977aba84807a".to_string()),
            get_pretty_tx_attr(&txs[0], "from")
        );
        assert_eq!(Some("90000".to_string()), get_pretty_tx_attr(&txs[0], "gas"));
        assert_eq!(Some("20000000000".to_string()), get_pretty_tx_attr(&txs[0], "gasPrice"));
        assert_eq!(
            Some("0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067".to_string()),
            get_pretty_tx_attr(&txs[0], "hash")
        );
        assert_eq!(Some("0x".to_string()), get_pretty_tx_attr(&txs[0], "input"));
        assert_eq!(Some("2".to_string()), get_pretty_tx_attr(&txs[0], "nonce"));
        assert_eq!(
            Some("0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88".to_string()),
            get_pretty_tx_attr(&txs[0], "r")
        );
        assert_eq!(
            Some("0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e".to_string()),
            get_pretty_tx_attr(&txs[0], "s")
        );
        assert_eq!(
            Some("0xdca8ce283150AB773BCbeB8d38289bdB5661dE1e".into()),
            get_pretty_tx_attr(&txs[0], "to")
        );
        assert_eq!(Some("0".to_string()), get_pretty_tx_attr(&txs[0], "transactionIndex"));
        assert_eq!(Some("37".to_string()), get_pretty_tx_attr(&txs[0], "v"));
        assert_eq!(Some("0".to_string()), get_pretty_tx_attr(&txs[0], "value"));
    }

    #[test]
    fn test_pretty_block_attr() {
        let json = serde_json::json!(
        {
            "baseFeePerGas": "0x7",
            "miner": "0x0000000000000000000000000000000000000001",
            "number": "0x1b4",
            "hash": "0x0e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d1527331",
            "parentHash": "0x9646252be9520f6e71339a8df9c55e4d7619deeb018d2a3f2d21fc165dde5eb5",
            "mixHash": "0x1010101010101010101010101010101010101010101010101010101010101010",
            "nonce": "0x0000000000000000",
            "sealFields": [
              "0xe04d296d2460cfb8472af2c5fd05b5a214109c25688d3704aed5484f9a7792f2",
              "0x0000000000000042"
            ],
            "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
            "logsBloom":  "0x0e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d1527331",
            "transactionsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            "receiptsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
            "stateRoot": "0xd5855eb08b3387c0af375e9cdb6acfc05eb8f519e419b874b6ff2ffda7ed1dff",
            "difficulty": "0x27f07",
            "totalDifficulty": "0x27f07",
            "extraData": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "size": "0x27f07",
            "gasLimit": "0x9f759",
            "minGasPrice": "0x9f759",
            "gasUsed": "0x9f759",
            "timestamp": "0x54e34e8e",
            "transactions": [],
            "uncles": []
          }
        );

        let block: Block = serde_json::from_value(json).unwrap();

        assert_eq!(None, get_pretty_block_attr(&block, ""));
        assert_eq!(Some("7".to_string()), get_pretty_block_attr(&block, "baseFeePerGas"));
        assert_eq!(Some("163591".to_string()), get_pretty_block_attr(&block, "difficulty"));
        assert_eq!(
            Some("0x0000000000000000000000000000000000000000000000000000000000000000".to_string()),
            get_pretty_block_attr(&block, "extraData")
        );
        assert_eq!(Some("653145".to_string()), get_pretty_block_attr(&block, "gasLimit"));
        assert_eq!(Some("653145".to_string()), get_pretty_block_attr(&block, "gasUsed"));
        assert_eq!(
            Some("0x0e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d1527331".to_string()),
            get_pretty_block_attr(&block, "hash")
        );
        assert_eq!(Some("0x0e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d1527331".to_string()), get_pretty_block_attr(&block, "logsBloom"));
        assert_eq!(
            Some("0x0000000000000000000000000000000000000001".to_string()),
            get_pretty_block_attr(&block, "miner")
        );
        assert_eq!(
            Some("0x1010101010101010101010101010101010101010101010101010101010101010".to_string()),
            get_pretty_block_attr(&block, "mixHash")
        );
        assert_eq!(Some("0x0000000000000000".to_string()), get_pretty_block_attr(&block, "nonce"));
        assert_eq!(Some("436".to_string()), get_pretty_block_attr(&block, "number"));
        assert_eq!(
            Some("0x9646252be9520f6e71339a8df9c55e4d7619deeb018d2a3f2d21fc165dde5eb5".to_string()),
            get_pretty_block_attr(&block, "parentHash")
        );
        assert_eq!(
            Some("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421".to_string()),
            get_pretty_block_attr(&block, "transactionsRoot")
        );
        assert_eq!(
            Some("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421".to_string()),
            get_pretty_block_attr(&block, "receiptsRoot")
        );
        assert_eq!(
            Some("0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347".to_string()),
            get_pretty_block_attr(&block, "sha3Uncles")
        );
        assert_eq!(Some("163591".to_string()), get_pretty_block_attr(&block, "size"));
        assert_eq!(
            Some("0xd5855eb08b3387c0af375e9cdb6acfc05eb8f519e419b874b6ff2ffda7ed1dff".to_string()),
            get_pretty_block_attr(&block, "stateRoot")
        );
        assert_eq!(Some("1424182926".to_string()), get_pretty_block_attr(&block, "timestamp"));
        assert_eq!(Some("163591".to_string()), get_pretty_block_attr(&block, "totalDifficulty"));
    }
}
