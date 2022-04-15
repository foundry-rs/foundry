use crate::{
    eth::{
        backend,
        error::{BlockchainError, FeeHistoryError, Result, ToRpcResponseResult},
        fees::{FeeDetails, FeeHistory, FeeHistoryCache},
        pool::{
            transactions::{to_marker, PoolTransaction},
            Pool,
        },
        sign::Signer,
    },
    revm::TransactOut,
    Provider,
};
use anvil_core::{
    eth::{
        call::CallRequest,
        filter::Filter,
        transaction::{
            EthTransactionRequest, LegacyTransaction, PendingTransaction, TypedTransaction,
            TypedTransactionRequest,
        },
        EthRequest,
    },
    types::{EvmMineOptions, Forking, Index, Work},
};
use anvil_rpc::response::ResponseResult;
use ethers::{
    abi::ethereum_types::H64,
    providers::ProviderError,
    types::{
        Address, Block, BlockNumber, Bytes, Log, Trace, Transaction, TransactionReceipt, TxHash,
        H256, U256, U64,
    },
    utils::rlp,
};
use std::sync::Arc;
use tracing::trace;

/// The entry point for executing eth api RPC call - The Eth RPC interface.
///
/// This type is cheap to clone and can be used concurrently
#[derive(Clone)]
pub struct EthApi {
    /// The transaction pool
    pool: Arc<Pool>,
    /// Holds all blockchain related data
    /// In-Memory only for now
    backend: Arc<backend::mem::Backend>,
    /// Whether this node is mining
    is_mining: bool,
    /// available signers
    signers: Arc<Vec<Box<dyn Signer>>>,
    /// data required for `eth_feeHistory`
    fee_history_cache: FeeHistoryCache,
}

// === impl Eth RPC API ===

impl EthApi {
    /// Creates a new instance
    pub fn new(
        pool: Arc<Pool>,
        backend: Arc<backend::mem::Backend>,
        signers: Arc<Vec<Box<dyn Signer>>>,
        fee_history_cache: FeeHistoryCache,
    ) -> Self {
        Self { pool, backend, is_mining: true, signers, fee_history_cache }
    }

