use crate::eth::subscription::SubscriptionId;
use alloy_primitives::{Address, Bytes, TxHash, B256, B64, U256};
use alloy_rpc_types::{
    anvil::{Forking, MineOptions},
    pubsub::{Params as SubscriptionParams, SubscriptionKind},
    request::TransactionRequest,
    state::StateOverride,
    trace::{
        filter::TraceFilter,
        geth::{GethDebugTracingCallOptions, GethDebugTracingOptions},
    },
    BlockId, BlockNumberOrTag as BlockNumber, Filter, Index,
};
use alloy_serde::WithOtherFields;

pub mod block;
pub mod proof;
pub mod subscription;
pub mod transaction;
pub mod trie;
pub mod utils;

#[cfg(feature = "serde")]
pub mod serde_helpers;

#[cfg(feature = "serde")]
use self::serde_helpers::*;

#[cfg(feature = "serde")]
use foundry_common::serde_helpers::{
    deserialize_number, deserialize_number_opt, deserialize_number_seq,
};

/// Wrapper type that ensures the type is named `params`
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
pub struct Params<T: Default> {
    #[cfg_attr(feature = "serde", serde(default))]
    pub params: T,
}

/// Represents ethereum JSON-RPC API
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "method", content = "params"))]
pub enum EthRequest {
    #[cfg_attr(feature = "serde", serde(rename = "web3_clientVersion", with = "empty_params"))]
    Web3ClientVersion(()),

    #[cfg_attr(feature = "serde", serde(rename = "web3_sha3", with = "sequence"))]
    Web3Sha3(Bytes),

    #[cfg_attr(feature = "serde", serde(rename = "eth_chainId", with = "empty_params"))]
    EthChainId(()),

