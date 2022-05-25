use ethers_core::types::{Block, Transaction, TransactionReceipt, H256, U256, *};

pub fn to_bytes(uint: U256) -> Bytes {
    let mut buffer: [u8; 4 * 8] = [0; 4 * 8];
    uint.to_big_endian(&mut buffer);
    Bytes::from(buffer)
}

pub fn get_pretty_tx_attr(transaction: Transaction, attr: String) -> Option<String> {
    return match attr.as_str() {
        "blockHash" | "block_hash" => Some(transaction.block_hash.pretty()),
        "blockNumber" | "block_number" => Some(transaction.block_number.pretty()),
        "from" => Some(transaction.from.pretty()),
        "gas" => Some(transaction.gas.pretty()),
        "gasPrice" | "gas_price" => Some(transaction.gas_price.pretty()),
        "hash" => Some(transaction.hash.pretty()),
        "input" => Some(transaction.input.pretty()),
        "nonce" => Some(transaction.nonce.pretty()),
        "s" => Some(to_bytes(transaction.s).pretty()),
        "r" => Some(to_bytes(transaction.r).pretty()),
        "to" => Some(transaction.to.pretty()),
        "transactionIndex" | "transaction_index" => Some(transaction.transaction_index.pretty()),
        "v" => Some(transaction.v.pretty()),
        "value" => Some(transaction.value.pretty()),
        _ => None,
    }
}

pub fn get_pretty_block_attr<TX>(block: Block<TX>, attr: String) -> Option<String> {
    return match attr.as_str() {
        "baseFeePerGas" | "base_fee_per_gas" => Some(block.base_fee_per_gas.pretty()),
        "difficulty" => Some(block.difficulty.pretty()),
        "extraData" | "extra_data" => Some(block.extra_data.pretty()),
        "gasLimit" | "gas_limit" => Some(block.gas_limit.pretty()),
        "gasUsed" | "gas_used" => Some(block.gas_used.pretty()),
        "hash" => Some(block.hash.pretty()),
        "logsBloom" | "logs_bloom" => Some(block.logs_bloom.pretty()),
        "miner" | "author" => Some(block.author.pretty()),
        "mixHash" | "mix_hash" => Some(block.mix_hash.pretty()),
        "nonce" => Some(block.nonce.pretty()),
        "number" => Some(block.number.pretty()),
        "parentHash" | "parent_hash" => Some(block.parent_hash.pretty()),
        "receiptsRoot" | "receipts_root" => Some(block.receipts_root.pretty()),
        "sealFields" | "seal_fields" => Some(block.seal_fields.pretty()),
        "sha3Uncles" | "sha_3_uncles" => Some(block.uncles_hash.pretty()),
        "size" => Some(block.size.pretty()),
        "stateRoot" | "state_root" => Some(block.state_root.pretty()),
        "timestamp" => Some(block.timestamp.pretty()),
        "totalDifficulty" | "total_difficult" => Some(block.total_difficulty.pretty()),
        _ => None,
    }
}

pub fn get_pretty_tx_receipt_attr(receipt: TransactionReceipt, attr: String) -> Option<String> {
    return match attr.as_str() {
        "blockHash" | "block_hash" => Some(receipt.block_hash.pretty()),
        "blockNumber" | "block_number" => Some(receipt.block_number.pretty()),
        "contractAddress" | "contract_address" => Some(receipt.contract_address.pretty()),
        "cumulativeGasUsed" | "cumulative_gas_used" => Some(receipt.cumulative_gas_used.pretty()),
        "effectiveGasPrice" | "effective_gas_price" => Some(receipt.effective_gas_price.pretty()),
        "gasUsed" | "gas_used" => Some(receipt.gas_used.pretty()),
        "logs" => Some(receipt.logs.pretty()),
        "logsBloom" | "logs_bloom" => Some(receipt.logs_bloom.pretty()),
        "root" => Some(receipt.root.pretty()),
        "status" => Some(receipt.status.pretty()),
        "transactionHash" | "transaction_hash" => Some(receipt.transaction_hash.pretty()),
        "transactionIndex" | "transaction_index" => Some(receipt.transaction_index.pretty()),
        "type" | "transaction_type" => Some(receipt.transaction_type.pretty()),
        _ => None,
    }
}