    /// Executes the [EthRequest] and returns an RPC [RpcResponse]
    pub async fn execute(&self, request: EthRequest) -> ResponseResult {
        match request {
            EthRequest::EthGetBalance(addr, block) => self.balance(addr, block).to_rpc_result(),
            EthRequest::EthGetTransactionByHash(hash) => {
                self.transaction_by_hash(hash).await.to_rpc_result()
            }
            EthRequest::EthSendTransaction(request) => {
                self.send_transaction(*request).to_rpc_result()
            }
            EthRequest::EthChainId => self.chain_id().to_rpc_result(),
            EthRequest::EthGasPrice => self.gas_price().to_rpc_result(),
            EthRequest::EthAccounts => self.accounts().to_rpc_result(),
            EthRequest::EthBlockNumber => self.block_number().to_rpc_result(),
            EthRequest::EthGetStorageAt(addr, slot, block) => {
                self.storage_at(addr, slot, block).await.to_rpc_result()
            }
            EthRequest::EthGetBlockByHash(hash, full) => {
                self.block_by_hash(hash, full).await.to_rpc_result()
            }
            EthRequest::EthGetBlockByNumber(num, full) => {
                self.block_by_number(num, full).await.to_rpc_result()
            }
            EthRequest::EthGetTransactionCount(addr, block) => {
                self.transaction_count(addr, block).to_rpc_result()
            }
            EthRequest::EthGetTransactionCountByHash(hash) => {
                self.block_transaction_count_by_hash(hash).to_rpc_result()
            }
            EthRequest::EthGetTransactionCountByNumber(num) => {
                self.block_transaction_count_by_number(num).to_rpc_result()
            }
            EthRequest::EthGetUnclesCountByHash(hash) => {
                self.block_uncles_count_by_hash(hash).to_rpc_result()
            }
            EthRequest::EthGetUnclesCountByNumber(num) => {
                self.block_uncles_count_by_number(num).to_rpc_result()
            }
            EthRequest::EthGetCodeAt(addr, block) => {
                self.get_code(addr, block).await.to_rpc_result()
            }
            EthRequest::EthSendRawTransaction(tx) => self.send_raw_transaction(tx).to_rpc_result(),
            EthRequest::EthCall(call, block) => self.call(call, block).to_rpc_result(),
            EthRequest::EthEstimateGas(call, block) => {
                self.estimate_gas(call, block).to_rpc_result()
            }
            EthRequest::EthGetTransactionByBlockHashAndIndex(hash, index) => {
                self.transaction_by_block_hash_and_index(hash, index).await.to_rpc_result()
            }
            EthRequest::EthGetTransactionByBlockNumberAndIndex(num, index) => {
                self.transaction_by_block_number_and_index(num, index).to_rpc_result()
            }
            EthRequest::EthGetTransactionReceipt(tx) => {
                self.transaction_receipt(tx).await.to_rpc_result()
            }
            EthRequest::EthGetUncleByBlockHashAndIndex(hash, index) => {
                self.uncle_by_block_hash_and_index(hash, index).to_rpc_result()
            }
            EthRequest::EthGetUncleByBlockNumberAndIndex(num, index) => {
                self.uncle_by_block_number_and_index(num, index).to_rpc_result()
            }
            EthRequest::EthGetLogs(filter) => self.logs(filter).await.to_rpc_result(),
            EthRequest::EthGetWork => self.work().to_rpc_result(),
            EthRequest::EthSubmitWork(nonce, pow, digest) => {
                self.submit_work(nonce, pow, digest).to_rpc_result()
            }
            EthRequest::EthSubmitHashRate(rate, id) => {
                self.submit_hashrate(rate, id).to_rpc_result()
            }
            EthRequest::EthFeeHistory(count, newest, reward_percentiles) => {
                self.fee_history(count, newest, reward_percentiles).to_rpc_result()
            }

            // non eth-standard rpc calls
            EthRequest::DebugTraceTransaction(tx) => {
                self.debug_trace_transaction(tx).await.to_rpc_result()
            }
            EthRequest::ImpersonateAccount(addr) => {
                self.anvil_impersonate_account(addr).await.to_rpc_result()
            }
            EthRequest::StopImpersonatingAccount => {
                self.anvil_stop_impersonating_account().await.to_rpc_result()
            }
            EthRequest::GetAutoMine => self.anvil_get_auto_mine().await.to_rpc_result(),
            EthRequest::Mine(blocks, interval) => {
                self.anvil_mine(blocks, interval).await.to_rpc_result()
            }
            EthRequest::SetAutomine(enabled) => {
                self.anvil_set_auto_mine(enabled).await.to_rpc_result()
            }
            EthRequest::SetIntervalMining(interval) => {
                self.anvil_set_interval_mining(interval).await.to_rpc_result()
            }
            EthRequest::DropTransaction(tx) => {
                self.anvil_drop_transaction(tx).await.to_rpc_result()
            }
            EthRequest::Reset(res) => self.anvil_reset(res).await.to_rpc_result(),
            EthRequest::SetBalance(addr, val) => {
                self.anvil_set_balance(addr, val).await.to_rpc_result()
            }
            EthRequest::SetCode(addr, code) => {
                self.anvil_set_code(addr, code).await.to_rpc_result()
            }
            EthRequest::SetNonce(addr, nonce) => {
                self.anvil_set_nonce(addr, nonce).await.to_rpc_result()
            }
            EthRequest::SetStorageAt(addr, slot, val) => {
                self.anvil_set_storage_at(addr, slot, val).await.to_rpc_result()
            }
            EthRequest::SetCoinbase(addr) => self.anvil_set_coinbase(addr).await.to_rpc_result(),
            EthRequest::SetLogging(log) => self.anvil_set_logging(log).await.to_rpc_result(),
            EthRequest::SetMinGasPrice(gas) => {
                self.anvil_set_min_gas_price(gas).await.to_rpc_result()
            }
            EthRequest::SetNextBlockBaseFeePerGas(gas) => {
                self.anvil_set_next_block_base_fee_per_gas(gas).await.to_rpc_result()
            }
            EthRequest::EvmSnapshot => self.evm_snapshot().await.to_rpc_result(),
            EthRequest::EvmRevert(id) => self.evm_revert(id).await.to_rpc_result(),
            EthRequest::EvmIncreaseTime(time) => self.evm_increase_time(time).await.to_rpc_result(),
            EthRequest::EvmSetNextBlockTimeStamp(time) => {
                self.evm_set_next_block_timestamp(time).await.to_rpc_result()
            }
            EthRequest::EvmMine(mine) => self.evm_mine(mine).await.to_rpc_result(),
            EthRequest::SetRpcUrl(url) => self.anvil_set_rpc_url(url).to_rpc_result(),
            EthRequest::EthSendUnsignedTransaction(tx) => {
                self.eth_send_unsigned_transaction(*tx).await.to_rpc_result()
            }
            EthRequest::EnableTrances => self.anvil_enable_traces().await.to_rpc_result(),
        }
    }

