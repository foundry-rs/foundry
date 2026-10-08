use crate::{
    block_env::BlockEnvOverrides,
    eth_api::on_large_stack,
    evm::AnvilNextBlockEnv,
    fork::{ForkInfo, NodeInfoProbe},
    impersonation::ImpersonationState,
    logging::LoggingState,
    miner::HookFuture,
    mining::{MiningController, wait_for_pool},
    node::Relauncher,
    simulate,
    snapshot::{Snapshot, SnapshotManager},
    state::{AnvilState, SharedAnvilState},
    state_dump::{CheckpointForks, SerializableHistoricalStates, SerializableState, StateDump},
    time::TimeManager,
    types::{ForkChoice, ForkUrl, ReorgOptions, ReorgParams, TransactionData, TransactionOrder},
};
use alloy_consensus::{
    Blob, BlockHeader, Transaction, TxEip4844Variant, TxEnvelope,
    transaction::{TxEip4844WithSidecar, TxHashRef},
};
use alloy_dyn_abi::TypedData;
use alloy_eips::{
    BlockId, BlockNumberOrTag, Decodable2718, Encodable2718, eip2718::EIP4844_TX_TYPE_ID,
    eip7594::BlobTransactionSidecarVariant,
};
use alloy_json_rpc::RpcObject;
use alloy_network::{TransactionBuilder, primitives::HeaderResponse};
use alloy_primitives::{Address, B256, Bytes, TxKind, U64, U256};
use alloy_provider::Provider;
use alloy_rpc_types::anvil::{
    ForkedNetwork, Forking, Metadata, MineOptions, NodeEnvironment, NodeForkConfig, NodeInfo,
};
use alloy_rpc_types_eth::{
    AccessListResult, BlockOverrides, Bundle, EthCallResponse, FeeHistory, Filter, FilterId,
    StateContext, TransactionRequest,
    erc4337::TransactionConditional,
    simulate::{SimulatePayload, SimulatedBlock},
    state::{AccountOverride, StateOverride, StateOverridesBuilder},
};
use alloy_signer_local::PrivateKeySigner;
use foundry_common::{
    provider::ProviderBuilder,
    version::{COMMIT_SHA, SEMVER_VERSION},
};
use foundry_evm_core::{decode::RevertDecoder, utils::block_env_from_header};
use jsonrpsee::{
    core::{RpcResult, async_trait},
    proc_macros::rpc,
    types::{
        ErrorObjectOwned,
        error::{INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE},
    },
};
use parking_lot::{Mutex, RwLock};
use reth_ethereum::{
    PooledTransactionVariant,
    chainspec::{EthChainSpec, EthereumHardforks, Hardforks, MIN_TRANSACTION_GAS},
    evm::primitives::ConfigureEvm,
    pool::{TransactionPool, TransactionPoolExt},
    primitives::{Bytecode, SealedHeader, TxTy},
    rpc::eth::{
        EthApiError, FillTransaction, RpcInvalidTransactionError, error::RpcPoolError,
        utils::recover_raw_transaction,
    },
    storage::{BlockNumReader, HeaderProvider, StateProviderFactory, TransactionsProvider},
};
use reth_execution_types::ChangedAccount;
use reth_rpc_eth_api::{
    EthApiServer, FullEthApiServer, RpcBlock, RpcReceipt, RpcTransaction, RpcTxReq, RpcTypes,
};
use reth_rpc_server_types::constants::gas_oracle::ESTIMATE_GAS_ERROR_RATIO;
use revm::context::{BlockEnv, Cfg};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

#[cfg(feature = "tempo")]
use crate::network::tempo_storage::TempoStorage;
#[cfg(feature = "tempo")]
use alloy_consensus::transaction::SignerRecoverable;
#[cfg(feature = "tempo")]
use alloy_rlp::{Encodable as _, Header as RlpHeader, PayloadView};
#[cfg(feature = "tempo")]
use alloy_signer::SignerSync;
#[cfg(feature = "tempo")]
use foundry_evm_core::tempo::PATH_USD_ADDRESS;
#[cfg(feature = "tempo")]
use tempo_precompiles::{
    NONCE_PRECOMPILE_ADDRESS,
    nonce::NonceManager,
    storage::{Handler, StorageCtx},
    tip_fee_manager::{IFeeManager, TipFeeManager},
    tip20::{ITIP20, TIP20Token},
    tip20_factory::TIP20Factory,
};
#[cfg(feature = "tempo")]
use tempo_primitives::{
    TEMPO_TX_TYPE_ID, TempoTxEnvelope,
    transaction::{FEE_PAYER_SIGNATURE_MARKER, TEMPO_EXPIRING_NONCE_KEY},
};
#[cfg(feature = "tempo")]
use tempo_transaction_pool::validator::DEFAULT_AA_VALID_AFTER_MAX_SECS;

/// The `anvil_*` RPC namespace, with the `hardhat_*` and `evm_*` aliases that anvil accepts.
#[rpc(server, namespace = "anvil")]
pub trait AnvilApi<B: RpcObject, TxReq: RpcObject> {
    /// Impersonates the given account for `eth_sendTransaction`.
    #[method(name = "impersonateAccount", aliases = ["hardhat_impersonateAccount"])]
    async fn anvil_impersonate_account(&self, address: Address) -> RpcResult<()>;

    /// Stops impersonating the given account.
    #[method(name = "stopImpersonatingAccount", aliases = ["hardhat_stopImpersonatingAccount"])]
    async fn anvil_stop_impersonating_account(&self, address: Address) -> RpcResult<()>;

    /// Enables or disables impersonation of every account.
    #[method(name = "autoImpersonateAccount", aliases = ["hardhat_autoImpersonateAccount"])]
    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> RpcResult<()>;

    /// Returns whether automine is enabled.
    #[method(name = "getAutomine", aliases = ["hardhat_getAutomine"])]
    async fn anvil_get_automine(&self) -> RpcResult<bool>;

    /// Returns the interval mining period in seconds, if enabled.
    #[method(name = "getIntervalMining")]
    async fn anvil_get_interval_mining(&self) -> RpcResult<Option<u64>>;

    /// Enables or disables automine.
    #[method(name = "setAutomine", aliases = ["evm_setAutomine"])]
    async fn anvil_set_automine(&self, enabled: bool) -> RpcResult<()>;

    /// Sets the interval mining period in seconds. Zero disables interval mining.
    #[method(name = "setIntervalMining", aliases = ["evm_setIntervalMining"])]
    async fn anvil_set_interval_mining(&self, interval: u64) -> RpcResult<()>;

    /// Mines the given number of blocks, with an optional timestamp interval between them.
    #[method(name = "mine", aliases = ["hardhat_mine"])]
    async fn anvil_mine(&self, num_blocks: Option<U256>, interval: Option<U256>) -> RpcResult<()>;

    /// Mines blocks and returns them with full transactions.
    #[method(name = "mine_detailed", aliases = ["evm_mine_detailed"])]
    async fn anvil_mine_detailed(&self, opts: Option<MineOptions>) -> RpcResult<Vec<B>>;

    /// Snapshots the chain head and the anvil settings. Returns the snapshot id.
    #[method(name = "snapshot", aliases = ["evm_snapshot"])]
    async fn anvil_snapshot(&self) -> RpcResult<U256>;

    /// Reverts the chain to the given snapshot. Returns whether the snapshot existed.
    #[method(name = "revert", aliases = ["evm_revert"])]
    async fn anvil_revert(&self, id: U256) -> RpcResult<bool>;

    /// Rewinds the chain by the given number of blocks.
    #[method(name = "rollback")]
    async fn anvil_rollback(&self, depth: Option<u64>) -> RpcResult<()>;

    /// Rewinds the chain by `depth` blocks and mines `depth` blocks with the given transactions.
    #[method(name = "reorg")]
    async fn anvil_reorg(
        &self,
        params: ReorgParams<TxReq>,
        tx_block_pairs: Option<Vec<(TransactionData<TxReq>, u64)>>,
    ) -> RpcResult<()>;

    /// Resets the chain to genesis, or to the fork block when forking. Changing the fork endpoint
    /// or block is not supported yet.
    #[method(name = "reset", aliases = ["hardhat_reset"])]
    async fn anvil_reset(&self, forking: Option<Forking>) -> RpcResult<()>;

    /// Sets the chain id. The node relaunches with its state and height; earlier blocks are no
    /// longer served.
    #[method(name = "setChainId")]
    async fn anvil_set_chain_id(&self, chain_id: u64) -> RpcResult<()>;

    /// Replaces the fork endpoint.
    #[method(name = "setRpcUrl")]
    async fn anvil_set_rpc_url(&self, url: String) -> RpcResult<()>;

    /// Sets the minimum gas price. Rejected while EIP-1559 is active, as in anvil.
    #[method(name = "setMinGasPrice", aliases = ["hardhat_setMinGasPrice"])]
    async fn anvil_set_min_gas_price(&self, gas_price: U256) -> RpcResult<()>;

    /// Enables or disables transaction logging.
    #[method(name = "setLoggingEnabled", aliases = ["hardhat_setLoggingEnabled"])]
    async fn anvil_set_logging_enabled(&self, enabled: bool) -> RpcResult<()>;

    /// Returns the wall clock time at which the last block was built.
    #[method(name = "getLastBlockWallTime")]
    async fn anvil_get_last_block_wall_time(&self) -> RpcResult<u64>;

    /// Returns the pool blob with the given versioned hash.
    #[method(name = "getBlobByHash")]
    async fn anvil_get_blob_by_hash(&self, hash: B256) -> RpcResult<Option<String>>;

    /// Returns the pool blobs of the given transaction.
    #[method(name = "getBlobsByTransactionHash")]
    async fn anvil_get_blobs_by_transaction_hash(
        &self,
        hash: B256,
    ) -> RpcResult<Option<Vec<String>>>;

    /// Sets the ERC20 balance of an account by finding and overriding the balance slot.
    #[method(name = "dealERC20", aliases = ["hardhat_dealERC20", "anvil_setERC20Balance"])]
    async fn anvil_deal_erc20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()>;

    /// Sets an ERC20 allowance by finding and overriding the allowance slot.
    #[method(name = "setERC20Allowance")]
    async fn anvil_set_erc20_allowance(
        &self,
        owner: Address,
        spender: Address,
        token_address: Address,
        amount: U256,
    ) -> RpcResult<()>;

    /// Removes a transaction from the pool.
    #[method(name = "dropTransaction", aliases = ["hardhat_dropTransaction"])]
    async fn anvil_drop_transaction(&self, tx_hash: B256) -> RpcResult<Option<B256>>;

    /// Removes all transactions from the pool.
    #[method(name = "dropAllTransactions", aliases = ["hardhat_dropAllTransactions"])]
    async fn anvil_drop_all_transactions(&self) -> RpcResult<()>;

    /// Removes all transactions sent by the given address from the pool.
    #[method(name = "removePoolTransactions")]
    async fn anvil_remove_pool_transactions(&self, address: Address) -> RpcResult<()>;

    /// Returns the genesis block timestamp.
    #[method(name = "getGenesisTime")]
    async fn anvil_get_genesis_time(&self) -> RpcResult<u64>;

    /// Returns node configuration and the current head.
    #[method(name = "nodeInfo")]
    async fn anvil_node_info(&self) -> RpcResult<NodeInfo>;

    /// Returns client metadata.
    #[method(name = "metadata", aliases = ["hardhat_metadata"])]
    async fn anvil_metadata(&self) -> RpcResult<Metadata>;

    /// Sets the timestamp interval between mined blocks.
    #[method(name = "setBlockTimestampInterval")]
    async fn anvil_set_block_timestamp_interval(&self, seconds: u64) -> RpcResult<()>;

    /// Removes the block timestamp interval. Returns whether one was set.
    #[method(name = "removeBlockTimestampInterval")]
    async fn anvil_remove_block_timestamp_interval(&self) -> RpcResult<bool>;

    /// Moves the clock forward by the given number of seconds.
    #[method(name = "increaseTime", aliases = ["evm_increaseTime"])]
    async fn anvil_increase_time(&self, seconds: U256) -> RpcResult<i64>;

    /// Sets the clock to the given timestamp. Returns the applied offset in seconds.
    #[method(name = "setTime", aliases = ["evm_setTime"])]
    async fn anvil_set_time(&self, timestamp: u64) -> RpcResult<u64>;

    /// Sets the exact timestamp of the next block.
    #[method(name = "setNextBlockTimestamp", aliases = ["evm_setNextBlockTimestamp"])]
    async fn anvil_set_next_block_timestamp(&self, seconds: u64) -> RpcResult<()>;

    /// Sets the gas limit of all future blocks.
    #[method(name = "setBlockGasLimit", aliases = ["evm_setBlockGasLimit"])]
    async fn anvil_set_block_gas_limit(&self, gas_limit: U256) -> RpcResult<bool>;

    /// Sets the coinbase of all future blocks.
    #[method(name = "setCoinbase", aliases = ["hardhat_setCoinbase"])]
    async fn anvil_set_coinbase(&self, address: Address) -> RpcResult<()>;

    /// Sets the base fee of the next block.
    #[method(name = "setNextBlockBaseFeePerGas", aliases = ["hardhat_setNextBlockBaseFeePerGas"])]
    async fn anvil_set_next_block_base_fee_per_gas(&self, base_fee: U256) -> RpcResult<()>;

    /// Sets the prevrandao of the next block.
    #[method(name = "setNextBlockPrevRandao")]
    async fn anvil_set_next_block_prev_randao(&self, prev_randao: B256) -> RpcResult<()>;

    /// Sets the parent beacon block root of the next block.
    #[method(name = "setNextBlockParentBeaconBlockRoot")]
    async fn anvil_set_next_block_parent_beacon_block_root(&self, root: B256) -> RpcResult<()>;

    /// Sets the balance of an account.
    #[method(name = "setBalance", aliases = ["hardhat_setBalance", "tenderly_setBalance"])]
    async fn anvil_set_balance(&self, address: Address, balance: U256) -> RpcResult<()>;

    /// Adds to the balance of an account.
    #[method(name = "addBalance", aliases = ["hardhat_addBalance", "tenderly_addBalance"])]
    async fn anvil_add_balance(&self, address: Address, balance: U256) -> RpcResult<()>;

    /// Sets the nonce of an account.
    #[method(name = "setNonce", aliases = ["hardhat_setNonce", "evm_setAccountNonce"])]
    async fn anvil_set_nonce(&self, address: Address, nonce: U256) -> RpcResult<()>;

    /// Sets the code of an account.
    #[method(name = "setCode", aliases = ["hardhat_setCode"])]
    async fn anvil_set_code(&self, address: Address, code: Bytes) -> RpcResult<()>;

    /// Sets one storage slot of an account.
    #[method(name = "setStorageAt", aliases = ["hardhat_setStorageAt"])]
    async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        value: B256,
    ) -> RpcResult<bool>;

    /// Makes every transaction carrying `signature` recover to `address`.
    #[method(name = "impersonateSignature")]
    async fn anvil_impersonate_signature(
        &self,
        signature: Bytes,
        address: Address,
    ) -> RpcResult<()>;

    /// Returns the state of the chain as gzipped JSON.
    #[method(name = "dumpState", aliases = ["hardhat_dumpState"])]
    async fn anvil_dump_state(&self, preserve_historical_states: Option<bool>) -> RpcResult<Bytes>;

    /// Applies a state dump on top of the current state. Nonces take the higher value.
    #[method(name = "loadState", aliases = ["hardhat_loadState"])]
    async fn anvil_load_state(&self, buf: Bytes) -> RpcResult<bool>;

    /// Sets the balance of an account in a TIP-20 token. Tempo only.
    #[method(name = "dealTIP20")]
    async fn anvil_deal_tip20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()>;

    /// Sets the token an account pays fees with. Tempo only.
    #[method(name = "setFeeToken")]
    async fn anvil_set_fee_token(&self, user: Address, token: Address) -> RpcResult<()>;

    /// Sets the token a validator receives fees in. Tempo only.
    #[method(name = "setValidatorFeeToken")]
    async fn anvil_set_validator_fee_token(
        &self,
        validator: Address,
        token: Address,
    ) -> RpcResult<()>;

    /// Adds Fee AMM liquidity for a token pair. Tempo only.
    #[method(name = "setFeeAmmLiquidity")]
    async fn anvil_set_fee_amm_liquidity(
        &self,
        user_token: Address,
        validator_token: Address,
        amount: U256,
    ) -> RpcResult<()>;
}