pub trait UIfmt {
    fn pretty(&self) -> String;
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

impl UIfmt for U64 {
    fn pretty(&self) -> String {
        self.to_string()
    }
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

impl UIfmt for H256 {
    fn pretty(&self) -> String {
        format!("{:#x}", self)
    }
}

impl UIfmt for H160 {
    fn pretty(&self) -> String {
        format!("{:#x}", self)
    }
}

impl UIfmt for Bytes {
    fn pretty(&self) -> String {
        format!("{:#x}", self)
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

fn tab_paragraph(paragraph: String) -> String {
    paragraph.lines().into_iter().fold("".to_string(), |acc, x| acc + "\t" + x + "\n")
}
#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use ethers_core::types::{Block, Transaction};
    #[test]
    fn print_block_w_txs() {
        let block = r#"{"number":"0x3","hash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","parentHash":"0x689c70c080ca22bc0e681694fa803c1aba16a69c8b6368fed5311d279eb9de90","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","transactionsRoot":"0x7270c1c4440180f2bd5215809ee3d545df042b67329499e1ab97eb759d31610d","stateRoot":"0x29f32984517a7d25607da485b23cefabfd443751422ca7e603395e1de9bc8a4b","receiptsRoot":"0x056b23fbba480696b65fe5a59b8f2148a1299103c4f57df839233af2cf4ca2d2","miner":"0x0000000000000000000000000000000000000000","difficulty":"0x0","totalDifficulty":"0x0","extraData":"0x","size":"0x3e8","gasLimit":"0x6691b7","gasUsed":"0x5208","timestamp":"0x5ecedbb9","transactions":[{"hash":"0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067","nonce":"0x2","blockHash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","blockNumber":"0x3","transactionIndex":"0x0","from":"0xfdcedc3bfca10ecb0890337fbdd1977aba84807a","to":"0xdca8ce283150ab773bcbeb8d38289bdb5661de1e","value":"0x0","gas":"0x15f90","gasPrice":"0x4a817c800","input":"0x","v":"0x25","r":"0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88","s":"0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e"}],"uncles":[]}"#;
        let _block: Block<Transaction> = serde_json::from_str(block).unwrap();
        let output =String::from("\nblockHash               0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972
blockNumber             3
from                    0xfdcedc3bfca10ecb0890337fbdd1977aba84807a
gas                     90000
gasPrice                20000000000
hash                    0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067
input                   0x
nonce                   2
r                       0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88
s                       0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e
to                      0xdca8ce283150ab773bcbeb8d38289bdb5661de1e
transactionIndex        0
v                       37
value                   0");
        let generated = _block.transactions[0].pretty();
        assert_eq!(generated.as_str(), output.as_str());
    }

    #[test]
    fn uifmt_option_u64() {
        let empty: Option<U64> = Option::None;
        assert_eq!("".to_string(), empty.pretty());
        assert_eq!("100".to_string(), U64::from_dec_str("100").unwrap().pretty());
        assert_eq!("100".to_string(), Option::from(U64::from_dec_str("100").unwrap()).pretty())
    }

    #[test]
    fn uifmt_option_h64() {
        let empty: Option<H256> = Option::None;
        assert_eq!("".to_string(), empty.pretty());
        H256::from_low_u64_be(100);
        assert_eq!(
            "0x0000000000000000000000000000000000000000000000000000000000000064",
            H256::from_low_u64_be(100).pretty()
        );
        assert_eq!(
            "0x0000000000000000000000000000000000000000000000000000000000000064",
            Option::Some(H256::from_low_u64_be(100)).pretty()
        );
    }
    #[test]
    fn uifmt_option_bytes() {
        let empty: Option<Bytes> = Option::None;
        assert_eq!("".to_string(), empty.pretty());
        assert_eq!(
            "0x0000000000000000000000000000000000000000000000000000000000000064".to_string(),
            Bytes::from_str("0x0000000000000000000000000000000000000000000000000000000000000064")
                .unwrap()
                .pretty()
        );
        assert_eq!(
            "0x0000000000000000000000000000000000000000000000000000000000000064".to_string(),
            Option::Some(
                Bytes::from_str(
                    "0x0000000000000000000000000000000000000000000000000000000000000064"
                )
                .unwrap()
            )
            .pretty()
        );
    }
    #[test]
    fn test_pretty_tx_attr() {
        let block = r#"{"number":"0x3","hash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","parentHash":"0x689c70c080ca22bc0e681694fa803c1aba16a69c8b6368fed5311d279eb9de90","mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000","nonce":"0x0000000000000000","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","transactionsRoot":"0x7270c1c4440180f2bd5215809ee3d545df042b67329499e1ab97eb759d31610d","stateRoot":"0x29f32984517a7d25607da485b23cefabfd443751422ca7e603395e1de9bc8a4b","receiptsRoot":"0x056b23fbba480696b65fe5a59b8f2148a1299103c4f57df839233af2cf4ca2d2","miner":"0x0000000000000000000000000000000000000000","difficulty":"0x0","totalDifficulty":"0x0","extraData":"0x","size":"0x3e8","gasLimit":"0x6691b7","gasUsed":"0x5208","timestamp":"0x5ecedbb9","transactions":[{"hash":"0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067","nonce":"0x2","blockHash":"0xda53da08ef6a3cbde84c33e51c04f68c3853b6a3731f10baa2324968eee63972","blockNumber":"0x3","transactionIndex":"0x0","from":"0xfdcedc3bfca10ecb0890337fbdd1977aba84807a","to":"0xdca8ce283150ab773bcbeb8d38289bdb5661de1e","value":"0x0","gas":"0x15f90","gasPrice":"0x4a817c800","input":"0x","v":"0x25","r":"0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88","s":"0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e"}],"uncles":[]}"#;
        let _block: Block<Transaction> = serde_json::from_str(block).unwrap();
        assert_eq!(None, get_pretty_tx_attr(_block.transactions[0].clone(), "".to_string()));
        assert_eq!(
            Some("3".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "blockNumber".to_string())
        );
        assert_eq!(
            Some("0xfdcedc3bfca10ecb0890337fbdd1977aba84807a".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "from".to_string())
        );
        assert_eq!(
            Some("90000".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "gas".to_string())
        );
        assert_eq!(
            Some("20000000000".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "gasPrice".to_string())
        );
        assert_eq!(
            Some("0xc3c5f700243de37ae986082fd2af88d2a7c2752a0c0f7b9d6ac47c729d45e067".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "hash".to_string())
        );
        assert_eq!(
            Some("0x".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "input".to_string())
        );
        assert_eq!(
            Some("2".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "nonce".to_string())
        );
        assert_eq!(
            Some("0x19f2694eb9113656dbea0b925e2e7ceb43df83e601c4116aee9c0dd99130be88".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "r".to_string())
        );
        assert_eq!(
            Some("0x73e5764b324a4f7679d890a198ba658ba1c8cd36983ff9797e10b1b89dbb448e".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "s".to_string())
        );
        assert_eq!(
            Some("0xdca8ce283150ab773bcbeb8d38289bdb5661de1e".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "to".to_string())
        );
        assert_eq!(
            Some("0".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "transactionIndex".to_string())
        );
        assert_eq!(
            Some("37".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "v".to_string())
        );
        assert_eq!(
            Some("0".to_string()),
            get_pretty_tx_attr(_block.transactions[0].clone(), "value".to_string())
        );
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

        let _block: Block<()> = serde_json::from_value(json).unwrap();

        assert_eq!(None, get_pretty_block_attr(_block.clone(), "".to_string()));
        assert_eq!(
            Some("7".to_string()),
            get_pretty_block_attr(_block.clone(), "baseFeePerGas".to_string())
        );
        assert_eq!(
            Some("163591".to_string()),
            get_pretty_block_attr(_block.clone(), "difficulty".to_string())
        );
        assert_eq!(
            Some("0x0000000000000000000000000000000000000000000000000000000000000000".to_string()),
            get_pretty_block_attr(_block.clone(), "extraData".to_string())
        );
        assert_eq!(
            Some("653145".to_string()),
            get_pretty_block_attr(_block.clone(), "gasLimit".to_string())
        );
        assert_eq!(
            Some("653145".to_string()),
            get_pretty_block_attr(_block.clone(), "gasUsed".to_string())
        );
        assert_eq!(
            Some("0x0e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d1527331".to_string()),
            get_pretty_block_attr(_block.clone(), "hash".to_string())
        );
        assert_eq!(Some("0x0e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d15273310e670ec64341771606e55d6b4ca35a1a6b75ee3d5145a99d05921026d1527331".to_string()),  get_pretty_block_attr(_block.clone(), "logsBloom".to_string()));
        assert_eq!(
            Some("0x0000000000000000000000000000000000000001".to_string()),
            get_pretty_block_attr(_block.clone(), "miner".to_string())
        );
        assert_eq!(
            Some("0x1010101010101010101010101010101010101010101010101010101010101010".to_string()),
            get_pretty_block_attr(_block.clone(), "mixHash".to_string())
        );
        assert_eq!(
            Some("0".to_string()),
            get_pretty_block_attr(_block.clone(), "nonce".to_string())
        );
        assert_eq!(
            Some("436".to_string()),
            get_pretty_block_attr(_block.clone(), "number".to_string())
        );
        assert_eq!(
            Some("0x9646252be9520f6e71339a8df9c55e4d7619deeb018d2a3f2d21fc165dde5eb5".to_string()),
            get_pretty_block_attr(_block.clone(), "parentHash".to_string())
        );
        assert_eq!(
            Some("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421".to_string()),
            get_pretty_block_attr(_block.clone(), "receiptsRoot".to_string())
        );
        assert_eq!(
            Some("0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347".to_string()),
            get_pretty_block_attr(_block.clone(), "sha3Uncles".to_string())
        );
        assert_eq!(
            Some("163591".to_string()),
            get_pretty_block_attr(_block.clone(), "size".to_string())
        );
        assert_eq!(
            Some("0xd5855eb08b3387c0af375e9cdb6acfc05eb8f519e419b874b6ff2ffda7ed1dff".to_string()),
            get_pretty_block_attr(_block.clone(), "stateRoot".to_string())
        );
        assert_eq!(
            Some("1424182926".to_string()),
            get_pretty_block_attr(_block.clone(), "timestamp".to_string())
        );
        assert_eq!(
            Some("163591".to_string()),
            get_pretty_block_attr(_block, "totalDifficulty".to_string())
        );
    }
}