    fn sign_request(
        &self,
        from: &Address,
        request: TypedTransactionRequest,
    ) -> Result<TypedTransaction> {
        for signer in self.signers.iter() {
            if signer.accounts().contains(from) {
                return signer.sign(request, from)
            }
        }
        Err(BlockchainError::NoSignerAvailable)
    }

    /// Queries the current gas limit
    fn current_gas_limit(&self) -> Result<U256> {
        Ok(self.backend.gas_limit())
    }

    /// Returns protocol version encoded as a string (quotes are necessary).
    ///
    /// Handler for ETH RPC call: `eth_protocolVersion`
    pub fn protocol_version(&self) -> Result<u64> {
        Ok(1)
    }

    /// Returns the number of hashes per second that the node is mining with.
    ///
    /// Handler for ETH RPC call: `eth_hashrate`
    pub fn hashrate(&self) -> Result<U256> {
        Ok(U256::zero())
    }

    /// Returns the client coinbase address.
    ///
    /// Handler for ETH RPC call: `eth_coinbase`
    pub fn author(&self) -> Result<Address> {
        Ok(self.backend.coinbase())
    }

    /// Returns true if client is actively mining new blocks.
    ///
    /// Handler for ETH RPC call: `eth_mining`
    pub fn is_mining(&self) -> Result<bool> {
        Ok(self.is_mining)
    }

    /// Returns the chain ID used for transaction signing at the
    /// current best block. None is returned if not
    /// available.
    ///
    /// Handler for ETH RPC call: `eth_chainId`
    pub fn chain_id(&self) -> Result<Option<U64>> {
        Ok(Some(self.backend.chain_id().as_u64().into()))
    }

    pub fn gas_price(&self) -> Result<U256> {
        Ok(self.backend.gas_price())
    }

    /// Returns the accounts list
    ///
    /// Handler for ETH RPC call: `eth_accounts`
    pub fn accounts(&self) -> Result<Vec<Address>> {
        let mut accounts = Vec::new();
        for signer in self.signers.iter() {
            accounts.append(&mut signer.accounts());
        }
        Ok(accounts)
    }

    /// Returns the number of most recent block.
    ///
    /// Handler for ETH RPC call: `eth_blockNumber`
    pub fn block_number(&self) -> Result<U256> {
        Ok(self.backend.best_number().as_u64().into())
    }

    /// Returns balance of the given account.
    ///
    /// Handler for ETH RPC call: `eth_getBalance`
    pub fn balance(&self, address: Address, number: Option<BlockNumber>) -> Result<U256> {
        let number = number.unwrap_or(BlockNumber::Latest);
        match number {
            BlockNumber::Latest | BlockNumber::Pending => Ok(self.backend.current_balance(address)),
            BlockNumber::Number(num) => {
                if num != self.backend.best_number() {
                    Err(BlockchainError::RpcUnimplemented)
                } else {
                    Ok(self.backend.current_balance(address))
                }
            }
            _ => Err(BlockchainError::RpcUnimplemented),
        }
    }

    /// Returns content of the storage at given address.
    ///
    /// Handler for ETH RPC call: `eth_getStorageAt`
    pub async fn storage_at(
        &self,
        address: Address,
        index: U256,
        number: Option<BlockNumber>,
    ) -> Result<H256> {
        self.backend.storage_at(address, index, number).await
    }

    /// Returns block with given hash.
    ///
    /// Handler for ETH RPC call: `eth_getBlockByHash`
    pub async fn block_by_hash(&self, hash: H256, _full: bool) -> Result<Option<Block<TxHash>>> {
        self.backend.block_by_hash(hash).await
    }

