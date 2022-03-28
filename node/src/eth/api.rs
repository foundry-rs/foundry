use crate::eth::{
    backend::Backend,
    error::{BlockchainError, Result},
    pool::Pool,
    sign::Signer,
};
use ethers::{
    abi::ethereum_types::H64,
    types::{
        Address, Block, BlockNumber, Bytes, FeeHistory, Filter, Log, Transaction,
        TransactionReceipt, TxHash, H160, H256, U256, U64,
    },
};
use forge_node_core::{
    eth::{
        transaction::{EthTransactionRequest, TypedTransaction, TypedTransactionRequest},
        EthRequest,
    },
    response::RpcResponse,
    types::{Index, Work},
};
use std::sync::Arc;

/// The entry point for executing eth api RPC call - The Eth RPC interface
#[derive(Clone)]
pub struct EthApi {
    /// The transaction pool
    pool: Arc<Pool>,
    /// Holds all blockchain related data
    backend: Arc<Backend>,
    /// Whether this node is mining
    is_authority: bool,
    /// available signers
    signers: Arc<Vec<Box<dyn Signer>>>,

    chain_id: Option<U64>,
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
        todo!()
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

    /// Returns the block author.
    ///
    /// Handler for ETH RPC call: `eth_coinbase`
    pub async fn author(&self) -> Result<H160> {
        todo!()
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
        Ok(self.chain_id)
    }

    /// Returns the current gas_price
    ///
    /// Handler for ETH RPC call: `eth_gasPrice`
    pub fn gas_price(&self) -> Result<U256> {
        todo!()
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

    /// Returns the highest block number.
    ///
    /// Handler for ETH RPC call: `eth_blockNumber`
    pub async fn block_number(&self) -> Result<U256> {
        todo!()
    }

    /// Returns balance of the given account.
    ///
    /// Handler for ETH RPC call: `eth_getBalance`
    pub async fn balance(&self, _address: H160, _number: Option<BlockNumber>) -> Result<U256> {
        todo!()
    }

    /// Returns content of the storage at given address.
    ///
    /// Handler for ETH RPC call: `eth_getStorageAt`
    pub async fn storage_at(
        &self,
        _address: H160,
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
    pub fn transaction_count(&self, _address: H160, _: Option<BlockNumber>) -> Result<U256> {
        todo!()
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
    pub async fn code_at(&self, _address: H160, _: Option<BlockNumber>) -> Result<Bytes> {
        todo!()
    }

    /// Sends a transaction
    /// will wait for signer to return the transaction hash.
    ///
    /// Handler for ETH RPC call: `eth_sendTransaction`
    pub async fn send_transaction(&self, request: EthTransactionRequest) -> Result<H256> {
        let from = request.from.map(Ok).unwrap_or_else(|| {
            self.accounts()?.get(0).cloned().ok_or(BlockchainError::NoSignerAvailable)
        })?;

        let nonce = request.nonce.map(Ok).unwrap_or_else(|| self.transaction_count(from, None))?;

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
            _ => return { Err(BlockchainError::InvalidTransaction) },
        };

        let transaction = self.sign_request(&from, request)?;

        todo!()
    }

    /// Sends signed transaction, returning its hash.
    ///
    /// Handler for ETH RPC call: `eth_sendRawTransaction`
    pub async fn send_raw_transaction(&self, _: Bytes) -> Result<H256> {
        todo!()
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