/// The `evm_*` methods that have no `anvil_*` counterpart.
#[rpc(server, namespace = "evm")]
pub trait EvmApi {
    /// Mines blocks and returns `"0x0"`, as Hardhat does.
    #[method(name = "mine")]
    async fn evm_mine(&self, opts: Option<MineOptions>) -> RpcResult<String>;
}

/// The `eth_*` methods anvil adds on top of the standard namespace, or replaces.
#[rpc(server, namespace = "eth")]
pub trait EthExtApi<
    TxReq: RpcObject,
    Receipt: RpcObject,
    Tx: RpcObject,
    Blk: RpcObject,
    RawTx: RpcObject,
>
{
    /// Signs and sends a transaction from a dev account. A request without `to` deploys a
    /// contract.
    ///
    /// Replaces reth's method, which rejects a request without `to`, because a create recipient
    /// serializes as `null` and reads back as missing.
    #[method(name = "sendTransaction")]
    async fn eth_send_transaction(&self, request: TxReq) -> RpcResult<B256>;

    /// Signs and sends a transaction, and waits for its receipt.
    #[method(name = "sendTransactionSync")]
    async fn eth_send_transaction_sync(&self, request: TxReq) -> RpcResult<Receipt>;

    /// Sends a transaction from `from` without a signature, as if the account were impersonated.
    #[method(name = "sendUnsignedTransaction")]
    async fn eth_send_unsigned_transaction(&self, request: TxReq) -> RpcResult<B256>;

    /// Sends `request` again with a new gas price or gas limit, replacing the pending
    /// transaction with the same nonce.
    #[method(name = "resend")]
    async fn eth_resend(
        &self,
        request: TxReq,
        gas_price: Option<U256>,
        gas_limit: Option<U64>,
    ) -> RpcResult<B256>;

    /// Sends a signed transaction. A replacement must raise the fee, as on anvil.
    #[method(name = "sendRawTransaction")]
    async fn eth_send_raw_transaction(&self, tx: Bytes) -> RpcResult<B256>;

    /// Sends a signed transaction. The condition is accepted and ignored, as anvil does.
    #[method(name = "sendRawTransactionConditional")]
    async fn eth_send_raw_transaction_conditional(
        &self,
        tx: Bytes,
        condition: TransactionConditional,
    ) -> RpcResult<B256>;

    /// Returns the dev accounts, like `eth_accounts`.
    #[method(name = "requestAccounts")]
    async fn eth_request_accounts(&self) -> RpcResult<Vec<Address>>;

    /// Returns the chain id as a decimal string, like `net_version`.
    #[method(name = "networkId")]
    async fn eth_network_id(&self) -> RpcResult<Option<String>>;

    /// Returns the gas price: the base fee plus the suggested tip, the base fee alone when the
    /// minimum priority fee is disabled, or the node's gas price before London, as anvil does.
    #[method(name = "gasPrice")]
    async fn eth_gas_price(&self) -> RpcResult<U256>;

    /// Estimates the gas of a call down to the exact limit, as anvil does; reth stops its search
    /// within 1.5% above it. A request without `from` is not capped by the zero address's
    /// balance, and fee fields below the base fee do not fail the call.
    #[method(name = "estimateGas")]
    async fn eth_estimate_gas(
        &self,
        request: TxReq,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<U256>;

    /// Runs a call. Fee fields below the base fee do not fail the call, as on anvil.
    #[method(name = "call")]
    async fn eth_call(
        &self,
        request: TxReq,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<Bytes>;

    /// Runs bundles of calls. Every bundle answers, an empty one with no results, and each
    /// bundle runs one block later than the one before it, as on anvil.
    #[method(name = "callMany")]
    async fn eth_call_many(
        &self,
        bundles: Vec<Bundle<TxReq>>,
        state_context: Option<StateContext>,
        state_override: Option<StateOverride>,
    ) -> RpcResult<Vec<Vec<EthCallResponse>>>;

    /// Returns the base fee of the next block, with the `anvil_setNextBlockBaseFeePerGas`
    /// override.
    #[method(name = "baseFee")]
    async fn eth_base_fee(&self) -> RpcResult<Option<U256>>;

    /// Returns the fee history. The entry for the block after the newest one comes from that
    /// block when it exists, or from the next-block override, where reth computes it from the
    /// newest block alone; a block without a gas limit has a zero gas-used ratio instead of NaN.
    #[method(name = "feeHistory")]
    async fn eth_fee_history(
        &self,
        block_count: U64,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Option<Vec<f64>>,
    ) -> RpcResult<FeeHistory>;

    /// Creates an access list for a request. From MonadTen on, Monad lists one storage key per
    /// storage page, as anvil does.
    #[method(name = "createAccessList")]
    async fn eth_create_access_list(
        &self,
        request: TxReq,
        block: Option<BlockId>,
        state_override: Option<StateOverride>,
    ) -> RpcResult<AccessListResult>;

    /// Returns the receipt of a transaction. An impersonated transaction has no valid signature,
    /// so a lookup that recovers the sender from it fails; the lookup then runs again with the
    /// block in the RPC cache, which carries the senders the block recorded.
    #[method(name = "getTransactionReceipt")]
    async fn eth_get_transaction_receipt(&self, hash: B256) -> RpcResult<Option<Receipt>>;

    /// Returns a transaction by hash; see `eth_getTransactionReceipt` for impersonated
    /// transactions.
    #[method(name = "getTransactionByHash")]
    async fn eth_get_transaction_by_hash(&self, hash: B256) -> RpcResult<Option<Tx>>;

    /// Simulates blocks of calls. A call with a sidecar gets the blob hashes the sidecar
    /// carries, as on anvil; reth needs the hashes.
    #[method(name = "simulateV1")]
    async fn eth_simulate_v1(
        &self,
        payload: SimulatePayload<TxReq>,
        block: Option<BlockId>,
    ) -> RpcResult<Vec<SimulatedBlock<Blk>>>;

    /// Returns the block access list of a block. Before Amsterdam the list is `null`, a block
    /// above the head is an error, and on a fork the blocks at or below the fork block and the
    /// unknown hashes come from the fork endpoint, as on anvil.
    #[method(name = "getBlockAccessList")]
    async fn eth_block_access_list(&self, block: BlockId) -> RpcResult<Option<serde_json::Value>>;

    /// Returns the raw block access list of a block; see `eth_getBlockAccessList`.
    #[method(name = "getBlockAccessListRaw")]
    async fn eth_block_access_list_raw(&self, block: BlockId) -> RpcResult<Option<Bytes>>;

    /// Returns the block access list of a block by hash; see `eth_getBlockAccessList`.
    #[method(name = "getBlockAccessListByBlockHash")]
    async fn eth_block_access_list_by_block_hash(
        &self,
        hash: B256,
    ) -> RpcResult<Option<serde_json::Value>>;

    /// Returns the block access list of a block by number; see `eth_getBlockAccessList`.
    #[method(name = "getBlockAccessListByBlockNumber")]
    async fn eth_block_access_list_by_block_number(
        &self,
        number: BlockNumberOrTag,
    ) -> RpcResult<Option<serde_json::Value>>;

    /// Returns the EIP-2718 encoding of a transaction. A blob transaction comes without its
    /// sidecar, in the consensus encoding, as on anvil; reth returns the pooled encoding.
    #[method(name = "getRawTransactionByHash")]
    async fn eth_raw_transaction_by_hash(&self, hash: B256) -> RpcResult<Option<Bytes>>;

    /// Returns the transaction count of an account. At `pending`, the count comes from the pool
    /// and the latest state, as on anvil, without building reth's pending block, whose cache
    /// would then serve a block without the transactions that arrive in the next second.
    #[method(name = "getTransactionCount")]
    async fn eth_transaction_count(
        &self,
        address: Address,
        block: Option<BlockId>,
    ) -> RpcResult<U256>;

    /// Installs a log filter. A filter without `fromBlock` reports the blocks after the current
    /// one, as on anvil; reth's first poll includes the current block.
    #[method(name = "newFilter")]
    async fn eth_new_filter(&self, filter: Filter) -> RpcResult<FilterId>;

    /// Installs a block filter that reports the blocks after the current one, as on anvil.
    #[method(name = "newBlockFilter")]
    async fn eth_new_block_filter(&self) -> RpcResult<FilterId>;

    /// Returns the uncle count of a block, and fails for an unknown block, as anvil does; reth
    /// answers `null`.
    #[method(name = "getUncleCountByBlockHash")]
    async fn eth_block_uncles_count_by_hash(&self, hash: B256) -> RpcResult<Option<U256>>;

    /// Returns the uncle count of a block, and fails for a block above the head, as anvil does.
    #[method(name = "getUncleCountByBlockNumber")]
    async fn eth_block_uncles_count_by_number(
        &self,
        number: BlockNumberOrTag,
    ) -> RpcResult<Option<U256>>;

    /// Signs a transaction with a dev account. The chain id and the gas limit are filled in, as
    /// on anvil; reth requires them.
    #[method(name = "signTransaction")]
    async fn eth_sign_transaction(&self, request: TxReq) -> RpcResult<Bytes>;

    /// Signs a sender-signed Tempo transaction as the node's fee payer and returns it, without
    /// sending it. This is the sign-only mode of Tempo's fee payer service. Tempo only.
    #[method(name = "signRawTransaction")]
    async fn eth_sign_raw_transaction(&self, tx: Bytes) -> RpcResult<Bytes>;

    /// Fills the defaults of a transaction request and returns it with its unsigned encoding.
    #[method(name = "fillTransaction")]
    async fn eth_fill_transaction(&self, request: TxReq) -> RpcResult<FillTransaction<RawTx>>;

    /// Returns the native balance of an account. Tempo's API reports a placeholder for it;
    /// anvil reports the balance.
    #[method(name = "getBalance")]
    async fn eth_get_balance(&self, address: Address, block: Option<BlockId>) -> RpcResult<U256>;

    /// Returns the coinbase of the next block: the override set by `anvil_setCoinbase`, else the
    /// genesis coinbase.
    #[method(name = "coinbase")]
    async fn eth_coinbase(&self) -> RpcResult<Address>;

    /// Signs typed data, like `eth_signTypedData`.
    #[method(name = "signTypedData_v4")]
    async fn eth_sign_typed_data_v4(&self, address: Address, data: TypedData) -> RpcResult<Bytes>;

    /// Sends a signed transaction and waits for its receipt, for `timeout_ms` at most.
    #[method(name = "sendRawTransactionSync")]
    async fn eth_send_raw_transaction_sync(
        &self,
        tx: Bytes,
        timeout_ms: Option<u64>,
    ) -> RpcResult<Receipt>;
}

/// The `web3_*` methods anvil replaces.
#[rpc(server, namespace = "web3")]
pub trait Web3ExtApi {
    /// Returns the client version, `reth-anvil/v<version>`.
    #[method(name = "clientVersion")]
    async fn web3_client_version(&self) -> RpcResult<String>;
}

/// The `personal_*` namespace.
#[rpc(server, namespace = "personal")]
pub trait PersonalApi {
    /// Signs `message` with `address`, like `eth_sign` with the parameters swapped.
    #[method(name = "sign")]
    async fn personal_sign(&self, message: Bytes, address: Address) -> RpcResult<Bytes>;
}

/// The client version `anvil_metadata` and `web3_clientVersion` report.
pub const CLIENT_VERSION: &str = concat!(env!("CARGO_PKG_NAME"), "/v", env!("CARGO_PKG_VERSION"));

/// The header type of a provider.
type HeaderOf<Provider> = <Provider as HeaderProvider>::Header;

/// A transaction request, which may carry a batch of calls in place of one recipient.
pub trait CallBatch {
    /// Returns whether the request carries a batch of calls.
    fn has_calls(&self) -> bool {
        false
    }

    /// Returns where the nonce of the request's sender lives.
    fn nonce_lane(&self, _from: Address) -> NonceLane {
        NonceLane::Account
    }

    /// Returns whether a signature in the request covers its gas limit, as a Tempo fee payer's
    /// does.
    fn signs_gas(&self) -> bool {
        false
    }
}

/// Where the nonce of a transaction lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NonceLane {
    /// The sender's account nonce.
    Account,
    /// No nonce: an expiring nonce transaction carries nonce zero.
    Expiring,
    /// A storage slot of a contract, as a Tempo nonce lane.
    Storage(Address, U256),
}

impl CallBatch for TransactionRequest {}

#[cfg(feature = "tempo")]
impl CallBatch for tempo_alloy::rpc::TempoTransactionRequest {
    fn has_calls(&self) -> bool {
        !self.calls.is_empty()
    }

    fn signs_gas(&self) -> bool {
        self.fee_payer_signature.is_some()
    }

    fn nonce_lane(&self, from: Address) -> NonceLane {
        match self.nonce_key.filter(|key| !key.is_zero()) {
            None => NonceLane::Account,
            Some(TEMPO_EXPIRING_NONCE_KEY) => NonceLane::Expiring,
            Some(key) => NonceLane::Storage(
                NONCE_PRECOMPILE_ADDRESS,
                NonceManager::new().nonces[from][key].slot(),
            ),
        }
    }
}

/// Marks a request without `to` or calls as a contract creation, so the signer can build it.
fn with_recipient<TxReq: AsMut<TransactionRequest> + CallBatch>(mut request: TxReq) -> TxReq {
    if !request.has_calls() && request.as_mut().to.is_none() {
        request.as_mut().to = Some(TxKind::Create);
    }
    request
}

/// Funds the zero address, which stands in for a missing `from`, so a request with fee fields is
/// not capped by that balance. Anvil charges no fee for a request without `from`.
fn fund_default_caller(
    request: &TransactionRequest,
    overrides: Option<StateOverride>,
) -> Option<StateOverride> {
    if request.from.is_some() {
        return overrides;
    }
    let mut overrides = overrides.unwrap_or_default();
    overrides.entry(Address::ZERO).or_default().balance.get_or_insert(U256::from(u128::MAX));
    Some(overrides)
}

/// Gives a revert without data the empty data anvil reports, where reth leaves it out.
fn with_revert_data(error: ErrorObjectOwned) -> ErrorObjectOwned {
    if error.code() != REVERT_ERROR_CODE {
        return error;
    }
    let Some(data) = error.data() else {
        return ErrorObjectOwned::owned(error.code(), error.message().to_string(), Some("0x"));
    };
    // The message decodes the revert data as anvil does: the reason string, the panic, or the
    // custom error with its data. Tempo's API would name a precompile error by its selector,
    // which several precompiles share.
    match serde_json::from_str::<Bytes>(data.get()) {
        Ok(revert) => {
            let mut message = "execution reverted".to_string();
            if let Some(reason) = RevertDecoder::new().maybe_decode(&revert, None) {
                message = format!("{message}: {reason}");
            }
            ErrorObjectOwned::owned(error.code(), message, Some(revert))
        }
        Err(_) => error,
    }
}

/// The error code of a reverted call, as anvil and reth report it.
const REVERT_ERROR_CODE: i32 = 3;

/// The error code anvil reports for a transaction or a call it rejects.
const TRANSACTION_REJECTED_CODE: i32 = -32003;

/// How long a probe of another endpoint's `anvil_*` identity waits.
const ENDPOINT_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Encodes a blob as a hex string on the heap: serde's encoding of a 128 KiB blob goes through
/// a buffer of twice that size on the stack.
fn encode_blob(blob: &Blob) -> String {
    alloy_primitives::hex::encode_prefixed(blob.as_slice())
}

/// Returns the consensus encoding of a raw transaction: a pooled blob transaction loses its
/// sidecar; any other encoding is returned as it is.
fn without_sidecar(raw: Bytes) -> Bytes {
    if raw.first() != Some(&EIP4844_TX_TYPE_ID) {
        return raw;
    }
    match PooledTransactionVariant::decode_2718(&mut raw.as_ref()) {
        Ok(pooled) => pooled
            .into_envelope()
            .map_eip4844(|tx| match tx {
                TxEip4844Variant::TxEip4844(tx) => tx,
                TxEip4844Variant::TxEip4844WithSidecar(tx) => tx.tx,
            })
            .encoded_2718()
            .into(),
        Err(_) => raw,
    }
}

/// Where a block access list request is answered from.
enum AccessListRoute {
    /// The block has no list: Amsterdam is not active at it.
    Null,
    /// The fork endpoint serves the block.
    Forward,
    /// Reth serves the block.
    Local,
}

/// Reth's error for a transaction whose sender cannot be recovered.
const INVALID_SIGNATURE_MESSAGE: &str = "invalid transaction signature";

/// The error of a sender that cannot pay for a transaction.
fn insufficient_funds(cost: U256, balance: U256) -> ErrorObjectOwned {
    EthApiError::InvalidTransaction(RpcInvalidTransactionError::InsufficientFunds { cost, balance })
        .into()
}

/// How long a snapshot revert waits for the pool to take the reverted transactions back.
const POOL_RESTORE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `eth_sendTransactionSync` waits for the receipt.
const TRANSACTION_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(30);

/// The error code of a transaction confirmation timeout, as anvil reports it.
const TRANSACTION_CONFIRMATION_TIMEOUT_CODE: i32 = 4;

/// How often `eth_sendTransactionSync` polls for the receipt.
const RECEIPT_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// What `anvil_nodeInfo` reports about the network the node runs.
#[derive(Clone, Debug, Default)]
pub struct NodeIdentity {
    /// The network family, if not Ethereum.
    pub network: Option<&'static str>,
    /// The hardfork name, when the chain spec's Ethereum forks do not describe it.
    pub hardfork: Option<String>,
}

/// Implementation of the `anvil_*` RPC namespace.
#[derive(Debug, Clone)]
pub struct AnvilRpc<Pool, Provider: HeaderProvider, Eth, Spec> {
    identity: NodeIdentity,
    relauncher: Relauncher,
    impersonation: ImpersonationState,
    mining: MiningController<HeaderOf<Provider>>,
    time: TimeManager,
    block_env: BlockEnvOverrides,
    state: SharedAnvilState,
    snapshots: SnapshotManager<HeaderOf<Provider>>,
    chain_spec: Arc<Spec>,
    instance_id: Arc<RwLock<B256>>,
    logging: LoggingState,
    transaction_order: TransactionOrder,
    min_priority_fee_enforced: bool,
    fork: Option<Arc<dyn ForkInfo>>,
    pool: Pool,
    provider: Provider,
    eth: Eth,
    /// One lock per sender for requests without a nonce, so concurrent requests get distinct
    /// nonces.
    nonce_locks: Arc<Mutex<HashMap<Address, Arc<AsyncMutex<()>>>>>,
    /// Installs a log filter that reports the blocks after the current one; see
    /// `eth_newFilter`.
    new_filter: NewFilterHook,
    /// The dev account that sponsors Tempo fee-payer requests, if any.
    tempo_fee_payer: Option<PrivateKeySigner>,
    /// The chain spec's base fee rule for the block after a header.
    next_block_base_fee: NextBlockBaseFee<HeaderOf<Provider>>,
    /// Makes the pool drop the state it read at the tip, after an anvil state write.
    pool_refresh: Option<PoolRefresh>,
    /// Whether the first block takes the genesis base fee; see
    /// `AnvilNetwork::FIRST_BLOCK_KEEPS_GENESIS_BASE_FEE`.
    first_block_keeps_genesis_base_fee: bool,
}

/// Makes the pool drop the state it read at the tip. Tempo's pool keeps the reads it made at a
/// tip until the next block, so an anvil state write would stay hidden from it until then.
#[derive(Clone)]
pub struct PoolRefresh(Arc<dyn Fn() + Send + Sync>);

impl PoolRefresh {
    /// Wraps the refresh.
    pub fn new(refresh: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(refresh))
    }
}