    /// Returns block with given number.
    ///
    /// Handler for ETH RPC call: `eth_getBlockByNumber`
    pub async fn block_by_number(
        &self,
        number: BlockNumber,
        _: bool,
    ) -> Result<Option<Block<TxHash>>> {
        self.backend.block_by_number(number).await
    }

    /// Returns the number of transactions sent from given address at given time (block number).
    ///
    /// Handler for ETH RPC call: `eth_getTransactionCount`
    pub fn transaction_count(&self, address: Address, number: Option<BlockNumber>) -> Result<U256> {
        let number = number.unwrap_or(BlockNumber::Latest);
        match number {
            BlockNumber::Latest | BlockNumber::Pending => Ok(self.backend.current_nonce(address)),
            BlockNumber::Number(num) => {
                if num != self.backend.best_number() {
                    Err(BlockchainError::RpcUnimplemented)
                } else {
                    Ok(self.backend.current_nonce(address))
                }
            }
            _ => Err(BlockchainError::RpcUnimplemented),
        }
    }

    /// Returns the number of transactions in a block with given hash.
    ///
    /// Handler for ETH RPC call: `eth_getBlockTransactionCountByHash`
    pub fn block_transaction_count_by_hash(&self, _: H256) -> Result<Option<U256>> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Returns the number of transactions in a block with given block number.
    ///
    /// Handler for ETH RPC call: `eth_getBlockTransactionCountByNumber`
    pub fn block_transaction_count_by_number(&self, _: BlockNumber) -> Result<Option<U256>> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Returns the number of uncles in a block with given hash.
    ///
    /// Handler for ETH RPC call: `eth_getUncleCountByBlockHash`
    pub fn block_uncles_count_by_hash(&self, _: H256) -> Result<U256> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Returns the number of uncles in a block with given block number.
    ///
    /// Handler for ETH RPC call: `eth_getUncleCountByBlockNumber`
    pub fn block_uncles_count_by_number(&self, _: BlockNumber) -> Result<U256> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Returns the code at given address at given time (block number).
    ///
    /// Handler for ETH RPC call: `eth_getCode`
    pub async fn get_code(&self, address: Address, block: Option<BlockNumber>) -> Result<Bytes> {
        self.backend.get_code(address, block).await
    }

    /// Sends a transaction
    ///
    /// Handler for ETH RPC call: `eth_sendTransaction`
    pub fn send_transaction(&self, request: EthTransactionRequest) -> Result<TxHash> {
        let from = request.from.map(Ok).unwrap_or_else(|| {
            self.accounts()?.get(0).cloned().ok_or(BlockchainError::NoSignerAvailable)
        })?;

        let on_chain_nonce = self.transaction_count(from, None)?;
        let nonce = request.nonce.unwrap_or(on_chain_nonce);

        let chain_id = self.chain_id()?.ok_or(BlockchainError::ChainIdNotAvailable)?.as_u64();

        let max_fee_per_gas = request.max_fee_per_gas;
        let gas_price = request.gas_price;

        let gas_limit = request.gas.map(Ok).unwrap_or_else(|| self.current_gas_limit())?;

        let request = match request.into_typed_request() {
            Some(TypedTransactionRequest::Legacy(mut m)) => {
                m.nonce = nonce;
                m.chain_id = Some(chain_id);
                m.gas_limit = gas_limit;
                if gas_price.is_none() {
                    m.gas_price = self.gas_price().unwrap_or_default();
                }
                TypedTransactionRequest::Legacy(m)
            }
            Some(TypedTransactionRequest::EIP2930(mut m)) => {
                m.nonce = nonce;
                m.chain_id = chain_id;
                m.gas_limit = gas_limit;
                if gas_price.is_none() {
                    m.gas_price = self.gas_price().unwrap_or_default();
                }
                TypedTransactionRequest::EIP2930(m)
            }
            Some(TypedTransactionRequest::EIP1559(mut m)) => {
                m.nonce = nonce;
                m.chain_id = chain_id;
                m.gas_limit = gas_limit;
                if max_fee_per_gas.is_none() {
                    m.max_fee_per_gas = self.gas_price().unwrap_or_default();
                }
                TypedTransactionRequest::EIP1559(m)
            }
            _ => return Err(BlockchainError::FailedToDecodeTransaction),
        };

        let transaction = self.sign_request(&from, request)?;
        let pending_transaction = PendingTransaction::new(transaction)?;

        // pre-validate
        self.backend.validate_transaction(&pending_transaction)?;

        let prev_nonce = nonce.saturating_sub(U256::one());
        let requires = if on_chain_nonce < prev_nonce {
            vec![to_marker(prev_nonce.as_u64(), from)]
        } else {
            vec![]
        };

        let pool_transaction = PoolTransaction {
            requires,
            provides: vec![to_marker(nonce.as_u64(), from)],
            pending_transaction,
        };

        let tx = self.pool.add_transaction(pool_transaction)?;
        Ok(*tx.hash())
    }

