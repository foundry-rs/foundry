use crate::{
    error::RpcError,
    eth::{call::CallRequest, filter::Filter, transaction::EthTransactionRequest},
    types::Index,
};
use ethers_core::{
    abi::ethereum_types::H64,
    types::{Address, BlockNumber, Bytes, Transaction, TxHash, H256, U256},
};
use serde::{Deserialize, Serialize};

pub mod block;
pub mod call;
pub mod filter;
pub mod receipt;
pub mod transaction;
pub mod trie;
pub mod utils;

/// Represents ethereum JSON-RPC API
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum EthRequest {
    #[serde(rename = "eth_chainId")]
    EthChainId,

    #[serde(rename = "eth_gasPrice")]
    EthGasPrice,

    #[serde(rename = "eth_accounts")]
    EthAccounts,

    #[serde(rename = "eth_blockNumber")]
    EthBlockNumber,

    #[serde(rename = "eth_getBalance")]
    EthGetBalance(Address, Option<BlockNumber>),

    #[serde(rename = "eth_getStorageAt")]
    EthGetStorageAt(Address, U256, Option<BlockNumber>),

    #[serde(rename = "eth_getBlockByHash")]
    EthGetBlockByHash(H256, bool),

    #[serde(rename = "eth_getBlockByNumber")]
    EthGetBlockByNumber(BlockNumber, bool),

    #[serde(rename = "eth_getTransactionCount")]
    EthGetTransactionCount(Address, Option<BlockNumber>),

    #[serde(rename = "eth_getBlockTransactionCountByHash")]
    EthGetTransactionCountByHash(H256),

    #[serde(rename = "eth_getBlockTransactionCountByNumber")]
    EthGetTransactionCountByNumber(BlockNumber),

    #[serde(rename = "eth_getUncleCountByBlockHash")]
    EthGetUnclesCountByHash(H256),

    #[serde(rename = "eth_getUncleCountByBlockNumber")]
    EthGetUnclesCountByNumber(BlockNumber),

    #[serde(rename = "eth_getCode")]
    EthGetCodeAt(Address, Option<BlockNumber>),

    #[serde(rename = "eth_sendTransaction")]
    EthSendTransaction(Box<EthTransactionRequest>),

    #[serde(rename = "eth_sendTransaction")]
    EthSendRawTransaction(Bytes),

    #[serde(rename = "eth_call")]
    EthCall(CallRequest, Option<BlockNumber>),

    #[serde(rename = "eth_estimateGas")]
    EthEstimateGas(CallRequest, Option<BlockNumber>),

    #[serde(rename = "eth_getTransactionByHash")]
    EthGetTransactionByHash(TxHash),

    #[serde(rename = "eth_getTransactionByBlockHashAndIndex")]
    EthGetTransactionByBlockHashAndIndex(TxHash, Index),

    #[serde(rename = "eth_getTransactionByBlockNumberAndIndex")]
    EthGetTransactionByBlockNumberAndIndex(BlockNumber, Index),

    #[serde(rename = "eth_getTransactionReceipt")]
    EthGetTransactionReceipt(H256),

    #[serde(rename = "eth_getUncleByBlockHashAndIndex")]
    EthGetUncleByBlockHashAndIndex(H256, Index),

    #[serde(rename = "eth_getUncleByBlockNumberAndIndex")]
    EthGetUncleByBlockNumberAndIndex(BlockNumber, Index),

    #[serde(rename = "eth_getLogs")]
    EthGetLogs(Filter),

    #[serde(rename = "eth_getWork")]
    EthGetWork,

    #[serde(rename = "eth_submitWork")]
    EthSubmitWork(H64, H256, H256),

    #[serde(rename = "eth_submitHashrate")]
    EthSubmitHashRate(U256, H256),

    #[serde(rename = "eth_feeHistory")]
    EthFeeHistory(U256, BlockNumber, Option<Vec<f64>>),
}

#[allow(unused)]
mod sequence {
    use serde::{
        de::DeserializeOwned, ser::SerializeSeq, Deserialize, Deserializer, Serialize, Serializer,
    };

    #[allow(unused)]
    pub fn serialize<S, T>(val: &T, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        let mut seq = s.serialize_seq(Some(1))?;
        seq.serialize_element(val)?;
        seq.end()
    }

    pub fn deserialize<'de, T, D>(d: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let mut seq = Vec::<T>::deserialize(d)?;
        if seq.len() != 1 {
            return Err(serde::de::Error::custom(format!(
                "expected params sequence with length 1 but got {}",
                seq.len()
            )))
        }
        Ok(seq.remove(0))
    }
}

#[derive(Serialize)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum EthResponse {
    EthGetBalance(U256),
    EthGetTransactionByHash(Box<Option<Transaction>>),
    EthSendTransaction(Result<TxHash, RpcError>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    #[test]
    fn test_serde_eth_storage() {
        let s = r#"{"method": "eth_getStorageAt", "params": ["0x295a70b2de5e3953354a6a8344e616ed314d7251", "0x0", "latest"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        dbg!(value.clone());
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_eth_balance() {
        let s = r#"{"method": "eth_getBalance", "params": ["0x295a70b2de5e3953354a6a8344e616ed314d7251", "latest"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();

        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_res() {
        let val = EthResponse::EthGetBalance(U256::from(123u64));
        let _ser = serde_json::to_string(&val).unwrap();

        let val = EthResponse::EthGetTransactionByHash(Box::new(Some(Transaction::default())));
        let _ser = serde_json::to_string(&val).unwrap();
        let val = EthResponse::EthGetTransactionByHash(Box::new(None));
        let _ser = serde_json::to_string(&val).unwrap();

        let val = EthResponse::EthSendTransaction(Ok(TxHash::default()));
        let _ser = serde_json::to_string(&val).unwrap();
        let val = EthResponse::EthSendTransaction(Err(RpcError::parse_error()));
        let _ser = serde_json::to_string(&val).unwrap();
    }
}