impl fmt::Debug for PoolRefresh {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PoolRefresh")
    }
}

/// The base fee of the block after a header at a timestamp, by the chain spec's rule: EIP-1559,
/// or Tempo's fixed fee and its T7 controller.
#[derive(Clone)]
struct NextBlockBaseFee<H>(NextBlockBaseFeeFn<H>);

/// The base fee rule of a chain spec, over its header type.
type NextBlockBaseFeeFn<H> = Arc<dyn Fn(&H, u64) -> Option<u64> + Send + Sync>;

impl<H> fmt::Debug for NextBlockBaseFee<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NextBlockBaseFee")
    }
}

/// Installs a log filter, or a block filter for `None`, and returns its id.
#[derive(Clone)]
pub struct NewFilterHook(
    Arc<dyn Fn(Option<Filter>) -> HookFuture<RpcResult<FilterId>> + Send + Sync>,
);

impl fmt::Debug for NewFilterHook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NewFilterHook")
    }
}

impl NewFilterHook {
    /// Wraps the given installer.
    pub fn new(
        install: impl Fn(Option<Filter>) -> HookFuture<RpcResult<FilterId>> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(install))
    }
}

impl<Pool, Provider: HeaderProvider, Eth, Spec> AnvilRpc<Pool, Provider, Eth, Spec>
where
    Spec: EthChainSpec<Header = HeaderOf<Provider>> + 'static,
{
    /// Creates the `anvil_*` namespace over the given node components.
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        identity: NodeIdentity,
        relauncher: Relauncher,
        impersonation: ImpersonationState,
        mining: MiningController<HeaderOf<Provider>>,
        time: TimeManager,
        block_env: BlockEnvOverrides,
        state: SharedAnvilState,
        snapshots: SnapshotManager<HeaderOf<Provider>>,
        chain_spec: Arc<Spec>,
        instance_id: Arc<RwLock<B256>>,
        logging: LoggingState,
        transaction_order: TransactionOrder,
        min_priority_fee_enforced: bool,
        fork: Option<Arc<dyn ForkInfo>>,
        pool: Pool,
        provider: Provider,
        eth: Eth,
        new_filter: NewFilterHook,
    ) -> Self {
        let next_block_base_fee = {
            let chain_spec = chain_spec.clone();
            NextBlockBaseFee(Arc::new(move |header: &HeaderOf<Provider>, timestamp| {
                chain_spec.next_block_base_fee(header, timestamp)
            }))
        };
        Self {
            identity,
            relauncher,
            impersonation,
            mining,
            time,
            block_env,
            state,
            snapshots,
            chain_spec,
            instance_id,
            logging,
            transaction_order,
            min_priority_fee_enforced,
            fork,
            pool,
            provider,
            eth,
            nonce_locks: Default::default(),
            new_filter,
            tempo_fee_payer: None,
            next_block_base_fee,
            pool_refresh: None,
            first_block_keeps_genesis_base_fee: true,
        }
    }

    /// Sets whether the first block takes the genesis base fee, as on Ethereum, or the fee the
    /// chain spec's rule gives it.
    pub const fn with_first_block_keeps_genesis_base_fee(mut self, keeps: bool) -> Self {
        self.first_block_keeps_genesis_base_fee = keeps;
        self
    }

    /// Sets the refresh that makes the pool see anvil state writes before the next block.
    pub fn with_pool_refresh(mut self, refresh: Option<PoolRefresh>) -> Self {
        self.pool_refresh = refresh;
        self
    }

    /// Sets the dev account that sponsors Tempo fee-payer requests.
    pub fn with_tempo_fee_payer(mut self, fee_payer: Option<PrivateKeySigner>) -> Self {
        self.tempo_fee_payer = fee_payer;
        self
    }
}

impl<Pool, Provider: BlockNumReader + HeaderProvider, Eth, Spec>
    AnvilRpc<Pool, Provider, Eth, Spec>
{
    fn best_block_number(&self) -> RpcResult<u64> {
        self.provider
            .best_block_number()
            .map_err(|error| internal_error(format!("failed to read latest block number: {error}")))
    }

    fn sealed_header(&self, number: u64) -> RpcResult<SealedHeader<HeaderOf<Provider>>> {
        self.provider
            .sealed_header(number)
            .map_err(|error| internal_error(format!("failed to read header {number}: {error}")))?
            .ok_or_else(|| internal_error(format!("missing block header {number}")))
    }
}

impl<Pool, Provider, Eth, Spec> AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: TransactionPool,
    Provider:
        BlockNumReader + HeaderProvider + TransactionsProvider + StateProviderFactory + StateDump,
    Eth: FullEthApiServer<NetworkTypes: RpcTypes<TransactionRequest: Default + CallBatch>>,
    Spec: EthChainSpec + EthereumHardforks + Hardforks,
{
    async fn block_by_number(
        &self,
        number: u64,
        full: bool,
    ) -> RpcResult<RpcBlock<Eth::NetworkTypes>> {
        EthApiServer::block_by_number(&self.eth, BlockNumberOrTag::Number(number), full)
            .await?
            .ok_or_else(|| internal_error(format!("missing block {number}")))
    }

    async fn latest_block(&self) -> RpcResult<RpcBlock<Eth::NetworkTypes>> {
        self.block_by_number(self.best_block_number()?, false).await
    }

    /// Mines `blocks` blocks and returns their numbers.
    async fn mine_blocks(&self, blocks: u64) -> RpcResult<Vec<u64>> {
        // Anvil mines the blocks of one request within the same second. Blocks take longer
        // here, so they share the first block's timestamp.
        let shared = (blocks > 1 && self.time.interval().is_none())
            .then(|| self.time.current_call_timestamp());
        let mut mined = Vec::with_capacity(blocks as usize);
        for _ in 0..blocks {
            if let Some(timestamp) = shared {
                self.time.pin_next_timestamp(timestamp);
            }
            mined.push(self.mining.mine_block().await.map_err(internal_error)?.number());
        }
        Ok(mined)
    }

    /// Rewinds the chain to the given canonical header and drops the transactions of the removed
    /// blocks, so the pool does not mine them again.
    async fn rewind_to(&self, header: &SealedHeader<HeaderOf<Provider>>) -> RpcResult<()> {
        self.rewind_to_keeping(header, &HashSet::new()).await?;
        Ok(())
    }

    /// Rewinds the chain to `header`. The transactions of the removed blocks are dropped, so the
    /// pool does not take them back, except the ones in `keep`, whose hashes are returned.
    async fn rewind_to_keeping(
        &self,
        header: &SealedHeader<HeaderOf<Provider>>,
        keep: &HashSet<B256>,
    ) -> RpcResult<Vec<B256>> {
        let best = self.best_block_number()?;
        let mut kept = Vec::new();
        if header.number() < best {
            let removed = self
                .provider
                .transactions_by_block_range(header.number() + 1..=best)
                .map_err(|error| internal_error(format!("failed to read transactions: {error}")))?;
            let (keep_hashes, drop_hashes): (Vec<_>, Vec<_>) = removed
                .into_iter()
                .flatten()
                .map(|tx| *tx.tx_hash())
                .partition(|hash| keep.contains(hash));
            kept = keep_hashes;
            self.impersonation.drop_txs(drop_hashes);
        }
        self.mining.rewind(header.clone()).await.map_err(internal_error)?;
        self.state.write().rewind_to(header.number());
        Ok(kept)
    }

    /// Brings the pool back to the transactions in `keep` after a rewind to `head`: waits for
    /// the pool to take the removed blocks' transactions in `restored` back, and removes the
    /// transactions that are not in `keep`.
    async fn restore_pool(&self, head: B256, keep: &HashSet<B256>, restored: &[B256]) {
        wait_for_pool(&self.pool, head).await;
        let deadline = Instant::now() + POOL_RESTORE_TIMEOUT;
        while restored.iter().any(|hash| !self.pool.contains(hash)) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let extra: Vec<B256> = self
            .pool
            .all_transaction_hashes()
            .into_iter()
            .filter(|hash| !keep.contains(hash))
            .collect();
        if !extra.is_empty() {
            self.pool.remove_transactions(extra.clone());
            self.impersonation.forget_tx_senders(extra);
        }
    }

    /// Tells the pool the nonce and balance of an account after a state write, so it promotes or
    /// parks the account's transactions, as anvil's pool sees the write at once.
    fn sync_pool_account(&self, address: Address) -> RpcResult<()>
    where
        Pool: TransactionPoolExt,
    {
        let account = self
            .provider
            .latest()
            .and_then(|state| state.basic_account(&address))
            .map_err(|error| internal_error(format!("failed to read account: {error}")))?
            .unwrap_or_default();
        self.pool.update_accounts(vec![ChangedAccount {
            address,
            nonce: account.nonce,
            balance: account.balance,
        }]);
        self.refresh_pool();
        Ok(())
    }

    /// Returns the balance of the account in the latest state, including anvil state writes.
    fn latest_balance(&self, address: Address) -> RpcResult<U256> {
        Ok(self
            .provider
            .latest()
            .and_then(|state| state.basic_account(&address))
            .map_err(|error| internal_error(format!("failed to read account: {error}")))?
            .unwrap_or_default()
            .balance)
    }

    /// Finds the storage slot of `token_address` that `calldata` reads, by checking which slot
    /// from the access list changes the call result to `expected_value`.
    async fn find_erc20_storage_slot(
        &self,
        token_address: Address,
        calldata: Bytes,
        expected_value: U256,
    ) -> RpcResult<B256> {
        let mut tx = RpcTxReq::<Eth::NetworkTypes>::default();
        *tx.as_mut() = TransactionRequest::default().with_to(token_address).with_input(calldata);
        let access_list =
            EthApiServer::create_access_list(&self.eth, tx.clone(), None, None).await?.access_list;

        for item in access_list.0 {
            if item.address != token_address {
                continue;
            }
            for slot in item.storage_keys {
                let state_override = StateOverridesBuilder::default()
                    .append(
                        token_address,
                        AccountOverride::default()
                            .with_state_diff(std::iter::once((slot, expected_value.into()))),
                    )
                    .build();
                let Ok(result) =
                    EthApiServer::call(&self.eth, tx.clone(), None, Some(state_override), None)
                        .await
                else {
                    continue;
                };
                if U256::from_be_slice(result.as_ref()) == expected_value {
                    return Ok(slot);
                }
            }
        }

        Err(internal_error("Unable to find storage slot"))
    }

    /// Reads the accounts, the pending anvil state writes, the block environment, the blocks
    /// and their transactions, and with `preserve_historical_states` the state at every block,
    /// into a state dump.
    fn serializable_state(&self, preserve_historical_states: bool) -> RpcResult<SerializableState> {
        let best = self.best_block_number()?;
        let header = self.sealed_header(best)?;
        let mut accounts = self
            .provider
            .dump_accounts()
            .map_err(|error| internal_error(format!("failed to read accounts: {error}")))?;
        // The anvil state writes not yet in a block.
        {
            let state = self.state.read();
            for (address, account_override) in state.accounts() {
                let record = accounts.entry(*address).or_default();
                if let Some(balance) = account_override.balance() {
                    record.balance = balance;
                }
                if let Some(nonce) = account_override.nonce() {
                    record.nonce = nonce;
                }
                if let Some(code) =
                    account_override.code_hash().and_then(|hash| state.bytecode_by_hash(&hash))
                {
                    record.code = code.original_bytes();
                }
                for (slot, value) in account_override.storage() {
                    if value.is_zero() {
                        record.storage.remove(slot);
                    } else {
                        record.storage.insert(*slot, (*value).into());
                    }
                }
            }
        }
        let block = block_env_from_header::<BlockEnv>(header.header());
        let (blocks, transactions) = self
            .provider
            .dump_blocks(&|hash| self.impersonation.tx_sender(&hash))
            .map_err(|error| internal_error(format!("failed to read blocks: {error}")))?;
        let historical_states = preserve_historical_states
            .then(|| self.provider.dump_snapshots())
            .transpose()
            .map_err(|error| internal_error(format!("failed to read states: {error}")))?
            .map(SerializableHistoricalStates);
        Ok(SerializableState {
            block: Some(block),
            accounts,
            best_block_number: Some(best),
            blocks,
            transactions,
            historical_states,
        })
    }

    /// Returns the lowercase name of the latest hardfork active at the given block.
    fn hardfork_name(&self, timestamp: u64, number: u64) -> String {
        self.chain_spec
            .forks_iter()
            .filter(|(_, condition)| condition.active_at_timestamp_or_number(timestamp, number))
            .last()
            .map(|(fork, _)| fork.name().to_string())
            .unwrap_or_default()
    }
}