    /// Sends signed transaction, returning its hash.
    ///
    /// Handler for ETH RPC call: `eth_sendRawTransaction`
    pub fn send_raw_transaction(&self, tx: Bytes) -> Result<TxHash> {
        let data = tx.as_ref();
        if data.is_empty() {
            return Err(BlockchainError::EmptyRawTransactionData)
        }
        let transaction = if data[0] > 0x7f {
            // legacy transaction
            match rlp::decode::<LegacyTransaction>(data) {
                Ok(transaction) => TypedTransaction::Legacy(transaction),
                Err(_) => return Err(BlockchainError::FailedToDecodeSignedTransaction),
            }
        } else {
            // the [TypedTransaction] requires a valid rlp input,
            // but EIP-1559 prepends a version byte, so we need to encode the data first to get a
            // valid rlp and then rlp decode impl of `TypedTransaction` will remove and check the
            // version byte
            let extend = rlp::encode(&data);
            match rlp::decode::<TypedTransaction>(&extend[..]) {
                Ok(transaction) => transaction,
                Err(_) => return Err(BlockchainError::FailedToDecodeSignedTransaction),
            }
        };

        let pending_transaction = PendingTransaction::new(transaction)?;

        // pre-validate
        self.backend.validate_transaction(&pending_transaction)?;

        let on_chain_nonce = self.backend.current_nonce(*pending_transaction.sender());
        let nonce = *pending_transaction.transaction.nonce();
        let prev_nonce = nonce.saturating_sub(U256::one());

        let requires = if on_chain_nonce < prev_nonce {
            vec![to_marker(prev_nonce.as_u64(), *pending_transaction.sender())]
        } else {
            vec![]
        };

        let pool_transaction = PoolTransaction {
            requires,
            provides: vec![to_marker(nonce.as_u64(), *pending_transaction.sender())],
            pending_transaction,
        };

        let tx = self.pool.add_transaction(pool_transaction)?;
        Ok(*tx.hash())
    }

    /// Call contract, returning the output data.
    ///
    /// Handler for ETH RPC call: `eth_call`
    pub fn call(&self, request: CallRequest, _number: Option<BlockNumber>) -> Result<Bytes> {
        let fees = FeeDetails::new(
            request.gas_price,
            request.max_fee_per_gas,
            request.max_priority_fee_per_gas,
        )?;

        let out = match self.backend.call(request, fees).1 {
            TransactOut::None => Default::default(),
            TransactOut::Call(out) => out.to_vec().into(),
            TransactOut::Create(out, _) => out.to_vec().into(),
        };
        Ok(out)
    }

    /// Estimate gas needed for execution of given contract.
    ///
    /// Handler for ETH RPC call: `eth_estimateGas`
    pub fn estimate_gas(&self, request: CallRequest, _: Option<BlockNumber>) -> Result<U256> {
        let gas = self.backend.call(request, FeeDetails::zero()).2;

        Ok(gas.into())
    }

    /// Get transaction by its hash.
    ///
    /// Handler for ETH RPC call: `eth_getTransactionByHash`
    pub async fn transaction_by_hash(&self, hash: H256) -> Result<Option<Transaction>> {
        // TODO also check pending tx
        self.backend.transaction_by_hash(hash).await
    }

    /// Returns transaction at given block hash and index.
    ///
    /// Handler for ETH RPC call: `eth_getTransactionByBlockHashAndIndex`
    pub async fn transaction_by_block_hash_and_index(
        &self,
        hash: H256,
        index: Index,
    ) -> Result<Option<Transaction>> {
        self.backend.transaction_by_block_hash_and_index(hash, index).await
    }

