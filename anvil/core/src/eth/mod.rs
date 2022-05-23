use crate::{
    eth::{
        call::CallRequest,
        filter::Filter,
        subscription::{SubscriptionId, SubscriptionKind, SubscriptionParams},
        transaction::EthTransactionRequest,
    },
    types::{EvmMineOptions, Forking, GethDebugTracingOptions, Index},
};
use ethers_core::{
    abi::ethereum_types::H64,
    types::{Address, BlockId, BlockNumber, Bytes, TxHash, H256, U256},
};
use serde::{Deserialize, Deserializer};

pub mod block;
pub mod call;
pub mod filter;
pub mod receipt;
pub mod subscription;
pub mod transaction;
pub mod trie;
pub mod utils;

/// Represents ethereum JSON-RPC API
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum EthRequest {
    #[serde(rename = "web3_clientVersion", with = "empty_params")]
    Web3ClientVersion(()),

    #[serde(rename = "web3_sha3", with = "sequence")]
    Web3Sha3(Bytes),

    #[serde(rename = "eth_chainId", with = "empty_params")]
    EthChainId(()),

    #[serde(rename = "eth_networkId", alias = "net_version", with = "empty_params")]
    EthNetworkId(()),

    #[serde(rename = "eth_gasPrice", with = "empty_params")]
    EthGasPrice(()),

    #[serde(rename = "eth_accounts", alias = "eth_requestAccounts", with = "empty_params")]
    EthAccounts(()),

    #[serde(rename = "eth_blockNumber", with = "empty_params")]
    EthBlockNumber(()),

    #[serde(rename = "eth_getBalance")]
    EthGetBalance(Address, Option<BlockId>),

    #[serde(rename = "eth_getStorageAt")]
    EthGetStorageAt(Address, U256, Option<BlockId>),

    #[serde(rename = "eth_getBlockByHash")]
    EthGetBlockByHash(H256, bool),

    #[serde(rename = "eth_getBlockByNumber")]
    EthGetBlockByNumber(BlockNumber, bool),

    #[serde(rename = "eth_getTransactionCount")]
    EthGetTransactionCount(Address, Option<BlockId>),

    #[serde(rename = "eth_getBlockTransactionCountByHash")]
    EthGetTransactionCountByHash(H256),

    #[serde(rename = "eth_getBlockTransactionCountByNumber")]
    EthGetTransactionCountByNumber(BlockNumber),

    #[serde(rename = "eth_getUncleCountByBlockHash")]
    EthGetUnclesCountByHash(H256),

    #[serde(rename = "eth_getUncleCountByBlockNumber")]
    EthGetUnclesCountByNumber(BlockNumber),

    #[serde(rename = "eth_getCode")]
    EthGetCodeAt(Address, Option<BlockId>),

    /// The sign method calculates an Ethereum specific signature with:
    #[serde(rename = "eth_sign")]
    EthSign(Address, Bytes),

    #[serde(rename = "eth_sendTransaction", with = "sequence")]
    EthSendTransaction(Box<EthTransactionRequest>),

    #[serde(rename = "eth_sendRawTransaction", with = "sequence")]
    EthSendRawTransaction(Bytes),

    #[serde(rename = "eth_call")]
    EthCall(CallRequest, #[serde(default)] Option<BlockId>),

    #[serde(rename = "eth_createAccessList")]
    EthCreateAccessList(CallRequest, #[serde(default)] Option<BlockId>),

    #[serde(rename = "eth_estimateGas")]
    EthEstimateGas(CallRequest, #[serde(default)] Option<BlockId>),

    #[serde(rename = "eth_getTransactionByHash", with = "sequence")]
    EthGetTransactionByHash(TxHash),

    #[serde(rename = "eth_getTransactionByBlockHashAndIndex")]
    EthGetTransactionByBlockHashAndIndex(TxHash, Index),

    #[serde(rename = "eth_getTransactionByBlockNumberAndIndex")]
    EthGetTransactionByBlockNumberAndIndex(BlockNumber, Index),

    #[serde(rename = "eth_getTransactionReceipt", with = "sequence")]
    EthGetTransactionReceipt(H256),

    #[serde(rename = "eth_getUncleByBlockHashAndIndex")]
    EthGetUncleByBlockHashAndIndex(H256, Index),

    #[serde(rename = "eth_getUncleByBlockNumberAndIndex")]
    EthGetUncleByBlockNumberAndIndex(BlockNumber, Index),

    #[serde(rename = "eth_getLogs", with = "sequence")]
    EthGetLogs(Filter),

    /// Creates a filter object, based on filter options, to notify when the state changes (logs).
    #[serde(rename = "eth_newFilter", with = "sequence")]
    EthNewFilter(Filter),

    /// Polling method for a filter, which returns an array of logs which occurred since last poll.
    #[serde(rename = "eth_getFilterChanges", with = "sequence")]
    EthGetFilterChanges(String),

    /// Creates a filter in the node, to notify when a new block arrives.
    /// To check if the state has changed, call `eth_getFilterChanges`.
    #[serde(rename = "eth_newBlockFilter", with = "empty_params")]
    EthNewBlockFilter(()),

    /// Creates a filter in the node, to notify when new pending transactions arrive.
    /// To check if the state has changed, call `eth_getFilterChanges`.
    #[serde(rename = "eth_newPendingTransactionFilter", with = "empty_params")]
    EthNewPendingTransactionFilter(()),

    /// Returns an array of all logs matching filter with given id.
    #[serde(rename = "eth_getFilterLogs", with = "sequence")]
    EthGetFilterLogs(String),

    /// Removes the filter, returns true if the filter was installed
    #[serde(rename = "eth_uninstallFilter", with = "sequence")]
    EthUninstallFilter(String),

    #[serde(rename = "eth_getWork", with = "empty_params")]
    EthGetWork(()),

    #[serde(rename = "eth_submitWork")]
    EthSubmitWork(H64, H256, H256),

    #[serde(rename = "eth_submitHashrate")]
    EthSubmitHashRate(U256, H256),

    #[serde(rename = "eth_feeHistory")]
    EthFeeHistory(
        #[serde(deserialize_with = "deserialize_number")] U256,
        BlockNumber,
        #[serde(default)] Vec<f64>,
    ),

    /// geth's `debug_traceTransaction`  endpoint
    #[serde(rename = "debug_traceTransaction")]
    DebugTraceTransaction(H256, #[serde(default)] GethDebugTracingOptions),

    /// Trace transaction endpoint for parity's `trace_transaction`
    #[serde(rename = "trace_transaction", with = "sequence")]
    TraceTransaction(H256),

    /// Trace transaction endpoint for parity's `trace_block`
    #[serde(rename = "trace_block", with = "sequence")]
    TraceBlock(BlockNumber),

    // Custom endpoints, they're not extracted to a separate type out of serde convenience
    /// send transactions impersonating specific account and contract addresses.
    #[serde(
        rename = "anvil_impersonateAccount",
        alias = "hardhat_impersonateAccount",
        with = "sequence"
    )]
    ImpersonateAccount(Address),
    /// Stops impersonating an account if previously set with `anvil_impersonateAccount`
    #[serde(rename = "anvil_stopImpersonatingAccount", alias = "hardhat_stopImpersonatingAccount")]
    StopImpersonatingAccount,
    /// Returns true if automatic mining is enabled, and false.
    #[serde(rename = "anvil_getAutomine", alias = "hardhat_getAutomine", with = "empty_params")]
    GetAutoMine(()),
    /// Mines a series of blocks
    #[serde(rename = "anvil_mine", alias = "hardhat_mine")]
    Mine(
        /// Number of blocks to mine, if not set `1` block is mined
        #[serde(default, deserialize_with = "deserialize_number_opt")]
        Option<U256>,
        /// The time interval between each block in seconds, defaults to `1` seconds
        /// The interval is applied only to blocks mined in the given method invocation, not to
        /// blocks mined afterwards. Set this to `0` to instantly mine _all_ blocks
        #[serde(default, deserialize_with = "deserialize_number_opt")]
        Option<U256>,
    ),

    /// Enables or disables, based on the single boolean argument, the automatic mining of new
    /// blocks with each new transaction submitted to the network.
    #[serde(rename = "evm_setAutomine", with = "sequence")]
    SetAutomine(bool),

    /// Sets the mining behavior to interval with the given interval (seconds)
    #[serde(rename = "evm_setIntervalMining", with = "sequence")]
    SetIntervalMining(u64),

    /// Removes transactions from the pool
    #[serde(
        rename = "anvil_dropTransaction",
        alias = "hardhat_dropTransaction",
        with = "sequence"
    )]
    DropTransaction(H256),

    /// Reset the fork to a fresh forked state, and optionally update the fork config
    #[serde(rename = "anvil_reset", alias = "hardhat_reset", with = "sequence")]
    Reset(#[serde(default)] Option<Forking>),

    /// Sets the backend rpc url
    #[serde(rename = "anvil_setRpcUrl", with = "sequence")]
    SetRpcUrl(String),

    /// Modifies the balance of an account.
    #[serde(rename = "anvil_setBalance", alias = "hardhat_setBalance")]
    SetBalance(Address, #[serde(deserialize_with = "deserialize_number")] U256),

    /// Sets the code of a contract
    #[serde(rename = "anvil_setCode", alias = "hardhat_setCode")]
    SetCode(Address, Bytes),

    /// Sets the nonce of an address
    #[serde(rename = "anvil_setNonce", alias = "hardhat_setNonce")]
    SetNonce(Address, #[serde(deserialize_with = "deserialize_number")] U256),

    /// Writes a single slot of the account's storage
    #[serde(rename = "anvil_setStorageAt", alias = "hardhat_setStorageAt")]
    SetStorageAt(
        Address,
        /// slot
        U256,
        /// value
        U256,
    ),

    /// Sets the coinbase address
    #[serde(rename = "anvil_setCoinbase", alias = "hardhat_setCoinbase", with = "sequence")]
    SetCoinbase(Address),

    /// Enable or disable logging
    #[serde(
        rename = "anvil_setLoggingEnabled",
        alias = "hardhat_setLoggingEnabled",
        with = "sequence"
    )]
    SetLogging(bool),

    /// Set the minimum gas price for the node
    #[serde(rename = "anvil_setMinGasPrice", alias = "hardhat_setMinGasPrice", with = "sequence")]
    SetMinGasPrice(#[serde(deserialize_with = "deserialize_number")] U256),

    /// Sets the base fee of the next block
    #[serde(
        rename = "anvil_setNextBlockBaseFeePerGas",
        alias = "hardhat_setNextBlockBaseFeePerGas",
        with = "sequence"
    )]
    SetNextBlockBaseFeePerGas(#[serde(deserialize_with = "deserialize_number")] U256),

    // Ganache compatible calls
    /// Snapshot the state of the blockchain at the current block.
    #[serde(rename = "evm_snapshot", with = "empty_params")]
    EvmSnapshot(()),

    /// Revert the state of the blockchain to a previous snapshot.
    /// Takes a single parameter, which is the snapshot id to revert to.
    #[serde(rename = "evm_revert", with = "sequence")]
    EvmRevert(#[serde(deserialize_with = "deserialize_number")] U256),

    /// Jump forward in time by the given amount of time, in seconds.
    #[serde(rename = "evm_increaseTime", with = "sequence")]
    EvmIncreaseTime(#[serde(deserialize_with = "deserialize_number")] U256),

    /// Similar to `evm_increaseTime` but takes the exact timestamp that you want in the next block
    #[serde(rename = "evm_setNextBlockTimestamp", with = "sequence")]
    EvmSetNextBlockTimeStamp(u64),

    /// Mine a single block
    #[serde(rename = "evm_mine")]
    EvmMine(#[serde(default)] Option<Params<EvmMineOptions>>),

    /// Execute a transaction regardless of signature status
    #[serde(rename = "eth_sendUnsignedTransaction", with = "sequence")]
    EthSendUnsignedTransaction(Box<EthTransactionRequest>),

    /// Turn on call traces for transactions that are returned to the user when they execute a
    /// transaction (instead of just txhash/receipt)
    #[serde(rename = "anvil_enableTraces", with = "empty_params")]
    EnableTraces(()),

    /// Returns the number of transactions currently pending for inclusion in the next block(s), as
    /// well as the ones that are being scheduled for future execution only.
    /// Ref: [Here](https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_status)
    #[serde(rename = "txpool_status", with = "empty_params")]
    TxPoolStatus(()),

    /// Returns a summary of all the transactions currently pending for inclusion in the next
    /// block(s), as well as the ones that are being scheduled for future execution only.
    /// Ref: [Here](https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_inspect)
    #[serde(rename = "txpool_inspect", with = "empty_params")]
    TxPoolInspect(()),

    /// Returns the details of all transactions currently pending for inclusion in the next
    /// block(s), as well as the ones that are being scheduled for future execution only.
    /// Ref: [Here](https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_content)
    #[serde(rename = "txpool_content", with = "empty_params")]
    TxPoolContent(()),
}

/// Represents ethereum JSON-RPC API
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum EthPubSub {
    /// Subscribe to an eth subscription
    #[serde(rename = "eth_subscribe")]
    EthSubscribe(SubscriptionKind, #[serde(default)] SubscriptionParams),

    /// Unsubscribe from an eth subscription
    #[serde(rename = "eth_unsubscribe", with = "sequence")]
    EthUnSubscribe(SubscriptionId),
}

/// Container type for either a request or a pub sub
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum EthRpcCall {
    Request(Box<EthRequest>),
    PubSub(EthPubSub),
}

fn deserialize_number<'de, D>(deserializer: D) -> Result<U256, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Numeric {
        U256(U256),
        Num(u64),
    }

    let num = match Numeric::deserialize(deserializer)? {
        Numeric::U256(n) => n,
        Numeric::Num(n) => U256::from(n),
    };

    Ok(num)
}

fn deserialize_number_opt<'de, D>(deserializer: D) -> Result<Option<U256>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Numeric {
        U256(U256),
        Num(u64),
    }

    let num = match Option::<Numeric>::deserialize(deserializer)? {
        Some(Numeric::U256(n)) => Some(n),
        Some(Numeric::Num(n)) => Some(U256::from(n)),
        _ => None,
    };

    Ok(num)
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Params<T> {
    pub params: T,
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

/// A module that deserializes `[]` optionally
mod empty_params {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(d: D) -> Result<(), D::Error>
    where
        D: Deserializer<'de>,
    {
        let seq = Option::<Vec<()>>::deserialize(d)?.unwrap_or_default();
        if !seq.is_empty() {
            return Err(serde::de::Error::custom(format!(
                "expected params sequence with length 0 but got {}",
                seq.len()
            )))
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web3_client_version() {
        let s = r#"{"method": "web3_clientVersion", "params":[]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_web3_sha3() {
        let s = r#"{"method": "web3_sha3", "params":["0x68656c6c6f20776f726c64"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_eth_accounts() {
        let s = r#"{"method": "eth_accounts", "params":[]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_eth_network_id() {
        let s = r#"{"method": "eth_networkId", "params":[]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_eth_chain_id() {
        let s = r#"{"method": "eth_chainId", "params":[]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_eth_block_number() {
        let s = r#"{"method": "eth_blockNumber", "params":[]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_impersonate_account() {
        let s = r#"{"method": "anvil_impersonateAccount", "params": ["0xd84de507f3fada7df80908082d3239466db55a71"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_stop_impersonate_account() {
        let s = r#"{"method": "anvil_stopImpersonatingAccount"}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_get_automine() {
        let s = r#"{"method": "anvil_getAutomine", "params": []}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_mine() {
        let s = r#"{"method": "anvil_mine", "params": []}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Mine(num, time) => {
                assert!(num.is_none());
                assert!(time.is_none());
            }
            _ => unreachable!(),
        }
        let s =
            r#"{"method": "anvil_mine", "params": ["0xd84de507f3fada7df80908082d3239466db55a71"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Mine(num, time) => {
                assert!(num.is_some());
                assert!(time.is_none());
            }
            _ => unreachable!(),
        }
        let s = r#"{"method": "anvil_mine", "params": ["0xd84de507f3fada7df80908082d3239466db55a71", "0xd84de507f3fada7df80908082d3239466db55a71"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Mine(num, time) => {
                assert!(num.is_some());
                assert!(time.is_some());
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_custom_auto_mine() {
        let s = r#"{"method": "evm_setAutomine", "params": [false]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_interval_mining() {
        let s = r#"{"method": "evm_setIntervalMining", "params": [100]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_drop_tx() {
        let s = r#"{"method": "anvil_dropTransaction", "params": ["0x4a3b0fce2cb9707b0baa68640cf2fe858c8bb4121b2a8cb904ff369d38a560ff"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_reset() {
        let s = r#"{"method": "anvil_reset", "params": [ { "forking": {
                "jsonRpcUrl": "https://eth-mainnet.alchemyapi.io/v2/<key>",
                "blockNumber": 11095000
        }}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Reset(forking) => {
                assert_eq!(
                    forking,
                    Some(Forking {
                        json_rpc_url: Some(
                            "https://eth-mainnet.alchemyapi.io/v2/<key>".to_string()
                        ),
                        block_number: Some(11095000)
                    })
                )
            }
            _ => unreachable!(),
        }

        let s = r#"{"method": "anvil_reset", "params": [ { "forking": {
                "jsonRpcUrl": "https://eth-mainnet.alchemyapi.io/v2/<key>"
        }}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Reset(forking) => {
                assert_eq!(
                    forking,
                    Some(Forking {
                        json_rpc_url: Some(
                            "https://eth-mainnet.alchemyapi.io/v2/<key>".to_string()
                        ),
                        block_number: None
                    })
                )
            }
            _ => unreachable!(),
        }

        let s = r#"{"method":"anvil_reset","params":[{"jsonRpcUrl": "http://localhost:8545", "blockNumber": 14000000}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Reset(forking) => {
                assert_eq!(
                    forking,
                    Some(Forking {
                        json_rpc_url: Some("http://localhost:8545".to_string()),
                        block_number: Some(14000000)
                    })
                )
            }
            _ => unreachable!(),
        }

        let s = r#"{"method":"anvil_reset","params":[{ "blockNumber": 14000000}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Reset(forking) => {
                assert_eq!(
                    forking,
                    Some(Forking { json_rpc_url: None, block_number: Some(14000000) })
                )
            }
            _ => unreachable!(),
        }

        let s = r#"{"method":"anvil_reset","params":[{"jsonRpcUrl": "http://localhost:8545"}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Reset(forking) => {
                assert_eq!(
                    forking,
                    Some(Forking {
                        json_rpc_url: Some("http://localhost:8545".to_string()),
                        block_number: None
                    })
                )
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_custom_set_balance() {
        let s = r#"{"method": "anvil_setBalance", "params": ["0xd84de507f3fada7df80908082d3239466db55a71", "0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "anvil_setBalance", "params": ["0xd84de507f3fada7df80908082d3239466db55a71", 1337]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_set_code() {
        let s = r#"{"method": "anvil_setCode", "params": ["0xd84de507f3fada7df80908082d3239466db55a71", "0x0123456789abcdef"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_set_nonce() {
        let s = r#"{"method": "anvil_setNonce", "params": ["0xd84de507f3fada7df80908082d3239466db55a71", "0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_set_storage_at() {
        let s = r#"{"method": "anvil_setStorageAt", "params": ["0x295a70b2de5e3953354a6a8344e616ed314d7251", "0x0", "0x00"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_coinbase() {
        let s = r#"{"method": "anvil_setCoinbase", "params": ["0x295a70b2de5e3953354a6a8344e616ed314d7251"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_logging() {
        let s = r#"{"method": "anvil_setLoggingEnabled", "params": [false]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_min_gas_price() {
        let s = r#"{"method": "anvil_setMinGasPrice", "params": ["0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_next_block_base_fee() {
        let s = r#"{"method": "anvil_setNextBlockBaseFeePerGas", "params": ["0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_snapshot() {
        let s = r#"{"method": "evm_snapshot", "params": [] }"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_revert() {
        let s = r#"{"method": "evm_revert", "params": ["0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_increase_time() {
        let s = r#"{"method": "evm_increaseTime", "params": ["0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_next_timestamp() {
        let s = r#"{"method": "evm_setNextBlockTimestamp", "params": [100]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_evm_mine() {
        let s = r#"{"method": "evm_mine", "params": [100]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method": "evm_mine", "params": [{
            "timestamp": 100,
            "blocks": 100
        }]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::EvmMine(params) => {
                assert_eq!(
                    params.unwrap().params,
                    EvmMineOptions::Options { timestamp: Some(100), blocks: Some(100) }
                )
            }
            _ => unreachable!(),
        }

        let s = r#"{"method": "evm_mine"}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();

        match req {
            EthRequest::EvmMine(params) => {
                assert!(params.is_none())
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_eth_get_logs() {
        let s = r#"{"jsonrpc":"2.0","method":"eth_getLogs","params":[{"topics":["0x000000000000000000000000a94f5374fce5edbc8e2a8697c15331677e6ebf0b"]}],"id":74}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_eth_new_filter() {
        let s = r#"{"method": "eth_newFilter", "params": [{"topics":["0x000000000000000000000000a94f5374fce5edbc8e2a8697c15331677e6ebf0b"]}],"id":73}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_eth_unsubscribe() {
        let s = r#"{"id": 1, "method": "eth_unsubscribe", "params": ["0x9cef478923ff08bf67fde6c64013158d"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthPubSub>(value).unwrap();
    }

    #[test]
    fn test_serde_eth_subscribe() {
        let s = r#"{"id": 1, "method": "eth_subscribe", "params": ["newHeads"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthPubSub>(value).unwrap();

        let s = r#"{"id": 1, "method": "eth_subscribe", "params": ["logs", {"address": "0x8320fe7702b96808f7bbc0d4a888ed1468216cfd", "topics": ["0xd78a0cb8bb633d06981248b816e7bd33c2a35a6089241d099fa519e361cab902"]}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthPubSub>(value).unwrap();

        let s = r#"{"id": 1, "method": "eth_subscribe", "params": ["newPendingTransactions"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthPubSub>(value).unwrap();

        let s = r#"{"id": 1, "method": "eth_subscribe", "params": ["syncing"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthPubSub>(value).unwrap();
    }

    #[test]
    fn test_serde_debug_trace_transaction() {
        let s = r#"{"method": "debug_traceTransaction", "params": ["0x4a3b0fce2cb9707b0baa68640cf2fe858c8bb4121b2a8cb904ff369d38a560ff"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "debug_traceTransaction", "params": ["0x4a3b0fce2cb9707b0baa68640cf2fe858c8bb4121b2a8cb904ff369d38a560ff", {}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "debug_traceTransaction", "params": ["0x4a3b0fce2cb9707b0baa68640cf2fe858c8bb4121b2a8cb904ff369d38a560ff", {"disableStorage": true}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_eth_storage() {
        let s = r#"{"method": "eth_getStorageAt", "params": ["0x295a70b2de5e3953354a6a8344e616ed314d7251", "0x0", "latest"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_eth_call() {
        let req = r#"{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}"#;
        let _req = serde_json::from_str::<CallRequest>(req).unwrap();

        let s = r#"{"method": "eth_call", "params":  [{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"},"latest"]}"#;
        let _req = serde_json::from_str::<EthRequest>(s).unwrap();

        let s = r#"{"method": "eth_call", "params":  [{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}]}"#;
        let _req = serde_json::from_str::<EthRequest>(s).unwrap();

        let s = r#"{"method": "eth_call", "params":  [{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}, { "blockNumber": "latest" }]}"#;
        let _req = serde_json::from_str::<EthRequest>(s).unwrap();

        let s = r#"{"method": "eth_call", "params":  [{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}, { "blockNumber": "0x0" }]}"#;
        let _req = serde_json::from_str::<EthRequest>(s).unwrap();

        let s = r#"{"method": "eth_call", "params":  [{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}, { "blockHash": "0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3" }]}"#;
        let _req = serde_json::from_str::<EthRequest>(s).unwrap();
    }

    #[test]
    fn test_serde_eth_balance() {
        let s = r#"{"method": "eth_getBalance", "params": ["0x295a70b2de5e3953354a6a8344e616ed314d7251", "latest"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();

        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }
}
