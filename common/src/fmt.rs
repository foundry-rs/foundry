//! Contains a helper pretty() function to print human redeable string versions of usual ethers
//! types
use ethers_core::types::*;
use std::str;

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

impl UIfmt for H160 {
    fn pretty(&self) -> String {
        format!("{:#x}", self)
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
totalDifficulty      {}
transactions         {}",
            self.base_fee_per_gas.pretty(),
            self.difficulty.pretty(),
            self.extra_data.pretty(),
            self.gas_limit.pretty(),
            self.gas_used.pretty(),
            self.hash.pretty(),
            self.logs_bloom.pretty(),
            self.author.pretty(),
            self.mix_hash.pretty(),
            self.nonce.pretty(),
            self.number.pretty(),
            self.parent_hash.pretty(),
            self.receipts_root.pretty(),
            self.seal_fields.pretty(),
            self.uncles_hash.pretty(),
            self.size.pretty(),
            self.state_root.pretty(),
            self.timestamp.pretty(),
            self.total_difficulty.pretty(),
            self.transactions.pretty()
        )
    }
}

impl UIfmt for Block<TxHash> {
    fn pretty(&self) -> String {
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
totalDifficulty      {}
transactions:        {}",
            self.base_fee_per_gas.pretty(),
            self.difficulty.pretty(),
            self.extra_data.pretty(),
            self.gas_limit.pretty(),
            self.gas_used.pretty(),
            self.hash.pretty(),
            self.logs_bloom.pretty(),
            self.author.pretty(),
            self.mix_hash.pretty(),
            self.nonce.pretty(),
            self.number.pretty(),
            self.parent_hash.pretty(),
            self.receipts_root.pretty(),
            self.seal_fields.pretty(),
            self.uncles_hash.pretty(),
            self.size.pretty(),
            self.state_root.pretty(),
            self.timestamp.pretty(),
            self.total_difficulty.pretty(),
            self.transactions.pretty()
        )
    }
}

impl UIfmt for Transaction {
    fn pretty(&self) -> String {
        format!(
            "
blockHash               {}
blockNumber             {}
from                    {}
gas                     {}
gasPrice                {}
hash                    {}
input                   {}
nonce                   {}
r                       {}
s                       {}
to                      {}
transactionIndex        {}
v                       {}
value                   {}",
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
            self.value.pretty()
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