    /// Returns transaction by given block number and index.
    ///
    /// Handler for ETH RPC call: `eth_getTransactionByBlockNumberAndIndex`
    pub fn transaction_by_block_number_and_index(
        &self,
        _: BlockNumber,
        _: Index,
    ) -> Result<Option<Transaction>> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Returns transaction receipt by transaction hash.
    ///
    /// Handler for ETH RPC call: `eth_getTransactionReceipt`
    pub async fn transaction_receipt(&self, hash: H256) -> Result<Option<TransactionReceipt>> {
        self.backend.transaction_receipt(hash).await
    }

    /// Returns an uncles at given block and index.
    ///
    /// Handler for ETH RPC call: `eth_getUncleByBlockHashAndIndex`
    pub fn uncle_by_block_hash_and_index(
        &self,
        _: H256,
        _: Index,
    ) -> Result<Option<Block<TxHash>>> {
        Ok(None)
    }

    pub fn uncle_by_block_number_and_index(
        &self,
        _: BlockNumber,
        _: Index,
    ) -> Result<Option<Block<TxHash>>> {
        Ok(None)
    }

    /// Returns logs matching given filter object.
    ///
    /// Handler for ETH RPC call: `eth_getLogs`
    pub async fn logs(&self, filter: Filter) -> Result<Vec<Log>> {
        self.backend.logs(filter).await
    }

    /// Returns the hash of the current block, the seedHash, and the boundary condition to be met.
    ///
    /// Handler for ETH RPC call: `eth_getWork`
    pub fn work(&self) -> Result<Work> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Used for submitting a proof-of-work solution.
    ///
    /// Handler for ETH RPC call: `eth_submitWork`
    pub fn submit_work(&self, _: H64, _: H256, _: H256) -> Result<bool> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Used for submitting mining hashrate.
    ///
    /// Handler for ETH RPC call: `eth_submitHashrate`
    pub fn submit_hashrate(&self, _: U256, _: H256) -> Result<bool> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Introduced in EIP-1159 for getting information on the appropriate priority fee to use.
    ///
    /// Handler for ETH RPC call: `eth_feeHistory`
    ///
    /// TODO actually track fee history
    pub fn fee_history(
        &self,
        block_count: U256,
        newest_block: BlockNumber,
        reward_percentiles: Vec<f64>,
    ) -> Result<FeeHistory> {
        // max number of blocks in the requested range
        const MAX_BLOCK_COUNT: u64 = 1024u64;

        let range_limit = U256::from(MAX_BLOCK_COUNT);
        let block_count =
            if block_count > range_limit { range_limit.as_u64() } else { block_count.as_u64() };

        let number = match newest_block {
            BlockNumber::Latest | BlockNumber::Pending => self.backend.best_number().as_u64(),
            BlockNumber::Earliest => 0,
            BlockNumber::Number(n) => n.as_u64(),
        };

        // highest and lowest block num in the requested range
        let highest = number;
        let lowest = highest.saturating_sub(block_count);

        if lowest < self.backend.best_number().as_u64() {
            return Err(FeeHistoryError::InvalidBlockRange.into())
        }

        let fee_history = self.fee_history_cache.lock();

        let mut response = FeeHistory {
            oldest_block: U256::from(lowest),
            base_fee_per_gas: Vec::new(),
            gas_used_ratio: Vec::new(),
            reward: None,
        };

        let mut rewards = Vec::new();
        // iter over the requested block range
        for n in lowest..highest + 1 {
            // <https://eips.ethereum.org/EIPS/eip-1559>
            if let Some(block) = fee_history.get(&n) {
                response.base_fee_per_gas.push(U256::from(block.base_fee));
                response.gas_used_ratio.push(block.gas_used_ratio);

                // requested percentiles
                if !reward_percentiles.is_empty() {
                    let mut block_rewards = Vec::new();
                    let resolution_per_percentile: f64 = 2.0;
                    for p in &reward_percentiles {
                        let p = p.clamp(0.0, 100.0);
                        let index = ((p.round() / 2f64) * 2f64) * resolution_per_percentile;
                        let reward = if let Some(r) = block.rewards.get(index as usize) {
                            U256::from(*r)
                        } else {
                            U256::zero()
                        };
                        block_rewards.push(reward);
                    }
                    rewards.push(block_rewards);
                }
            }
        }

        response.reward = Some(rewards);

        // calculate next base fee
        if let (Some(last_gas_used), Some(last_fee_per_gas)) =
            (response.gas_used_ratio.last(), response.base_fee_per_gas.last())
        {
            let elasticity = self.backend.elasticity();
            let last_fee_per_gas = last_fee_per_gas.as_u64() as f64;
            if last_gas_used > &0.5 {
                // increase base gas
                let increase = ((last_gas_used - 0.5) * 2f64) * elasticity;
                let new_base_fee = (last_fee_per_gas + (last_fee_per_gas * increase)) as u64;
                response.base_fee_per_gas.push(U256::from(new_base_fee));
            } else if last_gas_used < &0.5 {
                // decrease gas
                let increase = ((0.5 - last_gas_used) * 2f64) * elasticity;
                let new_base_fee = (last_fee_per_gas - (last_fee_per_gas * increase)) as u64;
                response.base_fee_per_gas.push(U256::from(new_base_fee));
            } else {
                // same base gas
                response.base_fee_per_gas.push(U256::from(last_fee_per_gas as u64));
            }
        }

        Ok(response)
    }