#[async_trait]
impl<Pool, Provider, Eth, Spec>
    AnvilApiServer<RpcBlock<Eth::NetworkTypes>, RpcTxReq<Eth::NetworkTypes>>
    for AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: TransactionPool + TransactionPoolExt + Send + Sync + 'static,
    Provider: BlockNumReader
        + HeaderProvider
        + TransactionsProvider
        + StateProviderFactory
        + StateDump
        + Send
        + Sync
        + 'static,
    Eth: FullEthApiServer<NetworkTypes: RpcTypes<TransactionRequest: Default + CallBatch>>,
    Spec: EthChainSpec + EthereumHardforks + Hardforks + Send + Sync + 'static,
{
    async fn anvil_impersonate_account(&self, address: Address) -> RpcResult<()> {
        self.impersonation.impersonate(address);
        Ok(())
    }

    async fn anvil_stop_impersonating_account(&self, address: Address) -> RpcResult<()> {
        self.impersonation.stop_impersonating(address);
        Ok(())
    }

    async fn anvil_auto_impersonate_account(&self, enabled: bool) -> RpcResult<()> {
        self.impersonation.set_auto_impersonate(enabled);
        Ok(())
    }

    async fn anvil_get_automine(&self) -> RpcResult<bool> {
        Ok(self.mining.is_automine())
    }

    async fn anvil_get_interval_mining(&self) -> RpcResult<Option<u64>> {
        Ok(self.mining.interval_mining())
    }

    async fn anvil_set_automine(&self, enabled: bool) -> RpcResult<()> {
        self.mining.set_automine(enabled);
        if enabled {
            self.mining.trigger_if_pending();
        }
        Ok(())
    }

    async fn anvil_set_interval_mining(&self, interval: u64) -> RpcResult<()> {
        self.mining.set_interval_mining(interval);
        Ok(())
    }

    async fn anvil_mine(&self, num_blocks: Option<U256>, interval: Option<U256>) -> RpcResult<()> {
        let blocks = num_blocks.unwrap_or(U256::ONE).to::<u64>();

        let Some(interval) = interval.filter(|interval| !interval.is_zero()) else {
            self.mine_blocks(blocks).await?;
            return Ok(());
        };

        let previous_interval = self.time.interval();
        self.time.set_block_timestamp_interval(interval.to::<u64>());
        let result = self.mine_blocks(blocks).await;
        match previous_interval {
            Some(previous_interval) => self.time.set_block_timestamp_interval(previous_interval),
            None => {
                self.time.remove_block_timestamp_interval();
            }
        }
        result?;
        Ok(())
    }

    async fn anvil_mine_detailed(
        &self,
        opts: Option<MineOptions>,
    ) -> RpcResult<Vec<RpcBlock<Eth::NetworkTypes>>> {
        let (timestamp, blocks) = match opts.unwrap_or_default() {
            MineOptions::Options { timestamp, blocks } => (timestamp, blocks.unwrap_or(1)),
            MineOptions::Timestamp(timestamp) => (timestamp, 1),
        };

        if let Some(timestamp) = timestamp {
            self.time.set_next_block_timestamp(timestamp).map_err(invalid_params)?;
        }

        let mut mined = Vec::with_capacity(blocks as usize);
        for number in self.mine_blocks(blocks).await? {
            mined.push(self.block_by_number(number, true).await?);
        }
        Ok(mined)
    }

    async fn anvil_snapshot(&self) -> RpcResult<U256> {
        let header = self.sealed_header(self.best_block_number()?)?;
        let snapshot = Snapshot {
            header,
            state: self.state.read().clone(),
            time: self.time.snapshot(),
            block_env: self.block_env.snapshot(),
            pool: self.pool.all_transaction_hashes(),
        };
        Ok(self.snapshots.insert(snapshot))
    }

    async fn anvil_revert(&self, id: U256) -> RpcResult<bool> {
        let Some(snapshot) = self.snapshots.take(id) else {
            return Ok(false);
        };
        // The pool goes back to the snapshot too, as on anvil: the transactions mined since
        // return to it, the ones sent since go.
        let keep: HashSet<B256> = snapshot.pool.iter().copied().collect();
        let restored = self.rewind_to_keeping(&snapshot.header, &keep).await?;
        *self.state.write() = snapshot.state;
        self.time.restore(snapshot.time);
        self.block_env.restore(snapshot.block_env);
        self.restore_pool(snapshot.header.hash(), &keep, &restored).await;
        if self.mining.is_automine() {
            self.mining.trigger_if_pending();
        }
        Ok(true)
    }

    async fn anvil_rollback(&self, depth: Option<u64>) -> RpcResult<()> {
        let depth = depth.unwrap_or(1);
        let best = self.best_block_number()?;
        let target = best.checked_sub(depth).ok_or_else(|| {
            invalid_params(format!("cannot roll back {depth} blocks from {best}"))
        })?;
        let header = self.sealed_header(target)?;
        self.rewind_to(&header).await
    }

    async fn anvil_reorg(
        &self,
        params: ReorgParams<RpcTxReq<Eth::NetworkTypes>>,
        tx_block_pairs: Option<Vec<(TransactionData<RpcTxReq<Eth::NetworkTypes>>, u64)>>,
    ) -> RpcResult<()> {
        let ReorgOptions { depth, mut tx_block_pairs } = params.into_options(tx_block_pairs);
        if let Some((_, number)) = tx_block_pairs.iter().find(|(_, number)| *number >= depth) {
            let Some(last_block) = depth.checked_sub(1) else {
                return Err(invalid_params(
                    "Reorg depth must be at least 1 to include transactions",
                ));
            };
            return Err(invalid_params(format!(
                "Block number for reorg tx will exceed the reorged chain height. Block number {number} must not exceed (depth-1) {last_block}"
            )));
        }
        tx_block_pairs.sort_by_key(|(_, number)| *number);

        let current_height = self.best_block_number()?;
        let common_height = current_height.checked_sub(depth).ok_or_else(|| {
            invalid_params(format!(
                "Reorg depth must not exceed current chain height: current height {current_height}, depth {depth}"
            ))
        })?;
        let header = self.sealed_header(common_height)?;
        self.rewind_to(&header).await?;

        // Mine the blocks by hand so interval or automine blocks do not interleave.
        let automine = self.mining.is_automine();
        let interval = self.mining.interval_mining();
        self.mining.set_interval_mining(0);
        let mut pairs = tx_block_pairs.into_iter().peekable();
        let mut result = Ok(());
        for offset in 0..depth {
            while let Some((tx, _)) = pairs.next_if(|(_, number)| *number == offset) {
                let sent = match tx {
                    TransactionData::JSON(request) => match self.with_sender(request) {
                        Ok(request) => EthApiServer::send_transaction(&self.eth, request).await,
                        Err(error) => Err(error),
                    },
                    TransactionData::Raw(bytes) => {
                        EthApiServer::send_raw_transaction(&self.eth, bytes).await
                    }
                };
                if let Err(error) = sent {
                    result = Err(error);
                    break;
                }
            }
            if result.is_err() {
                break;
            }
            if let Err(error) = self.mining.mine_block().await {
                result = Err(internal_error(error));
                break;
            }
        }
        if let Some(interval) = interval {
            self.mining.set_interval_mining(interval);
        }
        self.mining.set_automine(automine);
        result
    }

    async fn anvil_reset(&self, forking: Option<Forking>) -> RpcResult<()> {
        // `anvil_setRpcUrl` on a node without a fork names the endpoint a reset forks.
        let has_fork_url = !self.relauncher.current_config().fork_urls.is_empty();
        if let Some(forking) = forking
            && (self.fork.is_some() || forking.json_rpc_url.is_some() || has_fork_url)
        {
            if let Some(url) = &forking.json_rpc_url {
                if self.is_own_endpoint(url).await {
                    return Err(invalid_params("cannot reset Anvil to its own RPC endpoint"));
                }
                // The node runs one network for good; another network needs another node.
                let current = self.relauncher.current_config();
                let mut target = current.clone();
                target.fork_chain_id = None;
                if !current.explicit_network
                    && let Some(networks) = target
                        .fork_networks(url)
                        .await
                        .map_err(|error| internal_error(format!("{error:#}")))?
                    && !current.networks.supports_fork_source(&networks)
                {
                    return Err(invalid_params(format!(
                        "cannot reset Anvil across network families ({} -> {}); start a new \
                         instance with matching network configuration",
                        current.networks.execution_family_name(),
                        networks.execution_family_name()
                    )));
                }
            }
            // A fork reset relaunches the node on the fork, at the given block or the endpoint's
            // latest one, as anvil does.
            return self
                .relauncher
                .relaunch(|config| {
                    if let Some(url) = forking.json_rpc_url {
                        config.fork_urls = vec![ForkUrl { url, block: None }];
                        config.fork_chain_id = None;
                        config.forget_fork_adoption();
                        // Another endpoint brings its own hardfork; a reset to memory restores
                        // the configured one.
                        config.hardfork = None;
                    }
                    config.fork_choice =
                        forking.block_number.map(|number| ForkChoice::Block(number.into()));
                    config.init_state = None;
                })
                .await
                .map_err(internal_error);
        }
        if self.fork.is_some() {
            // Anvil turns a forked node back into a plain one, with a fresh block environment.
            return self
                .relauncher
                .relaunch_from_original(|config| {
                    config.fork_urls.clear();
                    config.fork_choice = None;
                    config.init_state = None;
                })
                .await
                .map_err(internal_error);
        }
        let genesis = self.sealed_header(self.chain_spec.genesis_header().number())?;
        self.rewind_to(&genesis).await?;
        let hashes = self.pool.all_transaction_hashes();
        if !hashes.is_empty() {
            self.pool.remove_transactions(hashes.clone());
            self.impersonation.forget_tx_senders(hashes);
        }
        *self.state.write() = AnvilState::default();
        self.snapshots.clear();
        self.time.set_time(self.chain_spec.genesis().timestamp);
        self.time.remove_block_timestamp_interval();
        self.block_env.restore(Default::default());
        // The first block keeps the genesis base fee, as at launch.
        if self.first_block_keeps_genesis_base_fee
            && let Some(base_fee) = genesis.base_fee_per_gas()
        {
            self.block_env.set_next_base_fee(base_fee);
        }
        *self.instance_id.write() = B256::random();
        Ok(())
    }

    async fn anvil_set_rpc_url(&self, url: String) -> RpcResult<()> {
        let Some(fork) = &self.fork else {
            // Without a fork, the endpoint waits for a reset that forks it, as on anvil.
            self.relauncher.update_config(|config| {
                config.fork_urls = vec![ForkUrl { url, block: None }];
                config.fork_chain_id = None;
            });
            return Ok(());
        };
        // The replacement must serve the same chain at the same fork block, as anvil checks.
        let provider = ProviderBuilder::<alloy_network::AnyNetwork>::new(&url)
            .build()
            .map_err(|error| invalid_params(format!("invalid fork endpoint {url}: {error}")))?;
        if self.is_own_endpoint(&url).await {
            return Err(invalid_params("cannot set Anvil's fork provider to its own RPC endpoint"));
        }
        // The endpoint's identity is probed before and after the block check: an anvil endpoint
        // that stops answering in between may have reset.
        let config = self.relauncher.current_config();
        let mut probe = NodeInfoProbe::new(false, config.no_fork_node_info);
        let probe_error = |error: eyre::Report| internal_error(format!("{error:#}"));
        let identified = probe.request(&provider).await.map_err(probe_error)?.is_some();
        let chain_id = provider.get_chain_id().await.map_err(|error| {
            internal_error(format!("failed to fetch network chain ID from {url}: {error}"))
        })?;
        crate::fork::ensure_fork_network_supported(chain_id)
            .map_err(|error| invalid_params(error.to_string()))?;
        if chain_id != fork.chain_id() {
            return Err(invalid_params(format!(
                "fork endpoints must use the same chain ID: expected {}, got {chain_id} from {url}",
                fork.chain_id()
            )));
        }
        let block =
            provider.get_block_by_number(fork.block_number().into()).await.map_err(|error| {
                internal_error(format!("failed to confirm the fork block on {url}: {error}"))
            })?;
        probe.request(&provider).await.map_err(probe_error)?;
        if block.map(|block| block.header.hash) != Some(fork.block_hash()) {
            return Err(invalid_params(format!(
                "replacement fork endpoint does not contain active fork block {} with hash {}",
                fork.block_number(),
                fork.block_hash()
            )));
        }
        fork.set_rpc_url(url.clone()).map_err(|error| internal_error(error.to_string()))?;
        if identified {
            config.mark_anvil_endpoint(&url);
        }
        self.relauncher.update_config(|config| {
            config.fork_urls = vec![ForkUrl { url, block: None }];
            config.fork_chain_id = None;
        });
        Ok(())
    }

    async fn anvil_set_min_gas_price(&self, gas_price: U256) -> RpcResult<()> {
        if self.chain_spec.is_london_active_at_block(0) {
            return Err(invalid_params(
                "anvil_setMinGasPrice is not supported when EIP-1559 is active",
            ));
        }
        self.block_env.set_gas_price(gas_price.saturating_to());
        Ok(())
    }

    async fn anvil_set_logging_enabled(&self, enabled: bool) -> RpcResult<()> {
        self.logging.set_enabled(enabled);
        Ok(())
    }

    async fn anvil_get_last_block_wall_time(&self) -> RpcResult<u64> {
        Ok(self.time.last_block_wall_time())
    }

    async fn anvil_get_blob_by_hash(&self, hash: B256) -> RpcResult<Option<String>> {
        let pool = self.pool.clone();
        let blobs = on_large_stack(move || pool.get_blobs_for_versioned_hashes_v1(&[hash]))
            .await
            .map_err(|error| internal_error(format!("failed to read blobs: {error}")))?;
        Ok(blobs.into_iter().flatten().next().map(|blob| encode_blob(&blob.blob)))
    }

    async fn anvil_get_blobs_by_transaction_hash(
        &self,
        hash: B256,
    ) -> RpcResult<Option<Vec<String>>> {
        let pool = self.pool.clone();
        let sidecar = on_large_stack(move || pool.get_blob(hash))
            .await
            .map_err(|error| internal_error(format!("failed to read blobs: {error}")))?;
        Ok(sidecar.map(|sidecar| {
            let blobs = match sidecar.as_ref() {
                BlobTransactionSidecarVariant::Eip4844(sidecar) => &sidecar.blobs,
                BlobTransactionSidecarVariant::Eip7594(sidecar) => &sidecar.blobs,
            };
            blobs.iter().map(encode_blob).collect()
        }))
    }

    async fn anvil_deal_erc20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()> {
        const BALANCE_OF_SELECTOR: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];

        let mut calldata = Vec::with_capacity(4 + 32);
        calldata.extend_from_slice(&BALANCE_OF_SELECTOR);
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(address.as_slice());

        // A TIP-20 token keeps its balances in precompile storage, which no call reveals.
        if self.is_tempo() && self.try_set_tip20_balance(address, token_address, balance)? {
            return Ok(());
        }
        let slot = self.find_erc20_storage_slot(token_address, calldata.into(), balance).await?;
        self.state.write().set_storage_at(token_address, slot, balance);
        Ok(())
    }

    async fn anvil_set_erc20_allowance(
        &self,
        owner: Address,
        spender: Address,
        token_address: Address,
        amount: U256,
    ) -> RpcResult<()> {
        const ALLOWANCE_SELECTOR: [u8; 4] = [0xdd, 0x62, 0xed, 0x3e];

        let mut calldata = Vec::with_capacity(4 + 32 + 32);
        calldata.extend_from_slice(&ALLOWANCE_SELECTOR);
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(owner.as_slice());
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(spender.as_slice());

        let slot = self.find_erc20_storage_slot(token_address, calldata.into(), amount).await?;
        self.state.write().set_storage_at(token_address, slot, amount);
        Ok(())
    }

    async fn anvil_drop_transaction(&self, tx_hash: B256) -> RpcResult<Option<B256>> {
        let Some(tx) = self.pool.get(&tx_hash) else {
            return Ok(None);
        };
        // The sender's later transactions go with it, as on anvil; reth only parks them.
        let (sender, nonce) = (tx.sender(), tx.nonce());
        let mut hashes = vec![tx_hash];
        hashes.extend(
            self.pool
                .get_transactions_by_sender(sender)
                .into_iter()
                .filter(|tx| tx.nonce() > nonce)
                .map(|tx| *tx.hash()),
        );
        self.pool.remove_transactions(hashes.clone());
        self.impersonation.forget_tx_senders(hashes);
        Ok(Some(tx_hash))
    }

    async fn anvil_drop_all_transactions(&self) -> RpcResult<()> {
        let hashes = self.pool.all_transaction_hashes();
        if !hashes.is_empty() {
            self.pool.remove_transactions(hashes.clone());
            self.impersonation.forget_tx_senders(hashes);
        }
        Ok(())
    }

    async fn anvil_remove_pool_transactions(&self, address: Address) -> RpcResult<()> {
        let removed = self.pool.remove_transactions_by_sender(address);
        self.impersonation.forget_tx_senders(removed.into_iter().map(|tx| *tx.hash()));
        Ok(())
    }

    async fn anvil_get_genesis_time(&self) -> RpcResult<u64> {
        let header = self
            .provider
            .sealed_header(0)
            .map_err(|error| internal_error(format!("failed to read genesis header: {error}")))?
            .ok_or_else(|| internal_error("genesis block not found"))?;
        Ok(header.timestamp())
    }

    async fn anvil_node_info(&self) -> RpcResult<NodeInfo> {
        let latest = self.latest_block().await?;
        let gas_price = EthApiServer::gas_price(&self.eth).await?;

        Ok(NodeInfo {
            current_block_number: latest.header.number(),
            current_block_timestamp: latest.header.timestamp(),
            current_block_hash: latest.header.hash(),
            hard_fork: self.identity.hardfork.clone().unwrap_or_else(|| {
                self.hardfork_name(latest.header.timestamp(), latest.header.number())
            }),
            transaction_order: self.transaction_order.to_string(),
            environment: NodeEnvironment {
                // The base fee of the next block, as anvil reports it.
                base_fee: match self.block_env.next_base_fee() {
                    Some(base_fee) => base_fee,
                    None => self.next_base_fee()?.unwrap_or_default(),
                }
                .into(),
                chain_id: self.chain_spec.chain().id(),
                gas_limit: self.block_env.gas_limit().unwrap_or_else(|| latest.header.gas_limit()),
                gas_price: gas_price.to(),
            },
            fork_config: self.fork.as_ref().filter(|fork| !fork.url().is_empty()).map_or_else(
                NodeForkConfig::default,
                |fork| NodeForkConfig {
                    fork_url: Some(fork.url()),
                    fork_block_number: Some(fork.block_number()),
                    fork_retry_backoff: Some(fork.retry_backoff().as_millis()),
                },
            ),
            network: self.identity.network.map(str::to_string),
        })
    }

    async fn anvil_metadata(&self) -> RpcResult<Metadata> {
        let latest = self.latest_block().await?;

        Ok(Metadata {
            client_version: CLIENT_VERSION.to_string(),
            client_semver: Some(SEMVER_VERSION.to_string()),
            client_commit_sha: Some(COMMIT_SHA.to_string()),
            chain_id: self.chain_spec.chain().id(),
            instance_id: *self.instance_id.read(),
            latest_block_number: latest.header.number(),
            latest_block_hash: latest.header.hash(),
            forked_network: self.fork.as_ref().map(|fork| ForkedNetwork {
                chain_id: fork.source_chain_id(),
                fork_block_number: fork.block_number(),
                fork_block_hash: fork.block_hash(),
            }),
            snapshots: self.snapshots.metadata(),
        })
    }

    async fn anvil_set_block_timestamp_interval(&self, seconds: u64) -> RpcResult<()> {
        self.time.set_block_timestamp_interval(seconds);
        Ok(())
    }

    async fn anvil_remove_block_timestamp_interval(&self) -> RpcResult<bool> {
        Ok(self.time.remove_block_timestamp_interval())
    }

    async fn anvil_increase_time(&self, seconds: U256) -> RpcResult<i64> {
        let offset = self.time.increase_time(seconds.to::<u64>());
        Ok(offset.min(i64::MAX as i128) as i64)
    }

    async fn anvil_set_time(&self, timestamp: u64) -> RpcResult<u64> {
        // Accept millisecond timestamps for compatibility with anvil.
        let timestamp = if timestamp > 1_000_000_000_000 { timestamp / 1000 } else { timestamp };
        let now = self.time.current_call_timestamp();
        self.time.set_time(timestamp);
        Ok(timestamp.saturating_sub(now))
    }

    async fn anvil_set_next_block_timestamp(&self, seconds: u64) -> RpcResult<()> {
        self.time.set_next_block_timestamp(seconds).map_err(invalid_params)
    }

    async fn anvil_set_block_gas_limit(&self, gas_limit: U256) -> RpcResult<bool> {
        let gas_limit =
            gas_limit.try_into().map_err(|_| invalid_params("gas_limit exceeds u64::MAX"))?;
        self.block_env.set_gas_limit(gas_limit);
        Ok(true)
    }

    async fn anvil_set_coinbase(&self, address: Address) -> RpcResult<()> {
        self.block_env.set_coinbase(address);
        self.relauncher.update_config(|config| config.coinbase = Some(address));
        Ok(())
    }

    async fn anvil_set_next_block_base_fee_per_gas(&self, base_fee: U256) -> RpcResult<()> {
        let base_fee =
            base_fee.try_into().map_err(|_| invalid_params("base_fee exceeds u64::MAX"))?;
        self.block_env.set_next_base_fee(base_fee);
        Ok(())
    }

    async fn anvil_set_next_block_prev_randao(&self, prev_randao: B256) -> RpcResult<()> {
        self.block_env.set_next_prev_randao(prev_randao);
        Ok(())
    }

    async fn anvil_set_next_block_parent_beacon_block_root(&self, root: B256) -> RpcResult<()> {
        self.block_env.set_next_parent_beacon_block_root(root);
        Ok(())
    }

    async fn anvil_set_balance(&self, address: Address, balance: U256) -> RpcResult<()> {
        self.state.write().set_balance(address, balance);
        self.sync_pool_account(address)
    }

    async fn anvil_add_balance(&self, address: Address, balance: U256) -> RpcResult<()> {
        let current = self.latest_balance(address)?;
        self.state.write().set_balance(address, current.saturating_add(balance));
        Ok(())
    }

    async fn anvil_set_nonce(&self, address: Address, nonce: U256) -> RpcResult<()> {
        let nonce = nonce.try_into().map_err(|_| invalid_params("nonce exceeds u64::MAX"))?;
        self.state.write().set_nonce(address, nonce);
        self.sync_pool_account(address)
    }

    async fn anvil_set_code(&self, address: Address, code: Bytes) -> RpcResult<()> {
        self.state.write().set_code(address, Bytecode::new_raw(code));
        self.refresh_pool();
        Ok(())
    }

    async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        value: B256,
    ) -> RpcResult<bool> {
        self.state.write().set_storage_at(address, slot.into(), value.into());
        self.refresh_pool();
        Ok(true)
    }

    async fn anvil_impersonate_signature(
        &self,
        signature: Bytes,
        address: Address,
    ) -> RpcResult<()> {
        if signature.len() != 65 {
            return Err(invalid_params("signature must be 65 bytes"));
        }
        self.impersonation.add_signature_override(signature, address);
        Ok(())
    }

    async fn anvil_set_chain_id(&self, chain_id: u64) -> RpcResult<()> {
        // The historical states keep the blocks mined so far traceable, as on anvil.
        let state = self.serializable_state(true)?;
        self.relauncher
            .relaunch(|config| {
                config.set_chain_id(Some(chain_id));
                config.init_state = Some(state);
            })
            .await
            .map_err(internal_error)
    }

    async fn anvil_dump_state(&self, preserve_historical_states: Option<bool>) -> RpcResult<Bytes> {
        self.serializable_state(preserve_historical_states.unwrap_or(false))?
            .encode()
            .map_err(|error| internal_error(error.to_string()))
    }

    async fn anvil_load_state(&self, buf: Bytes) -> RpcResult<bool> {
        let mut state = SerializableState::decode(&buf)
            .map_err(|error| invalid_params(format!("invalid state dump: {error}")))?;
        // A dump with a block environment replaces the chain head, as anvil does: the node
        // relaunches on the dump, keeping the blocks of the current chain as history.
        if state.block.is_some() {
            let current = self.serializable_state(false)?;
            let number = state.head_number().unwrap_or_default();
            if state.blocks.is_empty() {
                // A dump without blocks continues from a checkpoint block on top of the chain's
                // block before the head, as anvil does.
                let parent = number
                    .checked_sub(1)
                    .and_then(|parent| current.block_hash(parent))
                    .unwrap_or_default();
                let timestamp = state.block_env().map(|block| block.timestamp.saturating_to());
                let forks = CheckpointForks {
                    london: self.chain_spec.is_london_active_at_block(number),
                    shanghai: timestamp
                        .is_some_and(|ts| self.chain_spec.is_shanghai_active_at_timestamp(ts)),
                    cancun: timestamp
                        .is_some_and(|ts| self.chain_spec.is_cancun_active_at_timestamp(ts)),
                    prague: timestamp
                        .is_some_and(|ts| self.chain_spec.is_prague_active_at_timestamp(ts)),
                };
                state.synthesize_head(parent, forks);
            } else if state.head_block().is_none() {
                return Err(internal_error(format!(
                    "Best hash not found for best number {number}"
                )));
            }
            // The chain's blocks stay as history; the dump's blocks win at equal heights.
            let mut blocks = current.blocks;
            blocks.extend(state.blocks);
            state.blocks = blocks;
            let mut transactions = current.transactions;
            transactions.extend(state.transactions);
            state.transactions = transactions;
            self.relauncher
                .relaunch(move |config| config.init_state = Some(state))
                .await
                .map_err(internal_error)?;
            return Ok(true);
        }
        let latest = self
            .provider
            .latest()
            .map_err(|error| internal_error(format!("failed to read state: {error}")))?;
        // Read the current nonces before taking the write lock: the state provider reads the
        // overlay under the same lock.
        let mut nonces = Vec::with_capacity(state.accounts.len());
        for (address, record) in &state.accounts {
            let current_nonce = latest
                .basic_account(address)
                .map_err(|error| internal_error(format!("failed to read account: {error}")))?
                .map(|account| account.nonce)
                .unwrap_or_default();
            nonces.push(record.nonce.max(current_nonce));
        }
        drop(latest);
        // An account-only dump keeps the chain, so the next blocks continue the chain's own
        // timeline, not the pending timestamp controls.
        let head = self.sealed_header(self.best_block_number()?)?;
        self.time.reset(head.timestamp());
        let mut writes = self.state.write();
        for ((address, record), nonce) in state.accounts.into_iter().zip(nonces) {
            writes.set_nonce(address, nonce);
            writes.set_balance(address, record.balance);
            if !record.code.is_empty() {
                writes.set_code(address, Bytecode::new_raw(record.code));
            }
            for (slot, value) in record.storage {
                writes.set_storage_at(address, slot, value.into());
            }
        }
        Ok(true)
    }

    async fn anvil_deal_tip20(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<()> {
        if self.try_set_tip20_balance(address, token_address, balance)? {
            Ok(())
        } else {
            Err(internal_error(format!("address {token_address} is not a deployed TIP-20 token")))
        }
    }

    async fn anvil_set_fee_token(&self, user: Address, token: Address) -> RpcResult<()> {
        self.set_fee_token(user, token)
    }

    async fn anvil_set_validator_fee_token(
        &self,
        validator: Address,
        token: Address,
    ) -> RpcResult<()> {
        self.set_validator_fee_token(validator, token)
    }

    async fn anvil_set_fee_amm_liquidity(
        &self,
        user_token: Address,
        validator_token: Address,
        amount: U256,
    ) -> RpcResult<()> {
        self.set_fee_amm_liquidity(user_token, validator_token, amount)
    }
}

