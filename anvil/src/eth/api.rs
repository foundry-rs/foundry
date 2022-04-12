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
    response::ResponseResult,
    types::{Index, Work},
};
use ethers::{
    abi::ethereum_types::H64,
    types::{
        Address, Block, BlockNumber, Bytes, Log, Transaction, TransactionReceipt, TxHash, H256,
        U256, U64,
    },
    utils::rlp,
};
use std::sync::Arc;

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
    is_authority: bool,
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
        Self { pool, backend, is_authority: true, signers, fee_history_cache }
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
            EthRequest::EthGetLogs(filter) => self.logs(filter).to_rpc_result(),
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
        Ok(self.is_authority)
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
    pub fn logs(&self, _: Filter) -> Result<Vec<Log>> {
        Err(BlockchainError::RpcUnimplemented)
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
}

// == impl EthApi forge endpoints ==

impl EthApi {
    /// Sets the reported block number
    ///
    /// Handler for ETH RPC call: `forge_setBlock`
    pub fn forge_set_block(&self, _block_number: U256) -> Result<U256> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Sets the backend rpc url
    ///
    /// Handler for ETH RPC call: `forge_setRpcUrl`
    pub fn forge_set_rpc_url(&self, _url: String) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Sets the mining mode
    ///
    /// Handler for ETH RPC call: `forge_mining`
    pub fn forge_mining(&self) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Sets block timestamp
    ///
    /// Handler for ETH RPC call: `forge_setTimestamp`
    pub fn forge_set_timestamp(&self) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// Turn on call traces for transactions that are returned to the user when they execute a
    /// transaction (instead of just txhash/receipt)
    ///
    /// Handler for ETH RPC call: `forge_enableTraces`
    pub fn forge_enable_traces(&self) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }

    /// execute a transaction regardless of signature status
    ///
    /// Handler for ETH RPC call: `eth_sendUnsignedTransaction`
    pub fn eth_send_unsigned_transaction(&self) -> Result<()> {
        Err(BlockchainError::RpcUnimplemented)
    }
}