    /// Introduced in EIP-1159, a Geth-specific and simplified priority fee oracle.
    /// Leverages the already existing fee history cache.
    ///
    /// Handler for ETH RPC call: `eth_maxPriorityFeePerGas`
    pub fn max_priority_fee_per_gas(&self) -> Result<U256> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Returns traces for the transaction hash
    ///
    /// Handler for RPC call: `debug_traceTransaction`
    pub async fn debug_trace_transaction(&self, _tx_hash: H256) -> Result<Vec<Trace>> {
        Err(BlockchainError::RpcUnimplemented)
    }
}

// == impl EthApi forge endpoints ==

impl EthApi {
    /// Send transactions impersonating specific account and contract addresses.
    ///
    /// Handler for ETH RPC call: `anvil_impersonateAccount`
    pub async fn anvil_impersonate_account(&self, address: Address) -> Result<()> {
        self.backend.cheats().impersonate(address);
        Ok(())
    }

    /// Stops impersonating an account if previously set with `anvil_impersonateAccount`.
    ///
    /// Handler for ETH RPC call: `anvil_stopImpersonatingAccount`
    pub async fn anvil_stop_impersonating_account(&self) -> Result<()> {
        self.backend.cheats().stop_impersonating();
        Ok(())
    }

    /// Returns true if automatic mining is enabled, and false.
    ///
    /// Handler for ETH RPC call: `anvil_getAutomine`
    pub async fn anvil_get_auto_mine(&self) -> Result<bool> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Enables or disables, based on the single boolean argument, the automatic mining of new
    /// blocks with each new transaction submitted to the network.
    ///
    /// Handler for ETH RPC call: `evm_setAutomine`
    pub async fn anvil_set_auto_mine(&self, _mine: bool) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Mines a series of blocks.
    ///
    /// Handler for ETH RPC call: `anvil_mine`
    pub async fn anvil_mine(
        &self,
        _num_blocks: Option<U256>,
        _interval: Option<U256>,
    ) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Sets the mining behavior to interval with the given interval (seconds)
    ///
    /// Handler for ETH RPC call: `evm_setIntervalMining`
    pub async fn anvil_set_interval_mining(&self, _secs: u64) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Removes transactions from the pool
    ///
    /// Handler for RPC call: `anvil_dropTransaction`
    pub async fn anvil_drop_transaction(&self, tx_hash: H256) -> Result<Option<H256>> {
        Ok(self.pool.drop_transaction(tx_hash).map(|tx| *tx.hash()))
    }

    /// Reset the fork to a fresh forked state, and optionally update the fork config
    ///
    /// Handler for RPC call: `anvil_reset`
    pub async fn anvil_reset(&self, _forking: Forking) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    ///Modifies the balance of an account.
    ///
    /// Handler for RPC call: `anvil_setBalance`
    pub async fn anvil_set_balance(&self, address: Address, balance: U256) -> Result<()> {
        self.backend.set_balance(address, balance);
        Ok(())
    }

    /// Sets the code of a contract.
    ///
    /// Handler for RPC call: `anvil_setCode`
    pub async fn anvil_set_code(&self, address: Address, code: Bytes) -> Result<()> {
        self.backend.set_code(address, code);
        Ok(())
    }

