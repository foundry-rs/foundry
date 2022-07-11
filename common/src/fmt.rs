//! Contains a helper pretty() function to print human redeable string versions of usual ethers
//! types
use ethers_core::{types::*, utils::to_checksum};
use serde::Deserialize;
use std::str;

/// length of the name column for pretty formatting `{:>20}{value}`
const NAME_COLUMN_LEN: usize = 20usize;

///
/// Uifmt is a helper trait to format the usual ethers types
/// It offers a `pretty()` function that returns a human readable String of the value
/// # Example
/// ```
/// use foundry_common::fmt::UIfmt;
/// let boolean: bool = true;
/// let string = boolean.pretty();
/// ```
pub trait UIfmt {
    /// Return a pretty-fied string version of the value
    fn pretty(&self) -> String;
}

impl UIfmt for bool {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for U256 {
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
        to_checksum(self, None)
    }
}

impl UIfmt for H64 {
    fn pretty(&self) -> String {
        format!("{:#x}", self)
    }
}

impl UIfmt for H256 {
    fn pretty(&self) -> String {
        format!("{:#x}", self)
    }
}

impl UIfmt for Bytes {
    fn pretty(&self) -> String {
        format!("{:#x}", self)
    }
}

impl UIfmt for [u8; 32] {
    fn pretty(&self) -> String {
        let res = str::from_utf8(self).unwrap().trim_matches(char::from(0));
        String::from(res)
    }
}

impl UIfmt for U64 {
    fn pretty(&self) -> String {
        self.to_string()
    }
}

impl UIfmt for Bloom {
    fn pretty(&self) -> String {
        format!("{:#x}", self)
    }
}

impl<T: UIfmt> UIfmt for Option<T> {
    fn pretty(&self) -> String {
        if let Some(ref inner) = self {
            inner.pretty()
        } else {
            "".to_string()
        }
    }
}

impl<T: UIfmt> UIfmt for Vec<T> {
    fn pretty(&self) -> String {
        if !self.is_empty() {
            format!(
                "[\n{}]",
                self.iter().fold("".to_string(), |s, x| s + tab_paragraph(x.pretty()).as_str())
            )
        } else {
            "[]".to_string()
        }
    }
}

impl UIfmt for TransactionReceipt {
    fn pretty(&self) -> String {
        format!(
            "
blockHash               {}
blockNumber             {}
contractAddress         {}
cumulativeGasUsed       {}
effectiveGasPrice       {}
gasUsed                 {}
logs                    {}
logsBloom               {}
root                    {}
status                  {}
transactionHash         {}
transactionIndex        {}
type                    {}",
            self.block_hash.pretty(),
            self.block_number.pretty(),
            self.contract_address.pretty(),
            self.cumulative_gas_used.pretty(),
            self.effective_gas_price.pretty(),
            self.gas_used.pretty(),
            serde_json::to_string(&self.logs).unwrap(),
            self.logs_bloom.pretty(),
            self.root.pretty(),
            self.status.pretty(),
            self.transaction_hash.pretty(),
            self.transaction_index.pretty(),
            self.transaction_type.pretty()
        )
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
            self.address.pretty(),
            self.block_hash.pretty(),
            self.block_number.pretty(),
            self.data.pretty(),
            self.log_index.pretty(),
            self.removed.pretty(),
            self.topics.pretty(),
            self.transaction_hash.pretty(),
            self.transaction_index.pretty(),
        )
    }
}

impl UIfmt for Block<Transaction> {
    fn pretty(&self) -> String {
        format!(
            "
{}
transactions         {}",
            pretty_block_basics(self),
            self.transactions.pretty()
        )
    }
}

impl UIfmt for Block<TxHash> {
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

fn pretty_block_basics<T>(block: &Block<T>) -> String {
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
receiptsRoot         {}
sealFields           {}
sha3Uncles           {}
size                 {}
stateRoot            {}
timestamp            {}
totalDifficulty      {}{}",
        block.base_fee_per_gas.pretty(),
        block.difficulty.pretty(),
        block.extra_data.pretty(),
        block.gas_limit.pretty(),
        block.gas_used.pretty(),
        block.hash.pretty(),
        block.logs_bloom.pretty(),
        block.author.pretty(),
        block.mix_hash.pretty(),
        block.nonce.pretty(),
        block.number.pretty(),
        block.parent_hash.pretty(),
        block.receipts_root.pretty(),
        block.seal_fields.pretty(),
        block.uncles_hash.pretty(),
        block.size.pretty(),
        block.state_root.pretty(),
        block.timestamp.pretty(),
        block.total_difficulty.pretty(),
        block.other.pretty()
    )
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

/// Various numerical ethereum types used for pretty printing
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
#[allow(missing_docs)]
pub enum EthValue {
    U64(U64),
    U256(U256),
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
            EthValue::U64(num) => num.pretty(),
            EthValue::U256(num) => num.pretty(),
            EthValue::Other(val) => val.to_string().trim_matches('"').to_string(),
        }
    }
}

impl UIfmt for Transaction {
    fn pretty(&self) -> String {
        format!(
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
            self.nonce.pretty(),
            to_bytes(self.r).pretty(),
            to_bytes(self.s).pretty(),
            self.to.pretty(),
            self.transaction_index.pretty(),
            self.v.pretty(),
            self.value.pretty(),
            self.other.pretty()
        )
    }
}

fn tab_paragraph(paragraph: String) -> String {
    paragraph.lines().into_iter().fold("".to_string(), |acc, x| acc + "\t" + x + "\n")
}

/// Convert a U256 to bytes
pub fn to_bytes(uint: U256) -> Bytes {
    let mut buffer: [u8; 4 * 8] = [0; 4 * 8];
    uint.to_big_endian(&mut buffer);
    Bytes::from(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

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
       r#"
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
txType
"#.trim()
       );
    }
}