    #[cfg_attr(
        feature = "serde",
        serde(rename = "eth_networkId", alias = "net_version", with = "empty_params")
    )]
    EthNetworkId(()),

    #[cfg_attr(feature = "serde", serde(rename = "net_listening", with = "empty_params"))]
    NetListening(()),

    #[cfg_attr(feature = "serde", serde(rename = "eth_gasPrice", with = "empty_params"))]
    EthGasPrice(()),

    #[cfg_attr(
        feature = "serde",
        serde(rename = "eth_maxPriorityFeePerGas", with = "empty_params")
    )]
    EthMaxPriorityFeePerGas(()),

    #[cfg_attr(feature = "serde", serde(rename = "eth_blobBaseFee", with = "empty_params"))]
    EthBlobBaseFee(()),

    #[cfg_attr(
        feature = "serde",
        serde(rename = "eth_accounts", alias = "eth_requestAccounts", with = "empty_params")
    )]
    EthAccounts(()),

    #[cfg_attr(feature = "serde", serde(rename = "eth_blockNumber", with = "empty_params"))]
    EthBlockNumber(()),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getBalance"))]
    EthGetBalance(Address, Option<BlockId>),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getAccount"))]
    EthGetAccount(Address, Option<BlockId>),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getStorageAt"))]
    EthGetStorageAt(Address, U256, Option<BlockId>),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getBlockByHash"))]
    EthGetBlockByHash(B256, bool),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getBlockByNumber"))]
    EthGetBlockByNumber(
        #[cfg_attr(
            feature = "serde",
            serde(deserialize_with = "lenient_block_number::lenient_block_number")
        )]
        BlockNumber,
        bool,
    ),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getTransactionCount"))]
    EthGetTransactionCount(Address, Option<BlockId>),

    #[cfg_attr(
        feature = "serde",
        serde(rename = "eth_getBlockTransactionCountByHash", with = "sequence")
    )]
    EthGetTransactionCountByHash(B256),

    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "eth_getBlockTransactionCountByNumber",
            deserialize_with = "lenient_block_number::lenient_block_number_seq"
        )
    )]
    EthGetTransactionCountByNumber(BlockNumber),

    #[cfg_attr(
        feature = "serde",
        serde(rename = "eth_getUncleCountByBlockHash", with = "sequence")
    )]
    EthGetUnclesCountByHash(B256),

    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "eth_getUncleCountByBlockNumber",
            deserialize_with = "lenient_block_number::lenient_block_number_seq"
        )
    )]
    EthGetUnclesCountByNumber(BlockNumber),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getCode"))]
    EthGetCodeAt(Address, Option<BlockId>),

    /// Returns the account and storage values of the specified account including the Merkle-proof.
    /// This call can be used to verify that the data you are pulling from is not tampered with.
    #[cfg_attr(feature = "serde", serde(rename = "eth_getProof"))]
    EthGetProof(Address, Vec<B256>, Option<BlockId>),

    /// The sign method calculates an Ethereum specific signature with:
    #[cfg_attr(feature = "serde", serde(rename = "eth_sign"))]
    EthSign(Address, Bytes),

    /// The sign method calculates an Ethereum specific signature, equivalent to eth_sign:
    /// <https://docs.metamask.io/wallet/reference/personal_sign/>
    #[cfg_attr(feature = "serde", serde(rename = "personal_sign"))]
    PersonalSign(Bytes, Address),

    #[cfg_attr(feature = "serde", serde(rename = "eth_signTransaction", with = "sequence"))]
    EthSignTransaction(Box<WithOtherFields<TransactionRequest>>),

    /// Signs data via [EIP-712](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-712.md).
    #[cfg_attr(feature = "serde", serde(rename = "eth_signTypedData"))]
    EthSignTypedData(Address, serde_json::Value),

    /// Signs data via [EIP-712](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-712.md).
    #[cfg_attr(feature = "serde", serde(rename = "eth_signTypedData_v3"))]
    EthSignTypedDataV3(Address, serde_json::Value),

    /// Signs data via [EIP-712](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-712.md), and includes full support of arrays and recursive data structures.
    #[cfg_attr(feature = "serde", serde(rename = "eth_signTypedData_v4"))]
    EthSignTypedDataV4(Address, alloy_dyn_abi::TypedData),

    #[cfg_attr(feature = "serde", serde(rename = "eth_sendTransaction", with = "sequence"))]
    EthSendTransaction(Box<WithOtherFields<TransactionRequest>>),

    #[cfg_attr(feature = "serde", serde(rename = "eth_sendRawTransaction", with = "sequence"))]
    EthSendRawTransaction(Bytes),

    #[cfg_attr(feature = "serde", serde(rename = "eth_call"))]
    EthCall(
        WithOtherFields<TransactionRequest>,
        #[cfg_attr(feature = "serde", serde(default))] Option<BlockId>,
        #[cfg_attr(feature = "serde", serde(default))] Option<StateOverride>,
    ),

    #[cfg_attr(feature = "serde", serde(rename = "eth_createAccessList"))]
    EthCreateAccessList(
        WithOtherFields<TransactionRequest>,
        #[cfg_attr(feature = "serde", serde(default))] Option<BlockId>,
    ),

    #[cfg_attr(feature = "serde", serde(rename = "eth_estimateGas"))]
    EthEstimateGas(
        WithOtherFields<TransactionRequest>,
        #[cfg_attr(feature = "serde", serde(default))] Option<BlockId>,
        #[cfg_attr(feature = "serde", serde(default))] Option<StateOverride>,
    ),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getTransactionByHash", with = "sequence"))]
    EthGetTransactionByHash(TxHash),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getTransactionByBlockHashAndIndex"))]
    EthGetTransactionByBlockHashAndIndex(TxHash, Index),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getTransactionByBlockNumberAndIndex"))]
    EthGetTransactionByBlockNumberAndIndex(BlockNumber, Index),

    #[cfg_attr(
        feature = "serde",
        serde(rename = "eth_getRawTransactionByHash", with = "sequence")
    )]
    EthGetRawTransactionByHash(TxHash),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getRawTransactionByBlockHashAndIndex"))]
    EthGetRawTransactionByBlockHashAndIndex(TxHash, Index),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getRawTransactionByBlockNumberAndIndex"))]
    EthGetRawTransactionByBlockNumberAndIndex(BlockNumber, Index),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getTransactionReceipt", with = "sequence"))]
    EthGetTransactionReceipt(B256),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getBlockReceipts", with = "sequence"))]
    EthGetBlockReceipts(BlockId),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getUncleByBlockHashAndIndex"))]
    EthGetUncleByBlockHashAndIndex(B256, Index),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getUncleByBlockNumberAndIndex"))]
    EthGetUncleByBlockNumberAndIndex(
        #[cfg_attr(
            feature = "serde",
            serde(deserialize_with = "lenient_block_number::lenient_block_number")
        )]
        BlockNumber,
        Index,
    ),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getLogs", with = "sequence"))]
    EthGetLogs(Filter),

    /// Creates a filter object, based on filter options, to notify when the state changes (logs).
    #[cfg_attr(feature = "serde", serde(rename = "eth_newFilter", with = "sequence"))]
    EthNewFilter(Filter),

    /// Polling method for a filter, which returns an array of logs which occurred since last poll.
    #[cfg_attr(feature = "serde", serde(rename = "eth_getFilterChanges", with = "sequence"))]
    EthGetFilterChanges(String),

    /// Creates a filter in the node, to notify when a new block arrives.
    /// To check if the state has changed, call `eth_getFilterChanges`.
    #[cfg_attr(feature = "serde", serde(rename = "eth_newBlockFilter", with = "empty_params"))]
    EthNewBlockFilter(()),

    /// Creates a filter in the node, to notify when new pending transactions arrive.
    /// To check if the state has changed, call `eth_getFilterChanges`.
    #[cfg_attr(
        feature = "serde",
        serde(rename = "eth_newPendingTransactionFilter", with = "empty_params")
    )]
    EthNewPendingTransactionFilter(()),

    /// Returns an array of all logs matching filter with given id.
    #[cfg_attr(feature = "serde", serde(rename = "eth_getFilterLogs", with = "sequence"))]
    EthGetFilterLogs(String),

    /// Removes the filter, returns true if the filter was installed
    #[cfg_attr(feature = "serde", serde(rename = "eth_uninstallFilter", with = "sequence"))]
    EthUninstallFilter(String),

    #[cfg_attr(feature = "serde", serde(rename = "eth_getWork", with = "empty_params"))]
    EthGetWork(()),

    #[cfg_attr(feature = "serde", serde(rename = "eth_submitWork"))]
    EthSubmitWork(B64, B256, B256),

    #[cfg_attr(feature = "serde", serde(rename = "eth_submitHashrate"))]
    EthSubmitHashRate(U256, B256),

    #[cfg_attr(feature = "serde", serde(rename = "eth_feeHistory"))]
    EthFeeHistory(
        #[cfg_attr(feature = "serde", serde(deserialize_with = "deserialize_number"))] U256,
        BlockNumber,
        #[cfg_attr(feature = "serde", serde(default))] Vec<f64>,
    ),

    #[cfg_attr(feature = "serde", serde(rename = "eth_syncing", with = "empty_params"))]
    EthSyncing(()),

    /// geth's `debug_getRawTransaction`  endpoint
    #[cfg_attr(feature = "serde", serde(rename = "debug_getRawTransaction", with = "sequence"))]
    DebugGetRawTransaction(TxHash),

    /// geth's `debug_traceTransaction`  endpoint
    #[cfg_attr(feature = "serde", serde(rename = "debug_traceTransaction"))]
    DebugTraceTransaction(
        B256,
        #[cfg_attr(feature = "serde", serde(default))] GethDebugTracingOptions,
    ),

    /// geth's `debug_traceCall`  endpoint
    #[cfg_attr(feature = "serde", serde(rename = "debug_traceCall"))]
    DebugTraceCall(
        WithOtherFields<TransactionRequest>,
        #[cfg_attr(feature = "serde", serde(default))] Option<BlockId>,
        #[cfg_attr(feature = "serde", serde(default))] GethDebugTracingCallOptions,
    ),

    /// Trace transaction endpoint for parity's `trace_transaction`
    #[cfg_attr(feature = "serde", serde(rename = "trace_transaction", with = "sequence"))]
    TraceTransaction(B256),

    /// Trace transaction endpoint for parity's `trace_block`
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "trace_block",
            deserialize_with = "lenient_block_number::lenient_block_number_seq"
        )
    )]
    TraceBlock(BlockNumber),

    // Return filtered traces over blocks
    #[cfg_attr(feature = "serde", serde(rename = "trace_filter",))]
    TraceFilter(TraceFilter),

    // Custom endpoints, they're not extracted to a separate type out of serde convenience
    /// send transactions impersonating specific account and contract addresses.
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_impersonateAccount",
            alias = "hardhat_impersonateAccount",
            with = "sequence"
        )
    )]
    ImpersonateAccount(Address),
    /// Stops impersonating an account if previously set with `anvil_impersonateAccount`
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_stopImpersonatingAccount",
            alias = "hardhat_stopImpersonatingAccount",
            with = "sequence"
        )
    )]
    StopImpersonatingAccount(Address),
    /// Will make every account impersonated
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_autoImpersonateAccount",
            alias = "hardhat_autoImpersonateAccount",
            with = "sequence"
        )
    )]
    AutoImpersonateAccount(bool),
    /// Returns true if automatic mining is enabled, and false.
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_getAutomine", alias = "hardhat_getAutomine", with = "empty_params")
    )]
    GetAutoMine(()),
    /// Mines a series of blocks
    #[cfg_attr(feature = "serde", serde(rename = "anvil_mine", alias = "hardhat_mine"))]
    Mine(
        /// Number of blocks to mine, if not set `1` block is mined
        #[cfg_attr(feature = "serde", serde(default, deserialize_with = "deserialize_number_opt"))]
        Option<U256>,
        /// The time interval between each block in seconds, defaults to `1` seconds
        /// The interval is applied only to blocks mined in the given method invocation, not to
        /// blocks mined afterwards. Set this to `0` to instantly mine _all_ blocks
        #[cfg_attr(feature = "serde", serde(default, deserialize_with = "deserialize_number_opt"))]
        Option<U256>,
    ),

    /// Enables or disables, based on the single boolean argument, the automatic mining of new
    /// blocks with each new transaction submitted to the network.
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_setAutomine", alias = "evm_setAutomine", with = "sequence")
    )]
    SetAutomine(bool),

    /// Sets the mining behavior to interval with the given interval (seconds)
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_setIntervalMining",
            alias = "evm_setIntervalMining",
            with = "sequence"
        )
    )]
    SetIntervalMining(u64),

    /// Removes transactions from the pool
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_dropTransaction",
            alias = "hardhat_dropTransaction",
            with = "sequence"
        )
    )]
    DropTransaction(B256),

    /// Removes transactions from the pool
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_dropAllTransactions",
            alias = "hardhat_dropAllTransactions",
            with = "empty_params"
        )
    )]
    DropAllTransactions(),

    /// Reset the fork to a fresh forked state, and optionally update the fork config
    #[cfg_attr(feature = "serde", serde(rename = "anvil_reset", alias = "hardhat_reset"))]
    Reset(#[cfg_attr(feature = "serde", serde(default))] Option<Params<Option<Forking>>>),

    /// Sets the backend rpc url
    #[cfg_attr(feature = "serde", serde(rename = "anvil_setRpcUrl", with = "sequence"))]
    SetRpcUrl(String),

    /// Modifies the balance of an account.
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_setBalance", alias = "hardhat_setBalance")
    )]
    SetBalance(
        Address,
        #[cfg_attr(feature = "serde", serde(deserialize_with = "deserialize_number"))] U256,
    ),

    /// Sets the code of a contract
    #[cfg_attr(feature = "serde", serde(rename = "anvil_setCode", alias = "hardhat_setCode"))]
    SetCode(Address, Bytes),

    /// Sets the nonce of an address
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_setNonce",
            alias = "hardhat_setNonce",
            alias = "evm_setAccountNonce"
        )
    )]
    SetNonce(
        Address,
        #[cfg_attr(feature = "serde", serde(deserialize_with = "deserialize_number"))] U256,
    ),

    /// Writes a single slot of the account's storage
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_setStorageAt", alias = "hardhat_setStorageAt")
    )]
    SetStorageAt(
        Address,
        /// slot
        U256,
        /// value
        B256,
    ),

    /// Sets the coinbase address
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_setCoinbase", alias = "hardhat_setCoinbase", with = "sequence")
    )]
    SetCoinbase(Address),

    /// Sets the chain id
    #[cfg_attr(feature = "serde", serde(rename = "anvil_setChainId", with = "sequence"))]
    SetChainId(u64),

    /// Enable or disable logging
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_setLoggingEnabled",
            alias = "hardhat_setLoggingEnabled",
            with = "sequence"
        )
    )]
    SetLogging(bool),

    /// Set the minimum gas price for the node
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_setMinGasPrice",
            alias = "hardhat_setMinGasPrice",
            deserialize_with = "deserialize_number_seq"
        )
    )]
    SetMinGasPrice(U256),

    /// Sets the base fee of the next block
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_setNextBlockBaseFeePerGas",
            alias = "hardhat_setNextBlockBaseFeePerGas",
            deserialize_with = "deserialize_number_seq"
        )
    )]
    SetNextBlockBaseFeePerGas(U256),

    /// Sets the specific timestamp
    /// Accepts timestamp (Unix epoch) with millisecond precision and returns the number of seconds
    /// between the given timestamp and the current time.
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_setTime",
            alias = "evm_setTime",
            deserialize_with = "deserialize_number_seq"
        )
    )]
    EvmSetTime(U256),

    /// Serializes the current state (including contracts code, contract's storage, accounts
    /// properties, etc.) into a savable data blob
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_dumpState", alias = "hardhat_dumpState", with = "empty_params")
    )]
    DumpState(()),

    /// Adds state previously dumped with `DumpState` to the current chain
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_loadState", alias = "hardhat_loadState", with = "sequence")
    )]
    LoadState(Bytes),

    /// Retrieves the Anvil node configuration params
    #[cfg_attr(feature = "serde", serde(rename = "anvil_nodeInfo", with = "empty_params"))]
    NodeInfo(()),

    /// Retrieves the Anvil node metadata.
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_metadata", alias = "hardhat_metadata", with = "empty_params")
    )]
    AnvilMetadata(()),

    // Ganache compatible calls
    /// Snapshot the state of the blockchain at the current block.
    ///
    /// Ref <https://github.com/trufflesuite/ganache/blob/ef1858d5d6f27e4baeb75cccd57fb3dc77a45ae8/src/chains/ethereum/ethereum/RPC-METHODS.md#evm_snapshot>
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_snapshot", alias = "evm_snapshot", with = "empty_params")
    )]
    EvmSnapshot(()),

    /// Revert the state of the blockchain to a previous snapshot.
    /// Takes a single parameter, which is the snapshot id to revert to.
    ///
    /// Ref <https://github.com/trufflesuite/ganache/blob/ef1858d5d6f27e4baeb75cccd57fb3dc77a45ae8/src/chains/ethereum/ethereum/RPC-METHODS.md#evm_revert>
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_revert",
            alias = "evm_revert",
            deserialize_with = "deserialize_number_seq"
        )
    )]
    EvmRevert(U256),

    /// Jump forward in time by the given amount of time, in seconds.
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_increaseTime",
            alias = "evm_increaseTime",
            deserialize_with = "deserialize_number_seq"
        )
    )]
    EvmIncreaseTime(U256),

    /// Similar to `evm_increaseTime` but takes the exact timestamp that you want in the next block
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_setNextBlockTimestamp",
            alias = "evm_setNextBlockTimestamp",
            deserialize_with = "deserialize_number_seq"
        )
    )]
    EvmSetNextBlockTimeStamp(U256),

    /// Set the exact gas limit that you want in the next block
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "anvil_setBlockGasLimit",
            alias = "evm_setBlockGasLimit",
            deserialize_with = "deserialize_number_seq"
        )
    )]
    EvmSetBlockGasLimit(U256),

    /// Similar to `evm_increaseTime` but takes sets a block timestamp `interval`.
    ///
    /// The timestamp of the next block will be computed as `lastBlock_timestamp + interval`.
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_setBlockTimestampInterval", with = "sequence")
    )]
    EvmSetBlockTimeStampInterval(u64),

    /// Removes a `anvil_setBlockTimestampInterval` if it exists
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_removeBlockTimestampInterval", with = "empty_params")
    )]
    EvmRemoveBlockTimeStampInterval(()),

    /// Mine a single block
    #[cfg_attr(feature = "serde", serde(rename = "evm_mine"))]
    EvmMine(#[cfg_attr(feature = "serde", serde(default))] Option<Params<Option<MineOptions>>>),

    /// Mine a single block and return detailed data
    ///
    /// This behaves exactly as `EvmMine` but returns different output, for compatibility reasons
    /// this is a separate call since `evm_mine` is not an anvil original.
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_mine_detailed", alias = "evm_mine_detailed",)
    )]
    EvmMineDetailed(
        #[cfg_attr(feature = "serde", serde(default))] Option<Params<Option<MineOptions>>>,
    ),

    /// Execute a transaction regardless of signature status
    #[cfg_attr(
        feature = "serde",
        serde(rename = "eth_sendUnsignedTransaction", with = "sequence")
    )]
    EthSendUnsignedTransaction(Box<WithOtherFields<TransactionRequest>>),

    /// Turn on call traces for transactions that are returned to the user when they execute a
    /// transaction (instead of just txhash/receipt)
    #[cfg_attr(feature = "serde", serde(rename = "anvil_enableTraces", with = "empty_params"))]
    EnableTraces(()),

    /// Returns the number of transactions currently pending for inclusion in the next block(s), as
    /// well as the ones that are being scheduled for future execution only.
    /// Ref: <https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_status>
    #[cfg_attr(feature = "serde", serde(rename = "txpool_status", with = "empty_params"))]
    TxPoolStatus(()),

    /// Returns a summary of all the transactions currently pending for inclusion in the next
    /// block(s), as well as the ones that are being scheduled for future execution only.
    /// Ref: <https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_inspect>
    #[cfg_attr(feature = "serde", serde(rename = "txpool_inspect", with = "empty_params"))]
    TxPoolInspect(()),

    /// Returns the details of all transactions currently pending for inclusion in the next
    /// block(s), as well as the ones that are being scheduled for future execution only.
    /// Ref: <https://geth.ethereum.org/docs/rpc/ns-txpool#txpool_content>
    #[cfg_attr(feature = "serde", serde(rename = "txpool_content", with = "empty_params"))]
    TxPoolContent(()),

    /// Otterscan's `ots_getApiLevel` endpoint
    /// Otterscan currently requires this endpoint, even though it's not part of the ots_*
    /// <https://github.com/otterscan/otterscan/blob/071d8c55202badf01804f6f8d53ef9311d4a9e47/src/useProvider.ts#L71>
    /// Related upstream issue: <https://github.com/otterscan/otterscan/issues/1081>
    #[cfg_attr(feature = "serde", serde(rename = "erigon_getHeaderByNumber"))]
    ErigonGetHeaderByNumber(
        #[cfg_attr(
            feature = "serde",
            serde(deserialize_with = "lenient_block_number::lenient_block_number_seq")
        )]
        BlockNumber,
    ),

    /// Otterscan's `ots_getApiLevel` endpoint
    /// Used as a simple API versioning scheme for the ots_* namespace
    #[cfg_attr(feature = "serde", serde(rename = "ots_getApiLevel", with = "empty_params"))]
    OtsGetApiLevel(()),

    /// Otterscan's `ots_getInternalOperations` endpoint
    /// Traces internal ETH transfers, contracts creation (CREATE/CREATE2) and self-destructs for a
    /// certain transaction.
    #[cfg_attr(feature = "serde", serde(rename = "ots_getInternalOperations", with = "sequence"))]
    OtsGetInternalOperations(B256),

    /// Otterscan's `ots_hasCode` endpoint
    /// Check if an ETH address contains code at a certain block number.
    #[cfg_attr(feature = "serde", serde(rename = "ots_hasCode"))]
    OtsHasCode(
        Address,
        #[cfg_attr(
            feature = "serde",
            serde(deserialize_with = "lenient_block_number::lenient_block_number", default)
        )]
        BlockNumber,
    ),

    /// Otterscan's `ots_traceTransaction` endpoint
    /// Trace a transaction and generate a trace call tree.
    #[cfg_attr(feature = "serde", serde(rename = "ots_traceTransaction", with = "sequence"))]
    OtsTraceTransaction(B256),

    /// Otterscan's `ots_getTransactionError` endpoint
    /// Given a transaction hash, returns its raw revert reason.
    #[cfg_attr(feature = "serde", serde(rename = "ots_getTransactionError", with = "sequence"))]
    OtsGetTransactionError(B256),

    /// Otterscan's `ots_getBlockDetails` endpoint
    /// Given a block number, return its data. Similar to the standard eth_getBlockByNumber/Hash
    /// method, but can be optimized by excluding unnecessary data such as transactions and
    /// logBloom
    #[cfg_attr(feature = "serde", serde(rename = "ots_getBlockDetails"))]
    OtsGetBlockDetails(
        #[cfg_attr(
            feature = "serde",
            serde(deserialize_with = "lenient_block_number::lenient_block_number_seq", default)
        )]
        BlockNumber,
    ),

    /// Otterscan's `ots_getBlockDetails` endpoint
    /// Same as `ots_getBlockDetails`, but receiving a block hash instead of number
    #[cfg_attr(feature = "serde", serde(rename = "ots_getBlockDetailsByHash", with = "sequence"))]
    OtsGetBlockDetailsByHash(B256),

    /// Otterscan's `ots_getBlockTransactions` endpoint
    /// Gets paginated transaction data for a certain block. Return data is similar to
    /// eth_getBlockBy* + eth_getTransactionReceipt.
    #[cfg_attr(feature = "serde", serde(rename = "ots_getBlockTransactions"))]
    OtsGetBlockTransactions(u64, usize, usize),

    /// Otterscan's `ots_searchTransactionsBefore` endpoint
    /// Address history navigation. searches backwards from certain point in time.
    #[cfg_attr(feature = "serde", serde(rename = "ots_searchTransactionsBefore"))]
    OtsSearchTransactionsBefore(Address, u64, usize),

    /// Otterscan's `ots_searchTransactionsAfter` endpoint
    /// Address history navigation. searches forward from certain point in time.
    #[cfg_attr(feature = "serde", serde(rename = "ots_searchTransactionsAfter"))]
    OtsSearchTransactionsAfter(Address, u64, usize),

    /// Otterscan's `ots_getTransactionBySenderAndNonce` endpoint
    /// Given a sender address and a nonce, returns the tx hash or null if not found. It returns
    /// only the tx hash on success, you can use the standard eth_getTransactionByHash after that
    /// to get the full transaction data.
    #[cfg_attr(feature = "serde", serde(rename = "ots_getTransactionBySenderAndNonce",))]
    OtsGetTransactionBySenderAndNonce(
        Address,
        #[cfg_attr(feature = "serde", serde(deserialize_with = "deserialize_number"))] U256,
    ),

    /// Otterscan's `ots_getTransactionBySenderAndNonce` endpoint
    /// Given an ETH contract address, returns the tx hash and the direct address who created the
    /// contract.
    #[cfg_attr(feature = "serde", serde(rename = "ots_getContractCreator", with = "sequence"))]
    OtsGetContractCreator(Address),

    /// Removes transactions from the pool by sender origin.
    #[cfg_attr(
        feature = "serde",
        serde(rename = "anvil_removePoolTransactions", with = "sequence")
    )]
    RemovePoolTransactions(Address),
}