    /// Sets the nonce of an address.
    ///
    /// Handler for RPC call: `anvil_setNonce`
    pub async fn anvil_set_nonce(&self, address: Address, nonce: U256) -> Result<()> {
        self.backend.set_nonce(address, nonce);
        Ok(())
    }

    /// Writes a single slot of the account's storage.
    ///
    /// Handler for RPC call: `anvil_setStorageAt`
    pub async fn anvil_set_storage_at(
        &self,
        address: Address,
        slot: U256,
        val: U256,
    ) -> Result<()> {
        self.backend.set_storage_at(address, slot, val);
        Ok(())
    }

    /// Enable or disable logging.
    ///
    /// Handler for RPC call: `anvil_setLoggingEnabled`
    pub async fn anvil_set_logging(&self, _enable: bool) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Set the minimum gas price for the node.
    ///
    /// Handler for RPC call: `anvil_setMinGasPrice`
    pub async fn anvil_set_min_gas_price(&self, _gas: U256) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Sets the base fee of the next block.
    ///
    /// Handler for RPC call: `anvil_setNextBlockBaseFeePerGas`
    pub async fn anvil_set_next_block_base_fee_per_gas(&self, _gas: U256) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Sets the coinbase address.
    ///
    /// Handler for RPC call: `anvil_setCoinbase`
    pub async fn anvil_set_coinbase(&self, address: Address) -> Result<()> {
        self.backend.set_coinbase(address);
        Ok(())
    }

    /// Snapshot the state of the blockchain at the current block.
    ///
    /// Handler for RPC call: `evm_snapshot`
    pub async fn evm_snapshot(&self) -> Result<U256> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Revert the state of the blockchain to a previous snapshot.
    /// Takes a single parameter, which is the snapshot id to revert to.
    ///
    /// Handler for RPC call: `evm_revert`
    pub async fn evm_revert(&self, _id: U256) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Jump forward in time by the given amount of time, in seconds.
    ///
    /// Handler for RPC call: `evm_increaseTime`
    pub async fn evm_increase_time(&self, seconds: U256) -> Result<()> {
        self.backend.time().increase_time(seconds.try_into().unwrap_or(u64::MAX));
        Ok(())
    }

    /// Similar to `evm_increaseTime` but takes the exact timestamp that you want in the next block
    ///
    /// Handler for RPC call: `evm_setNextBlockTimestamp`
    pub async fn evm_set_next_block_timestamp(&self, seconds: u64) -> Result<()> {
        self.backend.time().set_next_block_timestamp(seconds);
        Ok(())
    }

    /// Mine a single block
    ///
    /// Handler for RPC call: `evm_mine`
    pub async fn evm_mine(&self, _opts: EvmMineOptions) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Sets the reported block number
    ///
    /// Handler for ETH RPC call: `anvil_setBlock`
    pub fn anvil_set_block(&self, _block_number: U256) -> Result<U256> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Sets the backend rpc url
    ///
    /// Handler for ETH RPC call: `anvil_setRpcUrl`
    pub fn anvil_set_rpc_url(&self, url: String) -> Result<()> {
        if let Some(fork) = self.backend.get_fork() {
            let new_provider = Arc::new(Provider::try_from(&url).map_err(|_| {
                ProviderError::CustomError(format!("Failed to parse invalid url {}", url))
            })?);
            let mut config = fork.config.write();
            trace!(target: "backend", "Updated fork rpc from \"{}\" to \"{}\"", config.eth_rpc_url, url);
            config.eth_rpc_url = url;
            config.provider = new_provider;
        }
        Ok(())
    }

    /// Turn on call traces for transactions that are returned to the user when they execute a
    /// transaction (instead of just txhash/receipt)
    ///
    /// Handler for ETH RPC call: `anvil_enableTraces`
    pub async fn anvil_enable_traces(&self) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Execute a transaction regardless of signature status
    ///
    /// Handler for ETH RPC call: `eth_sendUnsignedTransaction`
    pub async fn eth_send_unsigned_transaction(
        &self,
        _req: EthTransactionRequest,
    ) -> Result<TxHash> {
        Err(BlockchainError::RpcUnimplemented)
    }
}
