use crate::eth::{
    backend,
    error::{BlockchainError, Result},
    pool::{
        transactions::{to_marker, PoolTransaction},
        Pool,
    },
    sign::Signer,
};
use ethers::{
    abi::ethereum_types::H64,
    types::{
        Address, Block, BlockNumber, Bytes, FeeHistory, Filter, Log, Transaction,
        TransactionReceipt, TxHash, H256, U256, U64,
    },
    utils::rlp,
};
use forge_node_core::{
    eth::{
        transaction::{
            EthTransactionRequest, LegacyTransaction, PendingTransaction, TypedTransaction,
            TypedTransactionRequest,
        },
        EthRequest,
    },
    response::RpcResponse,
    types::{Index, Work},
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
}

// === impl Eth RPC API ===

impl EthApi {
    /// Executes the [EthRequest] and returns an RPC [RpcResponse]
    pub async fn execute(&self, request: EthRequest) -> RpcResponse {
        match request {
            EthRequest::EthGetBalance(_, _) => {}
            EthRequest::EthGetTransactionByHash(_) => {}
            EthRequest::EthSendTransaction(request) => {
                self.send_transaction(*request).await;
            }
        }

        todo!()
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
    pub async fn protocol_version(&self) -> Result<u64> {
        Ok(1)
    }

    /// Returns the number of hashes per second that the node is mining with.
    ///
    /// Handler for ETH RPC call: `eth_hashrate`
    pub async fn hashrate(&self) -> Result<U256> {
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
    pub async fn is_mining(&self) -> Result<bool> {
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

    /// Returns the current gas_price
    ///
    /// Handler for ETH RPC call: `eth_gasPrice`
    pub fn gas_price(&self) -> Result<U256> {
        Err(BlockchainError::RpcUnimplemented)
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
    pub async fn block_number(&self) -> Result<U256> {
        Ok(self.backend.best_number().as_u64().into())
    }

    /// Returns balance of the given account.
    ///
    /// Handler for ETH RPC call: `eth_getBalance`
    pub async fn balance(&self, address: Address, number: Option<BlockNumber>) -> Result<U256> {
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
        _address: Address,
        _index: U256,
        _number: Option<BlockNumber>,
    ) -> Result<H256> {
        todo!()
    }

    /// Returns block with given hash.
    ///
    /// Handler for ETH RPC call: `eth_getBlockByHash`
    pub async fn block_by_hash(&self, _hash: H256, _full: bool) -> Result<Option<Block<TxHash>>> {
        todo!()
    }

    /// Returns block with given number.
    ///
    /// Handler for ETH RPC call: `eth_getBlockByNumber`
    pub async fn block_by_number(&self, _: BlockNumber, _: bool) -> Result<Option<Block<TxHash>>> {
        todo!()
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
    pub async fn block_transaction_count_by_hash(&self, _: H256) -> Result<Option<U256>> {
        todo!()
    }

    /// Returns the number of transactions in a block with given block number.
    ///
    /// Handler for ETH RPC call: `eth_getBlockTransactionCountByNumber`
    pub async fn block_transaction_count_by_number(&self, _: BlockNumber) -> Result<Option<U256>> {
        todo!()
    }

    /// Returns the number of uncles in a block with given hash.
    ///
    /// Handler for ETH RPC call: `eth_getUncleCountByBlockHash`
    pub async fn block_uncles_count_by_hash(&self, _: H256) -> Result<U256> {
        todo!()
    }

    /// Returns the number of uncles in a block with given block number.
    ///
    /// Handler for ETH RPC call: `eth_getUncleCountByBlockNumber`
    pub async fn block_uncles_count_by_number(&self, _: BlockNumber) -> Result<U256> {
        todo!()
    }

    /// Returns the code at given address at given time (block number).
    ///
    /// Handler for ETH RPC call: `eth_getCode`
    pub async fn code_at(&self, _address: Address, _: Option<BlockNumber>) -> Result<Bytes> {
        todo!()
    }

    /// Sends a transaction
    ///
    /// Handler for ETH RPC call: `eth_sendTransaction`
    pub async fn send_transaction(&self, request: EthTransactionRequest) -> Result<TxHash> {
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
            _ => return Err(BlockchainError::InvalidTransaction),
        };

        let transaction = self.sign_request(&from, request)?;
        let pending_transaction = PendingTransaction::new(transaction)?;

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
    pub async fn send_raw_transaction(&self, tx: Bytes) -> Result<TxHash> {
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
    pub async fn call(
        &self,
        _request: TypedTransaction,
        _number: Option<BlockNumber>,
    ) -> Result<Bytes> {
        todo!()
    }

    /// Estimate gas needed for execution of given contract.
    ///
    /// Handler for ETH RPC call: `eth_estimateGas`
    pub async fn estimate_gas(
        &self,
        _request: TypedTransaction,
        _number: Option<BlockNumber>,
    ) -> Result<U256> {
        todo!()
    }

    /// Get transaction by its hash.
    ///
    /// Handler for ETH RPC call: `eth_getTransactionByHash`
    pub async fn transaction_by_hash(&self, _: H256) -> Result<Option<Transaction>> {
        todo!()
    }

    /// Returns transaction at given block hash and index.
    ///
    /// Handler for ETH RPC call: `eth_getTransactionByBlockHashAndIndex`
    pub async fn transaction_by_block_hash_and_index(
        &self,
        _: H256,
        _: Index,
    ) -> Result<Option<Transaction>> {
        todo!()
    }

    /// Returns transaction by given block number and index.
    ///
    /// Handler for ETH RPC call: `eth_getTransactionByBlockNumberAndIndex`
    pub async fn transaction_by_block_number_and_index(
        &self,
        _: BlockNumber,
        _: Index,
    ) -> Result<Option<Transaction>> {
        todo!()
    }

    /// Returns transaction receipt by transaction hash.
    ///
    /// Handler for ETH RPC call: `eth_getTransactionReceipt`
    pub async fn transaction_receipt(&self, _hash: H256) -> Result<Option<TransactionReceipt>> {
        todo!()
    }

    /// Returns an uncles at given block and index.
    ///
    /// Handler for ETH RPC call: `eth_getUncleByBlockHashAndIndex`
    pub async fn uncle_by_block_hash_and_index(
        &self,
        _: H256,
        _: Index,
    ) -> Result<Option<Block<TxHash>>> {
        Ok(None)
    }

    /// Returns logs matching given filter object.
    ///
    /// Handler for ETH RPC call: `eth_getLogs`
    pub async fn logs(&self, _: Filter) -> Result<Vec<Log>> {
        todo!()
    }

    /// Returns the hash of the current block, the seedHash, and the boundary condition to be met.
    ///
    /// Handler for ETH RPC call: `eth_getWork`
    pub async fn work(&self) -> Result<Work> {
        todo!()
    }

    /// Used for submitting a proof-of-work solution.
    ///
    /// Handler for ETH RPC call: `eth_submitWork`
    pub async fn submit_work(&self, _: H64, _: H256, _: H256) -> Result<bool> {
        todo!()
    }

    /// Used for submitting mining hashrate.
    ///
    /// Handler for ETH RPC call: `eth_submitHashrate`
    pub async fn submit_hashrate(&self, _: U256, _: H256) -> Result<bool> {
        todo!()
    }

    /// Introduced in EIP-1159 for getting information on the appropriate priority fee to use.
    ///
    /// Handler for ETH RPC call: `eth_feeHistory`
    pub async fn fee_history(
        &self,
        _block_count: U256,
        _newest_block: BlockNumber,
        _reward_percentiles: Option<Vec<f64>>,
    ) -> Result<FeeHistory> {
        todo!()
    }

    /// Introduced in EIP-1159, a Geth-specific and simplified priority fee oracle.
    /// Leverages the already existing fee history cache.
    ///
    /// Handler for ETH RPC call: `eth_maxPriorityFeePerGas`
    pub async fn max_priority_fee_per_gas(&self) -> Result<U256> {
        todo!()
    }
}

// == impl EthApi forge endpoints ==

impl EthApi {
    /// Sets the reported block number
    ///
    /// Handler for ETH RPC call: `forge_setBlock`
    pub async fn forge_set_block(&self, _block_number: U256) -> Result<U256> {
        todo!()
    }

    /// Sets the backend rpc url
    ///
    /// Handler for ETH RPC call: `forge_setRpcUrl`
    pub async fn forge_set_rpc_url(&self, _url: String) -> Result<()> {
        todo!()
    }

    /// Sets the mining mode
    ///
    /// Handler for ETH RPC call: `forge_mining`
    pub async fn forge_mining(&self) -> Result<()> {
        todo!()
    }

    /// Sets block timestamp
    ///
    /// Handler for ETH RPC call: `forge_setTimestamp`
    pub async fn forge_set_timestamp(&self) -> Result<()> {
        todo!()
    }

    /// Turn on call traces for transactions that are returned to the user when they execute a
    /// transaction (instead of just txhash/receipt)
    ///
    /// Handler for ETH RPC call: `forge_enableTraces`
    pub async fn forge_enable_traces(&self) -> Result<()> {
        todo!()
    }

    /// execute a transaction regardless of signature status
    ///
    /// Handler for ETH RPC call: `eth_sendUnsignedTransaction`
    pub async fn eth_send_unsigned_transaction(&self) -> Result<()> {
        todo!()
    }
}