#[async_trait]
impl<Pool, Provider, Eth, Spec> EvmApiServer for AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: TransactionPool + TransactionPoolExt + Send + Sync + 'static,
    Provider: BlockNumReader
        + HeaderProvider
        + TransactionsProvider
        + StateProviderFactory
        + StateDump
        + Send
        + Sync
        + 'static,
    Eth: FullEthApiServer<NetworkTypes: RpcTypes<TransactionRequest: Default + CallBatch>>,
    Spec: EthChainSpec + EthereumHardforks + Hardforks + Send + Sync + 'static,
{
    async fn evm_mine(&self, opts: Option<MineOptions>) -> RpcResult<String> {
        self.anvil_mine_detailed(opts).await?;
        Ok("0x0".to_string())
    }
}

impl<Pool, Provider, Eth, Spec> AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: TransactionPool,
    Provider: BlockNumReader + HeaderProvider + StateProviderFactory,
    Eth: FullEthApiServer<NetworkTypes: RpcTypes<TransactionRequest: CallBatch>>,
    Spec: EthChainSpec + EthereumHardforks,
{
    /// Makes a call or an estimate on Tempo run with the request's nonce, as anvil runs it, by
    /// overriding the sender's nonce, or its lane's nonce, with it. Reth runs calls with the
    /// state's nonce, and Tempo charges a new account's cost to a transaction with nonce zero.
    #[cfg_attr(not(feature = "tempo"), expect(clippy::missing_const_for_fn))]
    fn with_request_nonce(
        &self,
        request: &RpcTxReq<Eth::NetworkTypes>,
        state_overrides: Option<StateOverride>,
    ) -> Option<StateOverride> {
        #[cfg(feature = "tempo")]
        if self.is_tempo()
            && let Some(nonce) = request.as_ref().nonce
            && let Some(from) = request.as_ref().from
        {
            let mut overrides = state_overrides.unwrap_or_default();
            match request.nonce_lane(from) {
                NonceLane::Account => {
                    let account = overrides.entry(from).or_default();
                    account.nonce.get_or_insert(nonce);
                }
                // An expiring nonce has no lane state.
                NonceLane::Expiring => {}
                NonceLane::Storage(address, slot) => {
                    let account = overrides.entry(address).or_default();
                    account
                        .state_diff
                        .get_or_insert_default()
                        .entry(slot.into())
                        .or_insert(U256::from(nonce).into());
                }
            }
            return Some(overrides);
        }
        let _ = request;
        state_overrides
    }

    /// Fills a missing `from` with the first dev account, as anvil does, and marks a missing
    /// `to` as a contract creation.
    fn with_sender(
        &self,
        mut request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<RpcTxReq<Eth::NetworkTypes>> {
        self.ensure_network_supports(request.as_ref())?;
        if request.as_ref().from.is_none() {
            let accounts = EthApiServer::accounts(&self.eth)?;
            let from =
                accounts.first().copied().ok_or_else(|| invalid_params("No Signer available"))?;
            request.as_mut().from = Some(from);
        }
        Ok(with_recipient(request))
    }

    /// Estimates the gas of a call down to the exact limit; see `eth_estimateGas`.
    async fn estimate_gas_exact(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<U256> {
        let request = self.with_call_fees(request)?;
        let state_overrides = self.with_request_nonce(&request, state_overrides);
        // Anvil checks the value against the balance before it runs the call.
        if let Some(from) = request.as_ref().from
            && let Some(value) = request.as_ref().value
            && !value.is_zero()
        {
            let balance = self.balance_of(from, block, state_overrides.as_ref())?;
            if value > balance {
                return Err(insufficient_funds(value, balance));
            }
        }
        let state_overrides = fund_default_caller(request.as_ref(), state_overrides);
        let estimate = EthApiServer::estimate_gas(
            &self.eth,
            request.clone(),
            block,
            state_overrides.clone(),
            block_overrides.clone(),
        )
        .await?;
        let mut high = estimate.saturating_to::<u64>();
        if high <= MIN_TRANSACTION_GAS {
            return Ok(estimate);
        }
        // Reth stops once the failing and the passing limit are within the error ratio, so the
        // exact limit is above `low`. Probe the calls between them.
        let mut low = (high as f64 * (1.0 - ESTIMATE_GAS_ERROR_RATIO)) as u64;
        while low + 1 < high {
            let mid = low + (high - low) / 2;
            let mut probe = request.clone();
            probe.as_mut().gas = Some(mid);
            let passes = EthApiServer::call(
                &self.eth,
                probe,
                block,
                state_overrides.clone(),
                block_overrides.clone(),
            )
            .await
            .is_ok();
            if passes {
                high = mid;
            } else {
                low = mid;
            }
        }
        Ok(U256::from(high))
    }

    /// Prepares a request for `eth_sendTransaction`: the sender and the recipient as
    /// [`Self::with_sender`], a gas limit, and fees. A request without a gas limit gets the
    /// estimate, or, when the estimate fails because the call reverts, the largest limit a
    /// transaction may have, so the transaction is mined and reverts, as on anvil. A request
    /// without fees gets the ones reth fills, or the gas price before London.
    async fn prepare_send(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<RpcTxReq<Eth::NetworkTypes>> {
        let mut request = self.with_sender(request)?;
        if request.as_ref().gas.is_none() {
            let gas = match self.estimate_gas_exact(request.clone(), None, None, None).await {
                Ok(gas) => gas.saturating_to(),
                Err(_) => self.fallback_gas_limit()?,
            };
            request.as_mut().gas = Some(gas);
        }
        let has_blobs =
            request.as_ref().sidecar.is_some() || request.as_ref().blob_versioned_hashes.is_some();
        if has_blobs && request.as_ref().max_fee_per_blob_gas.is_none() {
            let blob_fee = EthApiServer::blob_base_fee(&self.eth).await?;
            request.as_mut().max_fee_per_blob_gas = Some(blob_fee.saturating_to::<u128>().max(1));
        }
        self.fill_fees(&mut request).await?;
        Ok(request)
    }

    /// Fills missing fees: twice the base fee plus the suggested tip, or the gas price before
    /// London, as anvil does.
    async fn fill_fees(&self, request: &mut RpcTxReq<Eth::NetworkTypes>) -> RpcResult<()> {
        if request.as_ref().gas_price.is_none() && request.as_ref().max_fee_per_gas.is_none() {
            match self.sealed_header(self.best_block_number()?)?.base_fee_per_gas() {
                Some(base_fee) => {
                    let tip = match request.as_ref().max_priority_fee_per_gas {
                        Some(tip) => tip,
                        None => EthApiServer::max_priority_fee_per_gas(&self.eth).await?.to(),
                    };
                    let tx = request.as_mut();
                    tx.max_priority_fee_per_gas = Some(tip);
                    tx.max_fee_per_gas = Some(u128::from(base_fee) * 2 + tip);
                }
                None => request.as_mut().gas_price = Some(self.gas_price().await?.to()),
            }
        }
        Ok(())
    }

    /// Returns the next nonce of the request's sender: the pending account nonce, zero for an
    /// expiring nonce, or the nonce stored for its Tempo nonce lane.
    async fn next_nonce(&self, request: &RpcTxReq<Eth::NetworkTypes>) -> RpcResult<u64> {
        let from = request.as_ref().from.unwrap_or_default();
        match request.nonce_lane(from) {
            NonceLane::Account => Ok(self.pending_nonce(from).await?.to()),
            NonceLane::Expiring => Ok(0),
            NonceLane::Storage(address, slot) => Ok(self
                .provider
                .latest()
                .and_then(|state| state.storage(address, slot.into()))
                .map_err(|error| internal_error(format!("failed to read state: {error}")))?
                .unwrap_or_default()
                .saturating_to()),
        }
    }

    /// Returns the gas price: the base fee plus the suggested tip, the base fee alone when the
    /// minimum priority fee is disabled, or the node's gas price before London, as anvil does.
    async fn gas_price(&self) -> RpcResult<U256> {
        let base_fee = self.sealed_header(self.best_block_number()?)?.base_fee_per_gas();
        match base_fee {
            // Before London, the node's gas price, as `anvil_setMinGasPrice` sets it.
            None if let Some(gas_price) = self.block_env.gas_price() => Ok(U256::from(gas_price)),
            Some(base_fee) if !self.min_priority_fee_enforced => Ok(U256::from(base_fee)),
            _ => EthApiServer::gas_price(&self.eth).await,
        }
    }

    /// Sends a transaction from a dev or impersonated account; see `eth_sendTransaction`. The
    /// sender's lock is held from the nonce selection to the pool insertion, so concurrent
    /// requests get distinct nonces and a replacement is checked against the pool it meets.
    async fn send(&self, request: RpcTxReq<Eth::NetworkTypes>) -> RpcResult<B256> {
        simulate::validate_request(request.as_ref())?;
        ensure_chain_id(request.as_ref(), self.chain_spec.chain().id())?;
        let request = self.with_sender(request)?;
        let _guard = self.nonce_lock(request.as_ref().from.unwrap_or_default()).lock_owned().await;
        let mut request = self.prepare_send(request).await?;
        if let Some(max_fee) = request.as_ref().max_fee_per_gas.or(request.as_ref().gas_price) {
            self.ensure_fee_cap(max_fee)?;
        }
        self.ensure_request_replacement_priced(&request)?;
        // Reth signs a blob transaction without its sidecar and the pool rejects it; the signed
        // transaction gets the sidecar back and goes in as a pooled transaction.
        let Some(sidecar) = request.as_ref().sidecar.clone() else {
            return EthApiServer::send_transaction(&self.eth, request)
                .await
                .map_err(|error| self.pool_error(error));
        };
        if request.as_ref().blob_versioned_hashes.is_none() {
            request.as_mut().blob_versioned_hashes = Some(sidecar.versioned_hashes().collect());
        }
        // Signing fills nothing in, unlike sending.
        if request.as_ref().nonce.is_none() {
            let from = request.as_ref().from.unwrap_or_default();
            request.as_mut().nonce = Some(self.pending_nonce(from).await?.saturating_to());
        }
        if request.as_ref().chain_id.is_none() {
            request.as_mut().chain_id = EthApiServer::chain_id(&self.eth).await?.map(|id| id.to());
        }
        let encoded = EthApiServer::sign_transaction(&self.eth, request).await?;
        let envelope = TxEnvelope::decode_2718(&mut encoded.as_ref()).map_err(|error| {
            internal_error(format!("failed to decode the signed transaction: {error}"))
        })?;
        let TxEnvelope::Eip4844(signed) = envelope else {
            return Err(invalid_params("a transaction with a sidecar must be a blob transaction"));
        };
        let pooled = PooledTransactionVariant::Eip4844(signed.map(|tx| {
            let tx = match tx {
                TxEip4844Variant::TxEip4844(tx) => tx,
                TxEip4844Variant::TxEip4844WithSidecar(tx) => tx.tx,
            };
            TxEip4844WithSidecar::from_tx_and_sidecar(tx, sidecar)
        }));
        EthApiServer::send_raw_transaction(&self.eth, pooled.encoded_2718().into()).await
    }

    /// Sends a signed transaction; see `eth_sendRawTransaction`. The sender's lock is held
    /// through the pool insertion, as in [`Self::send`]. A transaction reth cannot decode fails
    /// in reth's handler with reth's error.
    async fn send_raw(&self, tx: Bytes) -> RpcResult<B256> {
        // A Tempo transaction that asks for sponsorship gets the node's fee payer signature
        // first, as the sign-and-relay mode of Tempo's fee payer service does.
        let tx = if self.is_tempo() {
            let tx = self.sponsor_raw_transaction(&tx, false)?;
            self.ensure_tempo_valid_after(&tx)?;
            tx
        } else {
            tx
        };
        let recovered = recover_raw_transaction::<PooledTransactionVariant>(&tx).ok();
        let _guard = match &recovered {
            Some(recovered) => Some(self.nonce_lock(recovered.signer()).lock_owned().await),
            None => None,
        };
        if let Some(recovered) = &recovered {
            self.ensure_fee_cap(recovered.max_fee_per_gas())?;
            self.ensure_replacement_priced(
                recovered.signer(),
                recovered.nonce(),
                recovered.max_fee_per_gas(),
            )?;
        }
        EthApiServer::send_raw_transaction(&self.eth, tx)
            .await
            .map_err(|error| self.pool_error(error))
    }

    /// Returns whether a lookup by transaction hash that failed with `error` should run again:
    /// the lookup failed to recover the sender from the signature, and the transaction's block
    /// is now in the RPC cache. An impersonated transaction has no valid signature, and the
    /// cache carries the senders the block recorded.
    /// Returns whether the given endpoint is this node's own, by the instance id it reports.
    async fn is_own_endpoint(&self, url: &str) -> bool {
        let Ok(provider) = ProviderBuilder::<alloy_network::AnyNetwork>::new(url).build() else {
            return false;
        };
        let response = tokio::time::timeout(
            ENDPOINT_PROBE_TIMEOUT,
            provider.raw_request::<_, Metadata>("anvil_metadata".into(), ()),
        )
        .await;
        matches!(response, Ok(Ok(metadata)) if metadata.instance_id == *self.instance_id.read())
    }

    async fn cache_block_of(&self, hash: B256, error: &ErrorObjectOwned) -> RpcResult<bool>
    where
        Provider: TransactionsProvider,
    {
        if error.message() != INVALID_SIGNATURE_MESSAGE {
            return Ok(false);
        }
        let Some((_, meta)) = self
            .provider
            .transaction_by_hash_with_meta(hash)
            .map_err(|error| internal_error(format!("failed to read transaction: {error}")))?
        else {
            return Ok(false);
        };
        self.eth
            .cache()
            .get_recovered_block(meta.block_hash)
            .await
            .map_err(|error| internal_error(format!("failed to read block: {error}")))?;
        Ok(true)
    }

    /// Decides where a block access list request goes, as anvil does: a block above the head is
    /// an error, a fork's blocks at or below the fork block and unknown hashes come from the fork
    /// endpoint, a block before Amsterdam has no list, and the rest come from reth.
    fn access_list_route(&self, block: BlockId) -> RpcResult<AccessListRoute> {
        let best = self.best_block_number()?;
        let header = match block {
            BlockId::Number(BlockNumberOrTag::Number(number)) if number > best => {
                return Err(invalid_params(format!(
                    "BlockOutOfRangeError: block {number} is above the head {best}"
                )));
            }
            BlockId::Number(number) => {
                let number = self
                    .provider
                    .convert_block_number(number)
                    .map_err(|error| internal_error(format!("failed to resolve block: {error}")))?;
                number.map(|number| self.sealed_header(number)).transpose()?
            }
            BlockId::Hash(hash) => self
                .provider
                .sealed_header_by_hash(hash.block_hash)
                .map_err(|error| internal_error(format!("failed to read header: {error}")))?,
        };
        if let Some(fork) = &self.fork
            && header.as_ref().is_none_or(|header| header.number() <= fork.block_number())
        {
            return Ok(AccessListRoute::Forward);
        }
        // An unknown hash has no list; a number above the head failed above.
        let Some(header) = header else {
            return Ok(AccessListRoute::Null);
        };
        if !self.chain_spec.is_amsterdam_active_at_timestamp(header.timestamp()) {
            return Ok(AccessListRoute::Null);
        }
        Ok(AccessListRoute::Local)
    }

    /// Sends a request to the fork endpoint and returns its result.
    fn forward_json<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> RpcResult<T> {
        let fork = self.fork.as_ref().ok_or_else(|| internal_error("no fork"))?;
        fork.forward_json(method, params)
    }

    /// Returns the next nonce of an account: the one after its highest pooled transaction, or
    /// its nonce in the latest state.
    async fn pending_nonce(&self, address: Address) -> RpcResult<U256> {
        let latest =
            EthApiServer::transaction_count(&self.eth, address, Some(BlockId::latest())).await?;
        let pooled = self
            .pool
            .get_highest_transaction_by_sender(address)
            .map(|tx| U256::from(tx.nonce() + 1))
            .unwrap_or_default();
        Ok(latest.max(pooled))
    }

    /// Returns the nonce lock of a sender.
    fn nonce_lock(&self, sender: Address) -> Arc<AsyncMutex<()>> {
        self.nonce_locks.lock().entry(sender).or_default().clone()
    }

    /// Returns the balance of an account in a block, with the state override applied.
    fn balance_of(
        &self,
        address: Address,
        block: Option<BlockId>,
        state_overrides: Option<&StateOverride>,
    ) -> RpcResult<U256> {
        if let Some(AccountOverride { balance: Some(balance), .. }) =
            state_overrides.and_then(|overrides| overrides.get(&address))
        {
            return Ok(*balance);
        }
        let state = self
            .provider
            .state_by_block_id(block.unwrap_or_default())
            .map_err(|error| internal_error(format!("failed to read state: {error}")))?;
        let balance = state
            .account_balance(&address)
            .map_err(|error| internal_error(format!("failed to read account: {error}")))?;
        Ok(balance.unwrap_or_default())
    }

    /// Returns the base fee of the next block: the override, or the fee the latest block gives
    /// it. `None` before London.
    fn next_base_fee(&self) -> RpcResult<Option<u64>> {
        if let Some(fee) = self.block_env.building_base_fee().or(self.block_env.next_base_fee()) {
            return Ok(Some(fee));
        }
        let header = self.sealed_header(self.best_block_number()?)?;
        Ok((self.next_block_base_fee.0)(header.header(), self.time.current_call_timestamp()))
    }

    /// Rejects a transaction whose fee cap is below the next block's base fee, as anvil does;
    /// reth parks it in the pool until the base fee drops.
    fn ensure_fee_cap(&self, max_fee: u128) -> RpcResult<()> {
        if let Some(base_fee) = self.next_base_fee()?
            && max_fee < u128::from(base_fee)
        {
            return Err(
                EthApiError::InvalidTransaction(RpcInvalidTransactionError::FeeCapTooLow).into()
            );
        }
        Ok(())
    }

    /// Fails a priced call whose sender cannot pay for its gas and value, as anvil does; reth
    /// runs calls without the balance check. A call without a gas limit pays for at least a
    /// transfer.
    fn ensure_call_funds(
        &self,
        request: &RpcTxReq<Eth::NetworkTypes>,
        block: Option<BlockId>,
        state_overrides: Option<&StateOverride>,
    ) -> RpcResult<()> {
        let tx = request.as_ref();
        let (Some(from), Some(price)) = (tx.from, tx.gas_price.or(tx.max_fee_per_gas)) else {
            return Ok(());
        };
        if price == 0 {
            return Ok(());
        }
        let balance = self.balance_of(from, block, state_overrides)?;
        let gas = tx.gas.unwrap_or(MIN_TRANSACTION_GAS);
        let cost = U256::from(price) * U256::from(gas) + tx.value.unwrap_or_default();
        if balance < cost {
            return Err(insufficient_funds(cost, balance));
        }
        Ok(())
    }

    /// Rejects a transaction whose fee does not exceed the pooled transaction with the same
    /// sender and nonce, as anvil does; reth replaces at an equal fee.
    fn ensure_replacement_priced(
        &self,
        sender: Address,
        nonce: u64,
        max_fee: u128,
    ) -> RpcResult<()> {
        if let Some(existing) = self.pool.get_transaction_by_sender_and_nonce(sender, nonce)
            && max_fee <= existing.transaction.max_fee_per_gas()
        {
            return Err(EthApiError::PoolError(RpcPoolError::ReplaceUnderpriced).into());
        }
        Ok(())
    }

    /// [`Self::ensure_replacement_priced`] for a request with an explicit nonce and fee.
    fn ensure_request_replacement_priced(
        &self,
        request: &RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<()> {
        let tx = request.as_ref();
        if let (Some(from), Some(nonce), Some(max_fee)) =
            (tx.from, tx.nonce, tx.max_fee_per_gas.or(tx.gas_price))
        {
            self.ensure_replacement_priced(from, nonce, max_fee)?;
        }
        Ok(())
    }

    /// Returns the largest gas limit a transaction may have: the block gas limit, capped by the
    /// network's transaction gas cap as the EVM resolves it, such as EIP-7825 from Osaka on or
    /// Monad's own cap.
    fn fallback_gas_limit(&self) -> RpcResult<u64> {
        let header = self.sealed_header(self.best_block_number()?)?;
        let cap = self
            .eth
            .provider()
            .sealed_header(header.number())
            .ok()
            .flatten()
            .and_then(|header| self.eth.evm_config().evm_env(header.header()).ok())
            .map(|env| Cfg::tx_gas_limit_cap(&env.cfg_env))
            .unwrap_or(u64::MAX);
        Ok(header.gas_limit().min(cap))
    }

    /// Drops fee fields below the base fee from a call, so the call runs instead of failing
    /// the fee check; anvil runs calls with the base fee check off.
    fn with_call_fees(
        &self,
        mut request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<RpcTxReq<Eth::NetworkTypes>> {
        self.ensure_network_supports(request.as_ref())?;
        let Some(base_fee) = self.sealed_header(self.best_block_number()?)?.base_fee_per_gas()
        else {
            return Ok(request);
        };
        let fees = request.as_mut();
        let below = |fee: Option<u128>| fee.is_some_and(|fee| fee < u128::from(base_fee));
        if below(fees.gas_price) || below(fees.max_fee_per_gas) {
            fees.gas_price = None;
            fees.max_fee_per_gas = None;
            fees.max_priority_fee_per_gas = None;
        }
        // Anvil runs a blob call at a legacy gas price; reth rejects the mix, so the gas price
        // becomes the fee cap and the tip, which prices the call the same.
        if let Some(gas_price) = fees.gas_price
            && fees.max_fee_per_gas.is_none()
            && (fees.max_fee_per_blob_gas.is_some() || fees.blob_versioned_hashes.is_some())
        {
            fees.gas_price = None;
            fees.max_fee_per_gas = Some(gas_price);
            fees.max_priority_fee_per_gas = Some(gas_price);
        }
        Ok(request)
    }

    /// Waits for the receipt of a transaction. Anvil reports a timeout with code 4 and the
    /// transaction hash as data.
    async fn await_receipt(
        &self,
        hash: B256,
        timeout: Duration,
    ) -> RpcResult<RpcReceipt<Eth::NetworkTypes>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(receipt) = EthApiServer::transaction_receipt(&self.eth, hash).await? {
                return Ok(receipt);
            }
            if Instant::now() >= deadline {
                return Err(ErrorObjectOwned::owned(
                    TRANSACTION_CONFIRMATION_TIMEOUT_CODE,
                    "Transaction confirmation timeout",
                    Some(hash),
                ));
            }
            tokio::time::sleep(RECEIPT_POLL_INTERVAL).await;
        }
    }
}

#[async_trait]
impl<Pool, Provider, Eth, Spec>
    EthExtApiServer<
        RpcTxReq<Eth::NetworkTypes>,
        RpcReceipt<Eth::NetworkTypes>,
        RpcTransaction<Eth::NetworkTypes>,
        RpcBlock<Eth::NetworkTypes>,
        TxTy<Eth::Primitives>,
    > for AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: TransactionPool + 'static,
    Provider: BlockNumReader
        + HeaderProvider
        + StateProviderFactory
        + TransactionsProvider
        + Send
        + Sync
        + 'static,
    Eth: FullEthApiServer<NetworkTypes: RpcTypes<TransactionRequest: CallBatch>>,
    <Eth::Evm as ConfigureEvm>::NextBlockEnvCtx: AnvilNextBlockEnv,
    Spec: EthChainSpec + EthereumHardforks + Send + Sync + 'static,
{
    async fn eth_send_transaction(&self, request: RpcTxReq<Eth::NetworkTypes>) -> RpcResult<B256> {
        self.send(request).await
    }

    async fn eth_send_transaction_sync(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<RpcReceipt<Eth::NetworkTypes>> {
        let hash = self.send(request).await?;
        self.await_receipt(hash, TRANSACTION_CONFIRMATION_TIMEOUT).await
    }

    async fn eth_resend(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
        gas_price: Option<U256>,
        gas_limit: Option<U64>,
    ) -> RpcResult<B256> {
        let Some(nonce) = request.as_ref().nonce else {
            return Err(invalid_params("missing transaction nonce in transaction spec"));
        };
        let mut request = self.with_sender(request)?;
        let from = request.as_ref().from.unwrap_or_default();
        if self.pool.get_transaction_by_sender_and_nonce(from, nonce).is_none() {
            return Err(invalid_params("transaction not found"));
        }
        if let Some(gas_price) = gas_price {
            let gas_price =
                gas_price.try_into().map_err(|_| invalid_params("gas price exceeds u128"))?;
            let tx = request.as_mut();
            if tx.max_fee_per_gas.is_some() || tx.max_priority_fee_per_gas.is_some() {
                tx.max_fee_per_gas = Some(gas_price);
            } else {
                tx.gas_price = Some(gas_price);
            }
        }
        if let Some(gas_limit) = gas_limit {
            request.as_mut().gas = Some(gas_limit.to());
        }
        self.ensure_request_replacement_priced(&request)?;
        EthApiServer::send_transaction(&self.eth, request).await
    }

    async fn eth_send_raw_transaction(&self, tx: Bytes) -> RpcResult<B256> {
        self.send_raw(tx).await
    }

    async fn eth_send_raw_transaction_conditional(
        &self,
        tx: Bytes,
        _condition: TransactionConditional,
    ) -> RpcResult<B256> {
        self.send_raw(tx).await
    }

    async fn eth_request_accounts(&self) -> RpcResult<Vec<Address>> {
        EthApiServer::accounts(&self.eth)
    }

    async fn eth_network_id(&self) -> RpcResult<Option<String>> {
        Ok(EthApiServer::chain_id(&self.eth).await?.map(|id| id.to::<u64>().to_string()))
    }

    async fn eth_gas_price(&self) -> RpcResult<U256> {
        self.gas_price().await
    }

    async fn eth_estimate_gas(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<U256> {
        simulate::validate_request(request.as_ref())?;
        ensure_chain_id(request.as_ref(), self.chain_spec.chain().id())?;
        self.estimate_gas_exact(request, block, state_overrides, block_overrides)
            .await
            .map_err(with_revert_data)
    }

    async fn eth_call(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
        block: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<Bytes> {
        simulate::validate_request(request.as_ref())?;
        ensure_chain_id(request.as_ref(), self.chain_spec.chain().id())?;
        self.ensure_call_funds(&request, block, state_overrides.as_ref())?;
        let request = self.with_call_fees(request)?;
        let state_overrides = self.with_request_nonce(&request, state_overrides);
        EthApiServer::call(&self.eth, request, block, state_overrides, block_overrides)
            .await
            .map_err(with_revert_data)
    }

    async fn eth_call_many(
        &self,
        bundles: Vec<Bundle<RpcTxReq<Eth::NetworkTypes>>>,
        state_context: Option<StateContext>,
        state_override: Option<StateOverride>,
    ) -> RpcResult<Vec<Vec<EthCallResponse>>> {
        // Reth skips empty bundles and runs every bundle in the same block. Anvil answers every
        // bundle and moves each one a block and a second past the one before, from the block
        // the first bundle names or the pending block.
        let latest = self.sealed_header(self.best_block_number()?)?;
        let mut number = U256::from(latest.number() + 1);
        let mut time = latest.timestamp() + 1;
        let mut requests = Vec::with_capacity(bundles.len());
        let mut layout = Vec::with_capacity(bundles.len());
        for Bundle { transactions, block_override } in bundles {
            let mut block_override = block_override.unwrap_or_default();
            number = block_override.number.unwrap_or(number);
            time = block_override.time.unwrap_or(time);
            block_override.number = Some(number);
            block_override.time = Some(time);
            number += U256::ONE;
            time += 1;
            layout.push(transactions.is_empty());
            if !transactions.is_empty() {
                let transactions = transactions
                    .into_iter()
                    .map(|request| self.with_call_fees(request))
                    .collect::<RpcResult<Vec<_>>>()?;
                requests.push(Bundle { transactions, block_override: Some(block_override) });
            }
        }
        let mut results =
            EthApiServer::call_many(&self.eth, requests, state_context, state_override)
                .await?
                .into_iter();
        Ok(layout
            .into_iter()
            .map(|empty| if empty { Vec::new() } else { results.next().unwrap_or_default() })
            .collect())
    }

    async fn eth_base_fee(&self) -> RpcResult<Option<U256>> {
        Ok(self.next_base_fee()?.map(U256::from))
    }

    async fn eth_transaction_count(
        &self,
        address: Address,
        block: Option<BlockId>,
    ) -> RpcResult<U256> {
        if !block.is_some_and(|block| block.is_pending()) {
            return EthApiServer::transaction_count(&self.eth, address, block).await;
        }
        self.pending_nonce(address).await
    }

    async fn eth_simulate_v1(
        &self,
        mut payload: SimulatePayload<RpcTxReq<Eth::NetworkTypes>>,
        block: Option<BlockId>,
    ) -> RpcResult<Vec<SimulatedBlock<RpcBlock<Eth::NetworkTypes>>>> {
        for call in payload.block_state_calls.iter_mut().flat_map(|block| block.calls.iter_mut()) {
            let tx = call.as_mut();
            if let Some(sidecar) = tx.sidecar.take()
                && tx.blob_versioned_hashes.is_none()
            {
                tx.blob_versioned_hashes = Some(sidecar.versioned_hashes().collect());
            }
        }
        // Simulated blocks are spaced by the timestamp interval, else the mining interval
        // rounded up to whole seconds, else anvil's default.
        let interval = self.time.interval().unwrap_or_else(|| {
            self.mining
                .interval()
                .map(|duration| {
                    duration
                        .as_secs()
                        .saturating_add(u64::from(duration.subsec_nanos() != 0))
                        .max(1)
                })
                .unwrap_or(simulate::DEFAULT_BLOCK_INTERVAL_SECS)
        });
        let base_fee = self.block_env.building_base_fee().or(self.block_env.next_base_fee());
        simulate::simulate_v1(
            &self.eth,
            payload,
            block,
            interval,
            base_fee,
            self.fork.is_none(),
            self.fork.clone(),
        )
        .await
        .map_err(Into::into)
    }

    async fn eth_block_access_list(&self, block: BlockId) -> RpcResult<Option<serde_json::Value>> {
        match self.access_list_route(block)? {
            AccessListRoute::Null => Ok(None),
            AccessListRoute::Forward => {
                self.forward_json("eth_getBlockAccessList", serde_json::json!([block]))
            }
            AccessListRoute::Local => EthApiServer::block_access_list(&self.eth, block).await,
        }
    }

    async fn eth_block_access_list_raw(&self, block: BlockId) -> RpcResult<Option<Bytes>> {
        match self.access_list_route(block)? {
            AccessListRoute::Null => Ok(None),
            AccessListRoute::Forward => {
                self.forward_json("eth_getBlockAccessListRaw", serde_json::json!([block]))
            }
            AccessListRoute::Local => EthApiServer::block_access_list_raw(&self.eth, block).await,
        }
    }

    async fn eth_block_access_list_by_block_hash(
        &self,
        hash: B256,
    ) -> RpcResult<Option<serde_json::Value>> {
        match self.access_list_route(hash.into())? {
            AccessListRoute::Null => Ok(None),
            AccessListRoute::Forward => {
                self.forward_json("eth_getBlockAccessListByBlockHash", serde_json::json!([hash]))
            }
            AccessListRoute::Local => {
                EthApiServer::block_access_list_by_block_hash(&self.eth, hash).await
            }
        }
    }

    async fn eth_block_access_list_by_block_number(
        &self,
        number: BlockNumberOrTag,
    ) -> RpcResult<Option<serde_json::Value>> {
        match self.access_list_route(number.into())? {
            AccessListRoute::Null => Ok(None),
            AccessListRoute::Forward => self
                .forward_json("eth_getBlockAccessListByBlockNumber", serde_json::json!([number])),
            AccessListRoute::Local => {
                EthApiServer::block_access_list_by_block_number(&self.eth, number).await
            }
        }
    }

    async fn eth_raw_transaction_by_hash(&self, hash: B256) -> RpcResult<Option<Bytes>> {
        Ok(EthApiServer::raw_transaction_by_hash(&self.eth, hash).await?.map(without_sidecar))
    }

    async fn eth_new_filter(&self, filter: Filter) -> RpcResult<FilterId> {
        (self.new_filter.0)(Some(filter)).await
    }

    async fn eth_new_block_filter(&self) -> RpcResult<FilterId> {
        (self.new_filter.0)(None).await
    }

    async fn eth_block_uncles_count_by_hash(&self, hash: B256) -> RpcResult<Option<U256>> {
        let known = self
            .provider
            .block_number(hash)
            .map_err(|error| internal_error(format!("failed to read block: {error}")))?;
        if known.is_none() {
            return Err(EthApiError::HeaderNotFound(hash.into()).into());
        }
        EthApiServer::block_uncles_count_by_hash(&self.eth, hash).await
    }

    async fn eth_block_uncles_count_by_number(
        &self,
        number: BlockNumberOrTag,
    ) -> RpcResult<Option<U256>> {
        if let BlockNumberOrTag::Number(number) = number
            && number > self.best_block_number()?
        {
            return Err(EthApiError::HeaderNotFound(number.into()).into());
        }
        EthApiServer::block_uncles_count_by_number(&self.eth, number).await
    }

    async fn eth_sign_transaction(&self, request: RpcTxReq<Eth::NetworkTypes>) -> RpcResult<Bytes> {
        let mut request = self.with_sender(request)?;
        if request.as_ref().chain_id.is_none() {
            request.as_mut().chain_id = EthApiServer::chain_id(&self.eth).await?.map(|id| id.to());
        }
        if request.as_ref().nonce.is_none() {
            request.as_mut().nonce = Some(self.next_nonce(&request).await?);
        }
        if request.as_ref().gas.is_none() {
            // The estimate runs without the fee fields, which the signed transaction keeps as
            // given, so a tip above the fee cap does not fail it.
            let mut probe = request.clone();
            let fees = probe.as_mut();
            fees.gas_price = None;
            fees.max_fee_per_gas = None;
            fees.max_priority_fee_per_gas = None;
            let gas = match self.estimate_gas_exact(probe, None, None, None).await {
                Ok(gas) => gas.saturating_to(),
                Err(_) => self.fallback_gas_limit()?,
            };
            request.as_mut().gas = Some(gas);
        }
        // A Tempo transaction cannot be built without fees; reth signs what it is given.
        if self.is_tempo() {
            self.fill_fees(&mut request).await?;
        }
        EthApiServer::sign_transaction(&self.eth, request).await
    }

    async fn eth_get_balance(&self, address: Address, block: Option<BlockId>) -> RpcResult<U256> {
        if self.is_tempo() {
            return self.balance_of(address, block, None);
        }
        EthApiServer::balance(&self.eth, address, block).await
    }

    async fn eth_fill_transaction(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<FillTransaction<TxTy<Eth::Primitives>>> {
        self.ensure_network_supports(request.as_ref())?;
        EthApiServer::fill_transaction(&self.eth, request).await
    }

    async fn eth_sign_raw_transaction(&self, tx: Bytes) -> RpcResult<Bytes> {
        if !self.is_tempo() {
            return Err(tempo_only());
        }
        if tx.is_empty() {
            return Err(invalid_params("empty transaction data"));
        }
        self.sponsor_raw_transaction(&tx, true)
    }

    async fn eth_coinbase(&self) -> RpcResult<Address> {
        Ok(self.block_env.coinbase().unwrap_or(self.chain_spec.genesis().coinbase))
    }

    async fn eth_sign_typed_data_v4(&self, address: Address, data: TypedData) -> RpcResult<Bytes> {
        EthApiServer::sign_typed_data(&self.eth, address, data).await
    }

    async fn eth_get_transaction_receipt(
        &self,
        hash: B256,
    ) -> RpcResult<Option<RpcReceipt<Eth::NetworkTypes>>> {
        match EthApiServer::transaction_receipt(&self.eth, hash).await {
            Err(error) if self.cache_block_of(hash, &error).await? => {
                EthApiServer::transaction_receipt(&self.eth, hash).await
            }
            result => result,
        }
    }

    async fn eth_get_transaction_by_hash(
        &self,
        hash: B256,
    ) -> RpcResult<Option<RpcTransaction<Eth::NetworkTypes>>> {
        match EthApiServer::transaction_by_hash(&self.eth, hash).await {
            Err(error) if self.cache_block_of(hash, &error).await? => {
                EthApiServer::transaction_by_hash(&self.eth, hash).await
            }
            result => result,
        }
    }

    async fn eth_create_access_list(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
        block: Option<BlockId>,
        state_override: Option<StateOverride>,
    ) -> RpcResult<AccessListResult> {
        let state_override = self.with_request_nonce(&request, state_override);
        #[cfg_attr(not(feature = "monad"), allow(unused_mut))]
        let mut result = EthApiServer::create_access_list(
            &self.eth,
            request.clone(),
            block,
            state_override.clone(),
        )
        .await?;
        #[cfg(feature = "monad")]
        if self.identity.network == Some("monad")
            && let Some(hardfork) = self.identity.hardfork.as_deref()
            && let Ok(hardfork) = hardfork.parse::<foundry_evm_hardforks::MonadHardfork>()
            && foundry_evm_hardforks::MonadHardfork::MonadTen.is_enabled_in(hardfork)
        {
            for item in &mut result.access_list.0 {
                item.storage_keys.sort_unstable();
                item.storage_keys.dedup_by_key(|slot| {
                    monad_revm::page::page_index(U256::from_be_slice(slot.as_slice()))
                });
            }
            // The gas follows the list as Monad charges it, as anvil re-executes with it.
            let mut request = request;
            request.as_mut().access_list = Some(result.access_list.clone());
            let executed = <Eth as reth_rpc_eth_api::helpers::Call>::transact_call_at(
                &self.eth,
                request,
                block.unwrap_or_else(BlockId::pending),
                alloy_rpc_types_eth::state::EvmOverrides::new(state_override, None),
            )
            .await
            .map_err(Into::into)?;
            result.gas_used = U256::from(executed.result.tx_gas_used());
        }
        Ok(result)
    }

    async fn eth_fee_history(
        &self,
        block_count: U64,
        newest_block: BlockNumberOrTag,
        reward_percentiles: Option<Vec<f64>>,
    ) -> RpcResult<FeeHistory> {
        let mut history =
            EthApiServer::fee_history(&self.eth, block_count, newest_block, reward_percentiles)
                .await?;
        let blocks = history.base_fee_per_gas.len().saturating_sub(1) as u64;
        if blocks == 0 {
            return Ok(history);
        }
        let newest = history.oldest_block + blocks - 1;
        let child = self
            .provider
            .sealed_header(newest + 1)
            .map_err(|error| internal_error(format!("failed to read header: {error}")))?;
        let (next_base_fee, next_blob_fee) = match child {
            Some(child) => {
                let blob_params = self.chain_spec.blob_params_at_timestamp(child.timestamp());
                (
                    child.base_fee_per_gas().map(u128::from),
                    blob_params.and_then(|params| child.blob_fee(params)),
                )
            }
            None => (self.next_base_fee()?.map(u128::from), None),
        };
        if let Some(fee) = next_base_fee
            && let Some(last) = history.base_fee_per_gas.last_mut()
        {
            *last = fee;
        }
        if let Some(fee) = next_blob_fee
            && let Some(last) = history.base_fee_per_blob_gas.last_mut()
        {
            *last = fee;
        }
        for ratio in history.gas_used_ratio.iter_mut().chain(history.blob_gas_used_ratio.iter_mut())
        {
            if ratio.is_nan() {
                *ratio = 0.0;
            }
        }
        Ok(history)
    }

    async fn eth_send_raw_transaction_sync(
        &self,
        tx: Bytes,
        timeout_ms: Option<u64>,
    ) -> RpcResult<RpcReceipt<Eth::NetworkTypes>> {
        let hash = self.send_raw(tx).await?;
        let timeout = timeout_ms.map_or(TRANSACTION_CONFIRMATION_TIMEOUT, Duration::from_millis);
        self.await_receipt(hash, timeout).await
    }

    async fn eth_send_unsigned_transaction(
        &self,
        request: RpcTxReq<Eth::NetworkTypes>,
    ) -> RpcResult<B256> {
        let from = request.as_ref().from.ok_or_else(|| invalid_params("No Signer available"))?;
        let impersonated = self.impersonation.is_impersonated(&from);
        if !impersonated {
            self.impersonation.impersonate(from);
        }
        let result = EthApiServer::send_transaction(&self.eth, with_recipient(request)).await;
        if !impersonated {
            self.impersonation.stop_impersonating(from);
        }
        result
    }
}

#[async_trait]
impl<Pool, Provider, Eth, Spec> PersonalApiServer for AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: Send + Sync + 'static,
    Provider: HeaderProvider + Send + Sync + 'static,
    Eth: FullEthApiServer,
    Spec: Send + Sync + 'static,
{
    async fn personal_sign(&self, message: Bytes, address: Address) -> RpcResult<Bytes> {
        EthApiServer::sign(&self.eth, address, message).await
    }
}

fn internal_error(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, message.into(), None::<()>)
}

fn invalid_params(message: impl Into<String>) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INVALID_PARAMS_CODE, message.into(), None::<()>)
}

/// Rejects a request for another chain, as anvil does. The EVM skips the check on replays, so
/// blocks mined before `anvil_setChainId` stay traceable.
fn ensure_chain_id(request: &TransactionRequest, chain_id: u64) -> RpcResult<()> {
    if request.chain_id.is_some_and(|id| id != chain_id) {
        return Err(ErrorObjectOwned::owned(
            TRANSACTION_REJECTED_CODE,
            "invalid chain id for signer",
            None::<()>,
        ));
    }
    Ok(())
}

#[async_trait]
impl<Pool, Provider, Eth, Spec> Web3ExtApiServer for AnvilRpc<Pool, Provider, Eth, Spec>
where
    Pool: Send + Sync + 'static,
    Provider: HeaderProvider + Send + Sync + 'static,
    Eth: Send + Sync + 'static,
    Spec: Send + Sync + 'static,
{
    async fn web3_client_version(&self) -> RpcResult<String> {
        Ok(CLIENT_VERSION.to_string())
    }
}

impl<Pool, Provider, Eth, Spec> AnvilRpc<Pool, Provider, Eth, Spec>
where
    Provider: BlockNumReader + HeaderProvider + StateProviderFactory,
    Spec: EthChainSpec,
{
    /// Sets the balance of an account in a TIP-20 token, and returns whether the token is a
    /// TIP-20 token. Tempo only.
    fn try_set_tip20_balance(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> RpcResult<bool> {
        #[cfg(feature = "tempo")]
        {
            self.with_tempo_storage(|| {
                if !TIP20Factory::new().is_tip20(token_address)? {
                    return Ok(false);
                }
                TIP20Token::from_address(token_address)?.balances[address].write(balance)?;
                Ok(true)
            })
        }
        #[cfg(not(feature = "tempo"))]
        {
            let _ = (address, token_address, balance);
            Err(tempo_only())
        }
    }

    /// Sets the token an account pays fees with. Tempo only.
    fn set_fee_token(&self, user: Address, token: Address) -> RpcResult<()> {
        #[cfg(feature = "tempo")]
        {
            self.with_tempo_storage(|| {
                TipFeeManager::new().set_user_token(user, IFeeManager::setUserTokenCall { token })
            })
        }
        #[cfg(not(feature = "tempo"))]
        {
            let _ = (user, token);
            Err(tempo_only())
        }
    }

    /// Sets the token a validator receives fees in. Tempo only.
    fn set_validator_fee_token(&self, validator: Address, token: Address) -> RpcResult<()> {
        #[cfg(feature = "tempo")]
        {
            // The zero beneficiary passes the check that the validator is not the beneficiary.
            self.with_tempo_storage(|| {
                TipFeeManager::new().set_validator_token(
                    validator,
                    IFeeManager::setValidatorTokenCall { token },
                    Address::ZERO,
                )
            })
        }
        #[cfg(not(feature = "tempo"))]
        {
            let _ = (validator, token);
            Err(tempo_only())
        }
    }

    /// Mints both tokens to a helper account and adds them as Fee AMM liquidity for the pair.
    /// Tempo only.
    fn set_fee_amm_liquidity(
        &self,
        user_token: Address,
        validator_token: Address,
        amount: U256,
    ) -> RpcResult<()> {
        #[cfg(feature = "tempo")]
        {
            // From T3 on, liquidity cannot go to the zero address.
            let admin = Address::repeat_byte(0x11);
            self.with_tempo_storage(|| {
                for token in [user_token, validator_token] {
                    let mut token = TIP20Token::from_address(token)?;
                    token.grant_role_internal(admin, TIP20Token::issuer_role())?;
                    token.mint(admin, ITIP20::mintCall { to: admin, amount })?;
                }
                TipFeeManager::new().mint(admin, user_token, validator_token, amount, admin)?;
                Ok(())
            })
        }
        #[cfg(not(feature = "tempo"))]
        {
            let _ = (user_token, validator_token, amount);
            Err(tempo_only())
        }
    }

    /// Runs Tempo precompile logic over the latest state, and applies its storage and code
    /// writes as anvil state writes for the next block.
    #[cfg(feature = "tempo")]
    fn with_tempo_storage<R>(
        &self,
        f: impl FnOnce() -> tempo_precompiles::error::Result<R>,
    ) -> RpcResult<R> {
        self.run_tempo_storage(f, true)
    }

    /// Runs Tempo precompile logic over the latest state, and applies its writes when `apply`
    /// is set.
    #[cfg(feature = "tempo")]
    fn run_tempo_storage<R>(
        &self,
        f: impl FnOnce() -> tempo_precompiles::error::Result<R>,
        apply: bool,
    ) -> RpcResult<R> {
        if !self.is_tempo() {
            return Err(tempo_only());
        }
        let hardfork = self
            .identity
            .hardfork
            .as_deref()
            .and_then(|hardfork| hardfork.parse().ok())
            .ok_or_else(|| internal_error("unknown Tempo hardfork"))?;
        let base = self.provider.latest().map_err(|error| internal_error(error.to_string()))?;
        let mut storage = TempoStorage::new(
            Some(&*base),
            self.chain_spec.chain_id(),
            self.best_block_number()? + 1,
            self.time.current_call_timestamp(),
            hardfork,
        );
        let result = StorageCtx::enter(&mut storage, f)
            .map_err(|error| internal_error(error.to_string()))?;
        if !apply {
            return Ok(result);
        }
        {
            let mut state = self.state.write();
            for (address, writes) in storage.into_writes() {
                if let Some(code) = writes.code {
                    state.set_code(address, reth_ethereum::primitives::Bytecode(code));
                }
                for (slot, value) in writes.storage {
                    state.set_storage_at(address, slot.into(), value);
                }
            }
        }
        self.refresh_pool();
        Ok(result)
    }
}

impl<Pool, Provider, Eth, Spec> AnvilRpc<Pool, Provider, Eth, Spec>
where
    Provider: BlockNumReader + HeaderProvider + StateProviderFactory,
    Spec: EthChainSpec,
{
    /// Fee-payer signs a raw Tempo transaction that carries the sponsorship placeholder, as
    /// Tempo's fee payer service does: the node's fee payer picks the fee token when the sender
    /// left it open, and signs. With `sign_only`, the transaction must ask for sponsorship;
    /// otherwise a transaction that does not ask for it comes back as is.
    fn sponsor_raw_transaction(&self, raw: &Bytes, sign_only: bool) -> RpcResult<Bytes> {
        #[cfg(feature = "tempo")]
        {
            // Fee payer service clients send the transaction with a `0x00` placeholder in the
            // fee payer signature field.
            let normalized = normalize_fee_payer_service_encoding(raw);
            let mut data = normalized.as_deref().unwrap_or(raw);
            let transaction = match TempoTxEnvelope::decode_2718(&mut data) {
                Ok(TempoTxEnvelope::AA(transaction)) => transaction,
                Ok(_) if sign_only => {
                    return Err(invalid_params(
                        "only Tempo (0x76) transactions can be fee-payer signed",
                    ));
                }
                Err(_) if sign_only => {
                    return Err(invalid_params("failed to decode signed transaction"));
                }
                _ => return Ok(raw.clone()),
            };
            match transaction.tx().fee_payer_signature {
                Some(FEE_PAYER_SIGNATURE_MARKER) => {}
                _ if !sign_only => return Ok(raw.clone()),
                Some(_) => {
                    return Err(invalid_params("transaction is already fee-payer signed"));
                }
                None => {
                    return Err(invalid_params(
                        "transaction does not request sponsorship; sign it with the fee payer \
                         signature placeholder",
                    ));
                }
            }
            let sender = transaction.recover_signer().map_err(|_| {
                invalid_params("transaction must be signed by the sender before fee-payer signing")
            })?;
            let Some(signer) = &self.tempo_fee_payer else {
                return Err(invalid_params("no Tempo fee payer account available"));
            };
            let sponsor = signer.address();
            if sponsor == sender {
                return Err(invalid_params(format!(
                    "Tempo fee payer {sponsor} must not equal the transaction sender"
                )));
            }
            let (mut tx, sender_signature, _) = transaction.into_parts();
            // The fee payer signature commits to the fee token, so the token comes first.
            if tx.fee_token.is_none() {
                let token = self
                    .run_tempo_storage(
                        || {
                            TipFeeManager::new()
                                .user_tokens(IFeeManager::userTokensCall { user: sponsor })
                        },
                        false,
                    )
                    .unwrap_or_default();
                tx.fee_token = Some(if token.is_zero() { PATH_USD_ADDRESS } else { token });
            }
            let digest = tx.fee_payer_signature_hash(sender);
            tx.fee_payer_signature = Some(
                signer
                    .sign_hash_sync(&digest)
                    .map_err(|error| internal_error(error.to_string()))?,
            );
            Ok(TempoTxEnvelope::AA(tx.into_signed(sender_signature)).encoded_2718().into())
        }
        #[cfg(not(feature = "tempo"))]
        {
            if sign_only { Err(tempo_only()) } else { Ok(raw.clone()) }
        }
    }
}

impl<Pool, Provider, Eth, Spec> AnvilRpc<Pool, Provider, Eth, Spec>
where
    Provider: HeaderProvider,
{
    /// Returns whether the node runs the Tempo network.
    fn is_tempo(&self) -> bool {
        self.identity.network == Some("tempo")
    }

    /// Makes the pool see the anvil state writes made since the tip.
    fn refresh_pool(&self) {
        if let Some(refresh) = &self.pool_refresh {
            (refresh.0)();
        }
    }

    /// Rejects a Tempo transaction request on a node that does not run Tempo, as anvil does,
    /// instead of running it as an Ethereum transaction.
    fn ensure_network_supports(&self, request: &TransactionRequest) -> RpcResult<()> {
        if !self.is_tempo() && request.transaction_type == Some(TEMPO_TRANSACTION_TYPE) {
            return Err(invalid_params(
                "tempo transaction received but is not supported.\n\nYou can use it by running \
                 anvil with '--tempo'.",
            ));
        }
        Ok(())
    }

    /// Reports a pool rejection as anvil does: on Tempo, a fee payer short of fee tokens gets the
    /// fee token shortfall, which reth reports as missing native funds.
    fn pool_error(&self, error: ErrorObjectOwned) -> ErrorObjectOwned {
        const INSUFFICIENT_FUNDS: &str = "insufficient funds for gas * price + value: have ";
        if self.is_tempo()
            && let Some(amounts) = error.message().strip_prefix(INSUFFICIENT_FUNDS)
            && let Some((balance, required)) = amounts.split_once(" want ")
        {
            return ErrorObjectOwned::owned(
                error.code(),
                format!("insufficient fee token balance: have {balance}, need {required}"),
                None::<()>,
            );
        }
        error
    }

    /// Rejects a Tempo transaction whose `valid_after` lies more than Tempo's pool limit past the
    /// time of the next block, as anvil's clock gives it.
    #[cfg_attr(not(feature = "tempo"), expect(clippy::missing_const_for_fn))]
    fn ensure_tempo_valid_after(&self, raw: &Bytes) -> RpcResult<()> {
        #[cfg(feature = "tempo")]
        if let Ok(TempoTxEnvelope::AA(transaction)) =
            TempoTxEnvelope::decode_2718(&mut raw.as_ref())
        {
            let max_allowed =
                self.time.current_call_timestamp().saturating_add(DEFAULT_AA_VALID_AFTER_MAX_SECS);
            transaction.tx().ensure_valid_after(max_allowed).map_err(|error| {
                ErrorObjectOwned::owned(TRANSACTION_REJECTED_CODE, error.to_string(), None::<()>)
            })?;
        }
        #[cfg(not(feature = "tempo"))]
        let _ = raw;
        Ok(())
    }
}

/// Returns the standard encoding of a Tempo transaction sent in the fee payer service encoding,
/// which carries a `0x00` placeholder for the fee payer signature, or `None` for any other
/// transaction.
#[cfg(feature = "tempo")]
fn normalize_fee_payer_service_encoding(raw: &[u8]) -> Option<Vec<u8>> {
    let (tx_type, mut encoded_fields) = raw.split_first()?;
    if *tx_type != TEMPO_TX_TYPE_ID {
        return None;
    }
    let PayloadView::List(fields) = RlpHeader::decode_raw(&mut encoded_fields).ok()? else {
        return None;
    };
    if !encoded_fields.is_empty() {
        return None;
    }
    // The fee payer signature is the twelfth field of a Tempo transaction.
    if fields.get(11).is_none_or(|field| *field != [0x00]) {
        return None;
    }

    // The standard encoding of the placeholder signature, as Tempo encodes it.
    let marker = FEE_PAYER_SIGNATURE_MARKER;
    let mut marker_field = Vec::new();
    RlpHeader { list: true, payload_length: marker.rlp_rs_len() + marker.v().length() }
        .encode(&mut marker_field);
    marker.write_rlp_vrs(&mut marker_field, marker.v());

    let mut payload = Vec::new();
    for (index, field) in fields.into_iter().enumerate() {
        if index == 11 {
            payload.extend_from_slice(&marker_field);
        } else {
            payload.extend_from_slice(field);
        }
    }
    let mut normalized = vec![TEMPO_TX_TYPE_ID];
    RlpHeader { list: true, payload_length: payload.len() }.encode(&mut normalized);
    normalized.extend_from_slice(&payload);
    Some(normalized)
}

/// The type of a Tempo transaction.
const TEMPO_TRANSACTION_TYPE: u8 = 0x76;

/// The error for a Tempo method on a node that does not run Tempo, as anvil reports it.
fn tempo_only() -> ErrorObjectOwned {
    internal_error("Not implemented")
}