/// Represents ethereum JSON-RPC API
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "method", content = "params"))]
pub enum EthPubSub {
    /// Subscribe to an eth subscription
    #[cfg_attr(feature = "serde", serde(rename = "eth_subscribe"))]
    EthSubscribe(
        SubscriptionKind,
        #[cfg_attr(feature = "serde", serde(default))] Box<SubscriptionParams>,
    ),

    /// Unsubscribe from an eth subscription
    #[cfg_attr(feature = "serde", serde(rename = "eth_unsubscribe", with = "sequence"))]
    EthUnSubscribe(SubscriptionId),
}

/// Container type for either a request or a pub sub
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum EthRpcCall {
    Request(Box<EthRequest>),
    PubSub(EthPubSub),
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
    fn test_eth_get_proof() {
        let s = r#"{"method":"eth_getProof","params":["0x7F0d15C7FAae65896648C8273B6d7E43f58Fa842",["0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"],"latest"]}"#;
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
    fn test_net_listening() {
        let s = r#"{"method": "net_listening", "params":[]}"#;
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
    fn test_eth_max_priority_fee() {
        let s = r#"{"method": "eth_maxPriorityFeePerGas", "params":[]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_eth_syncing() {
        let s = r#"{"method": "eth_syncing", "params":[]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_impersonate_account() {
        let s = r#"{"method": "anvil_impersonateAccount", "params":
["0xd84de507f3fada7df80908082d3239466db55a71"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_stop_impersonate_account() {
        let s = r#"{"method": "anvil_stopImpersonatingAccount",  "params":
["0x364d6D0333432C3Ac016Ca832fb8594A8cE43Ca6"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_auto_impersonate_account() {
        let s = r#"{"method": "anvil_autoImpersonateAccount",  "params": [true]}"#;
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
        let s = r#"{"method": "anvil_mine", "params":
["0xd84de507f3fada7df80908082d3239466db55a71"]}"#;
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
        let s = r#"{"method": "anvil_setAutomine", "params": [false]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method": "evm_setAutomine", "params": [false]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_interval_mining() {
        let s = r#"{"method": "anvil_setIntervalMining", "params": [100]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method": "evm_setIntervalMining", "params": [100]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_drop_tx() {
        let s = r#"{"method": "anvil_dropTransaction", "params":
["0x4a3b0fce2cb9707b0baa68640cf2fe858c8bb4121b2a8cb904ff369d38a560ff"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_reset() {
        let s = r#"{"method": "anvil_reset", "params": [{"forking": {"jsonRpcUrl": "https://ethereumpublicnode.com",
        "blockNumber": "18441649"
      }
    }]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Reset(forking) => {
                let forking = forking.and_then(|f| f.params);
                assert_eq!(
                    forking,
                    Some(Forking {
                        json_rpc_url: Some("https://ethereumpublicnode.com".into()),
                        block_number: Some(18441649)
                    })
                )
            }
            _ => unreachable!(),
        }

        let s = r#"{"method": "anvil_reset", "params": [ { "forking": {
                "jsonRpcUrl": "https://eth-mainnet.alchemyapi.io/v2/<key>",
                "blockNumber": 11095000
        }}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Reset(forking) => {
                let forking = forking.and_then(|f| f.params);
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
                let forking = forking.and_then(|f| f.params);
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
                let forking = forking.and_then(|f| f.params);
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
                let forking = forking.and_then(|f| f.params);
                assert_eq!(
                    forking,
                    Some(Forking { json_rpc_url: None, block_number: Some(14000000) })
                )
            }
            _ => unreachable!(),
        }

        let s = r#"{"method":"anvil_reset","params":[{ "blockNumber": "14000000"}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Reset(forking) => {
                let forking = forking.and_then(|f| f.params);
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
                let forking = forking.and_then(|f| f.params);
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

        let s = r#"{"method": "anvil_reset"}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::Reset(forking) => {
                assert!(forking.is_none())
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_custom_set_balance() {
        let s = r#"{"method": "anvil_setBalance", "params":
["0xd84de507f3fada7df80908082d3239466db55a71", "0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "anvil_setBalance", "params":
["0xd84de507f3fada7df80908082d3239466db55a71", 1337]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_set_code() {
        let s = r#"{"method": "anvil_setCode", "params":
["0xd84de507f3fada7df80908082d3239466db55a71", "0x0123456789abcdef"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "anvil_setCode", "params":
["0xd84de507f3fada7df80908082d3239466db55a71", "0x"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "anvil_setCode", "params":
["0xd84de507f3fada7df80908082d3239466db55a71", ""]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_custom_set_nonce() {
        let s = r#"{"method": "anvil_setNonce", "params":
["0xd84de507f3fada7df80908082d3239466db55a71", "0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method":
"hardhat_setNonce", "params": ["0xd84de507f3fada7df80908082d3239466db55a71", "0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method": "evm_setAccountNonce", "params":
["0xd84de507f3fada7df80908082d3239466db55a71", "0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_set_storage_at() {
        let s = r#"{"method": "anvil_setStorageAt", "params":
["0x295a70b2de5e3953354a6a8344e616ed314d7251", "0x0",
"0x0000000000000000000000000000000000000000000000000000000000003039"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "hardhat_setStorageAt", "params":
["0xe9e7CEA3DedcA5984780Bafc599bD69ADd087D56",
"0xa6eef7e35abe7026729641147f7915573c7e97b47efa546f5f6e3230263bcb49",
"0x0000000000000000000000000000000000000000000000000000000000003039"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_coinbase() {
        let s = r#"{"method": "anvil_setCoinbase", "params":
["0x295a70b2de5e3953354a6a8344e616ed314d7251"]}"#;
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
    fn test_serde_set_time() {
        let s = r#"{"method": "anvil_setTime", "params": ["0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "anvil_increaseTime", "params": 1}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_dump_state() {
        let s = r#"{"method": "anvil_dumpState", "params": [] }"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_load_state() {
        let s = r#"{"method": "anvil_loadState", "params": ["0x0001"] }"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_snapshot() {
        let s = r#"{"method": "anvil_snapshot", "params": [] }"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "evm_snapshot", "params": [] }"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_revert() {
        let s = r#"{"method": "anvil_revert", "params": ["0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_increase_time() {
        let s = r#"{"method": "anvil_increaseTime", "params": ["0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "anvil_increaseTime", "params": [1]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "anvil_increaseTime", "params": 1}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "evm_increaseTime", "params": ["0x0"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "evm_increaseTime", "params": [1]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "evm_increaseTime", "params": 1}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_next_timestamp() {
        let s = r#"{"method": "anvil_setNextBlockTimestamp", "params": [100]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method": "evm_setNextBlockTimestamp", "params": [100]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method": "evm_setNextBlockTimestamp", "params": ["0x64e0f308"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_timestamp_interval() {
        let s = r#"{"method": "anvil_setBlockTimestampInterval", "params": [100]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_remove_timestamp_interval() {
        let s = r#"{"method": "anvil_removeBlockTimestampInterval", "params": []}"#;
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
                    params.unwrap().params.unwrap_or_default(),
                    MineOptions::Options { timestamp: Some(100), blocks: Some(100) }
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

        let s = r#"{"method": "evm_mine", "params": []}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_evm_mine_detailed() {
        let s = r#"{"method": "anvil_mine_detailed", "params": [100]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method": "anvil_mine_detailed", "params": [{
            "timestamp": 100,
            "blocks": 100
        }]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::EvmMineDetailed(params) => {
                assert_eq!(
                    params.unwrap().params.unwrap_or_default(),
                    MineOptions::Options { timestamp: Some(100), blocks: Some(100) }
                )
            }
            _ => unreachable!(),
        }

        let s = r#"{"method": "evm_mine_detailed"}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();

        match req {
            EthRequest::EvmMineDetailed(params) => {
                assert!(params.is_none())
            }
            _ => unreachable!(),
        }

        let s = r#"{"method": "anvil_mine_detailed", "params": []}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_custom_evm_mine_hex() {
        let s = r#"{"method": "evm_mine", "params": ["0x63b6ff08"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::EvmMine(params) => {
                assert_eq!(
                    params.unwrap().params.unwrap_or_default(),
                    MineOptions::Timestamp(Some(1672937224))
                )
            }
            _ => unreachable!(),
        }

        let s = r#"{"method": "evm_mine", "params": [{"timestamp": "0x63b6ff08"}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let req = serde_json::from_value::<EthRequest>(value).unwrap();
        match req {
            EthRequest::EvmMine(params) => {
                assert_eq!(
                    params.unwrap().params.unwrap_or_default(),
                    MineOptions::Options { timestamp: Some(1672937224), blocks: None }
                )
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_eth_uncle_count_by_block_hash() {
        let s = r#"{"jsonrpc":"2.0","method":"eth_getUncleCountByBlockHash","params":["0x4a3b0fce2cb9707b0baa68640cf2fe858c8bb4121b2a8cb904ff369d38a560ff"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_eth_block_tx_count_by_block_hash() {
        let s = r#"{"jsonrpc":"2.0","method":"eth_getBlockTransactionCountByHash","params":["0x4a3b0fce2cb9707b0baa68640cf2fe858c8bb4121b2a8cb904ff369d38a560ff"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
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
        let s = r#"{"id": 1, "method": "eth_unsubscribe", "params":
["0x9cef478923ff08bf67fde6c64013158d"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthPubSub>(value).unwrap();
    }

    #[test]
    fn test_serde_eth_subscribe() {
        let s = r#"{"id": 1, "method": "eth_subscribe", "params": ["newHeads"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthPubSub>(value).unwrap();

        let s = r#"{"id": 1, "method": "eth_subscribe", "params": ["logs", {"address":
"0x8320fe7702b96808f7bbc0d4a888ed1468216cfd", "topics":
["0xd78a0cb8bb633d06981248b816e7bd33c2a35a6089241d099fa519e361cab902"]}]}"#;
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
    fn test_serde_debug_raw_transaction() {
        let s = r#"{"jsonrpc":"2.0","method":"debug_getRawTransaction","params":["0x3ed3a89bc10115a321aee238c02de214009f8532a65368e5df5eaf732ee7167c"],"id":1}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"jsonrpc":"2.0","method":"eth_getRawTransactionByHash","params":["0x3ed3a89bc10115a321aee238c02de214009f8532a65368e5df5eaf732ee7167c"],"id":1}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"jsonrpc":"2.0","method":"eth_getRawTransactionByBlockHashAndIndex","params":["0x3ed3a89bc10115a321aee238c02de214009f8532a65368e5df5eaf732ee7167c",1],"id":1}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"jsonrpc":"2.0","method":"eth_getRawTransactionByBlockNumberAndIndex","params":["0x3ed3a89b",0],"id":1}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_debug_trace_transaction() {
        let s = r#"{"method": "debug_traceTransaction", "params":
["0x4a3b0fce2cb9707b0baa68640cf2fe858c8bb4121b2a8cb904ff369d38a560ff"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "debug_traceTransaction", "params":
["0x4a3b0fce2cb9707b0baa68640cf2fe858c8bb4121b2a8cb904ff369d38a560ff", {}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "debug_traceTransaction", "params":
["0x4a3b0fce2cb9707b0baa68640cf2fe858c8bb4121b2a8cb904ff369d38a560ff", {"disableStorage":
true}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_debug_trace_call() {
        let s = r#"{"method": "debug_traceCall", "params": [{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "debug_traceCall", "params": [{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}, { "blockNumber": "latest" }]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "debug_traceCall", "params": [{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}, { "blockNumber": "0x0" }]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "debug_traceCall", "params": [{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}, { "blockHash": "0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3" }]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        let s = r#"{"method": "debug_traceCall", "params": [{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}, { "blockNumber": "0x0" }, {"disableStorage": true}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_eth_storage() {
        let s = r#"{"method": "eth_getStorageAt", "params":
["0x295a70b2de5e3953354a6a8344e616ed314d7251", "0x0", "latest"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_eth_call() {
        let req = r#"{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}"#;
        let _req = serde_json::from_str::<TransactionRequest>(req).unwrap();

        let s = r#"{"method": "eth_call", "params":[{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"},"latest"]}"#;
        let _req = serde_json::from_str::<EthRequest>(s).unwrap();

        let s = r#"{"method": "eth_call", "params":[{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}]}"#;
        let _req = serde_json::from_str::<EthRequest>(s).unwrap();

        let s = r#"{"method": "eth_call", "params":[{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}, { "blockNumber": "latest" }]}"#;
        let _req = serde_json::from_str::<EthRequest>(s).unwrap();

        let s = r#"{"method": "eth_call", "params":[{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}, { "blockNumber": "0x0" }]}"#;
        let _req = serde_json::from_str::<EthRequest>(s).unwrap();

        let s = r#"{"method": "eth_call", "params":[{"data":"0xcfae3217","from":"0xd84de507f3fada7df80908082d3239466db55a71","to":"0xcbe828fdc46e3b1c351ec90b1a5e7d9742c0398d"}, { "blockHash":"0xd4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3" }]}"#;
        let _req = serde_json::from_str::<EthRequest>(s).unwrap();
    }

    #[test]
    fn test_serde_eth_balance() {
        let s = r#"{"method": "eth_getBalance", "params":
["0x295a70b2de5e3953354a6a8344e616ed314d7251", "latest"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();

        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_serde_eth_block_by_number() {
        let s = r#"{"method": "eth_getBlockByNumber", "params": ["0x0", true]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method": "eth_getBlockByNumber", "params": ["latest", true]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method": "eth_getBlockByNumber", "params": ["earliest", true]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method": "eth_getBlockByNumber", "params": ["pending", true]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();

        // this case deviates from the spec, but we're supporting this for legacy reasons: <https://github.com/foundry-rs/foundry/issues/1868>
        let s = r#"{"method": "eth_getBlockByNumber", "params": [0, true]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_eth_sign() {
        let s = r#"{"method": "eth_sign", "params":
["0xd84de507f3fada7df80908082d3239466db55a71", "0x00"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
        let s = r#"{"method": "personal_sign", "params":
["0x00", "0xd84de507f3fada7df80908082d3239466db55a71"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_eth_sign_typed_data() {
        let s = r#"{"method":"eth_signTypedData_v4","params":["0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826", {"types":{"EIP712Domain":[{"name":"name","type":"string"},{"name":"version","type":"string"},{"name":"chainId","type":"uint256"},{"name":"verifyingContract","type":"address"}],"Person":[{"name":"name","type":"string"},{"name":"wallet","type":"address"}],"Mail":[{"name":"from","type":"Person"},{"name":"to","type":"Person"},{"name":"contents","type":"string"}]},"primaryType":"Mail","domain":{"name":"Ether Mail","version":"1","chainId":1,"verifyingContract":"0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"},"message":{"from":{"name":"Cow","wallet":"0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"},"to":{"name":"Bob","wallet":"0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"},"contents":"Hello, Bob!"}}]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }

    #[test]
    fn test_remove_pool_transactions() {
        let s = r#"{"method": "anvil_removePoolTransactions",  "params":["0x364d6D0333432C3Ac016Ca832fb8594A8cE43Ca6"]}"#;
        let value: serde_json::Value = serde_json::from_str(s).unwrap();
        let _req = serde_json::from_value::<EthRequest>(value).unwrap();
    }
}
