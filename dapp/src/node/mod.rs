use std::{
    collections::HashMap,
    marker::PhantomData,
    net::SocketAddr,
    sync::{Arc, RwLock},
    time::Duration,
};

use axum::{
    extract::{rejection::JsonRejection, Extension, Json},
    handler::post,
    AddExtensionLayer, Router, Server,
};
use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::{
        transaction::{eip2718::TypedTransaction, eip2930::AccessList},
        Address, Block, Bytes, NameOrAddress, Signer, Transaction, TransactionReceipt, TxHash,
        Wallet, H256, U256, U64,
    },
    utils::keccak256,
};
use evm_adapters::Evm;

mod methods;
use methods::{BoxedError, EthRequest, EthResponse};

mod types;
use types::{Error, JsonRpcRequest, JsonRpcResponse, ResponseContent};

/// Configurations of the EVM node
pub struct NodeConfig {
    /// Chain ID of the EVM chain
    chain_id: U64,
    /// Default gas limit for all txs
    gas_limit: U256,
    /// Default gas price for all txs
    gas_price: U256,
    /// Signer accounts that will be initialised with `genesis_balance` in the genesis block
    genesis_accounts: Vec<Wallet<SigningKey>>,
    /// Native token balance of every genesis account in the genesis block
    genesis_balance: U256,
    /// Signer accounts that can sign messages/transactions from the EVM node
    accounts: HashMap<Address, Wallet<SigningKey>>,
    /// Configured block time for the EVM chain. Use `None` to mine a new block for every tx
    automine: Option<Duration>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            chain_id: U64::one(),
            gas_limit: U256::from(100_000),
            gas_price: U256::from(1_000_000_000),
            genesis_accounts: Vec::new(),
            genesis_balance: U256::zero(),
            accounts: HashMap::new(),
            automine: None,
        }
    }
}

impl NodeConfig {
    /// Returns the default node configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the chain ID
    pub fn chain_id<U: Into<U64>>(mut self, chain_id: U) -> Self {
        self.chain_id = chain_id.into();
        self
    }

    /// Sets the gas limit
    pub fn gas_limit<U: Into<U256>>(mut self, gas_limit: U) -> Self {
        self.gas_limit = gas_limit.into();
        self
    }

    /// Sets the gas price
    pub fn gas_price<U: Into<U256>>(mut self, gas_price: U) -> Self {
        self.gas_price = gas_price.into();
        self
    }

    /// Sets the genesis accounts
    pub fn genesis_accounts(mut self, accounts: Vec<Wallet<SigningKey>>) -> Self {
        self.genesis_accounts = accounts;
        self
    }

    /// Sets the balance of the genesis accounts in the genesis block
    pub fn genesis_balance<U: Into<U256>>(mut self, balance: U) -> Self {
        self.genesis_balance = balance.into();
        self
    }

    /// Sets the block time to automine blocks
    pub fn automine<D: Into<Duration>>(mut self, block_time: D) -> Self {
        self.automine = Some(block_time.into());
        self
    }
}

/// Node represents an EVM-compatible node designed for development environments. It serves an
/// Ethereum-compatible JSON-RPC 2.0 server
pub struct Node<S, E: Evm<S>> {
    /// The node can wrap around a generic implementation of EVM
    evm: E,
    /// The node's configuration
    config: NodeConfig,
    /// The blockchain state of the node
    blockchain: Blockchain,

    _s: PhantomData<S>,
}

#[derive(Default)]
/// Stores the blockchain data (blocks, transactions)
pub struct Blockchain {
    /// Mapping from block hash to the block number
    blocks_by_hash: HashMap<H256, U64>,
    /// Mapping from block number to the block
    blocks: Vec<Block<TxHash>>,
    /// Mapping from txhash to a tuple containing the transaction as well as the transaction
    /// receipt
    txs: HashMap<TxHash, (Transaction, TransactionReceipt)>,

    // TODO(rohit): this should be completely moved to a tx pool module
    /// Pending txs that haven't yet been included in the blockchain
    pending_txs: Vec<Transaction>,
}

impl Blockchain {
    /// Gets transaction by transaction hash
    pub fn tx(&self, tx_hash: TxHash) -> Option<Transaction> {
        self.txs.get(&tx_hash).cloned().and_then(|t| Some(t.0))
    }

    /// Gets transaction receipt by transaction hash
    pub fn tx_receipt(&self, tx_hash: TxHash) -> Option<TransactionReceipt> {
        self.txs.get(&tx_hash).cloned().and_then(|t| Some(t.1))
    }

    /// Gets block by block hash
    pub fn block_by_hash(&self, hash: H256) -> Option<Block<TxHash>> {
        self.blocks_by_hash.get(&hash).and_then(|i| {
            Some(self.block_by_number(*i).expect("block should exist if block hash was found"))
        })
    }

    /// Gets block by block number
    pub fn block_by_number(&self, n: U64) -> Option<Block<TxHash>> {
        if self.blocks.len() > n.as_usize() {
            Some(self.blocks[n.as_usize()].clone())
        } else {
            None
        }
    }

    /// Gets the latest block
    pub fn latest_block(&self) -> Option<Block<TxHash>> {
        self.blocks.last().cloned()
    }
}

impl Blockchain {
    /// Add a pending transaction eligible to be included in the next block
    pub fn insert_pending_tx(&mut self, tx: Transaction) {
        self.pending_txs.push(tx)
    }
}

/// A thread-safe instance guarded by a reader-writer lock
pub type SharedNode<S, E> = Arc<RwLock<Node<S, E>>>;

impl<S, E> Node<S, E>
where
    S: Send + Sync + 'static,
    E: Evm<S> + Send + Sync + 'static,
{
    /// Initialize an instance of Node passing in an EVM-implementation and node config, and run a
    /// JSON-RPC server
    pub async fn init_and_run(mut evm: E, config: NodeConfig) {
        // Configure the balance for genesis accounts
        for account in config.genesis_accounts.iter() {
            evm.set_balance(account.address(), config.genesis_balance);
        }

        // Get the automine configuration
        let automine = config.automine.clone();

        // Create a shared node instance
        let node = Arc::new(RwLock::new(Node {
            evm,
            config,
            blockchain: Blockchain::default(),
            _s: PhantomData,
        }));

        // If node is configured to automine blocks, spawn a new thread to periodically mine blocks
        if let Some(block_time) = automine {
            let shared_node = node.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(block_time.into());
                loop {
                    interval.tick().await;
                    shared_node.write().unwrap().mine_block();
                }
            });
        }

        // Create a service with the shared node's state and serve it
        let svc = Router::new()
            .route("/", post(handler::<E, S>))
            .layer(AddExtensionLayer::new(node))
            .into_make_service();
        let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
        Server::bind(&addr).serve(svc).await.unwrap();
    }
}

impl<S, E> Node<S, E>
where
    E: Evm<S>,
{
    /// Gets the native token balance of the account
    pub fn get_balance(&self, account: Address) -> U256 {
        self.evm.get_balance(account)
    }

    /// Gets the transaction nonce for the account
    pub fn get_nonce(&self, account: Address) -> U256 {
        self.evm.get_nonce(account)
    }

    /// Gets the transaction by txhash
    pub fn get_transaction(&self, tx_hash: TxHash) -> Option<Transaction> {
        self.blockchain.tx(tx_hash)
    }

    /// Gets the transaction receipt by txhash
    pub fn get_tx_receipt(&self, tx_hash: TxHash) -> Option<TransactionReceipt> {
        self.blockchain.tx_receipt(tx_hash)
    }

    /// Gets the block by its number in the blockchain
    pub fn get_block_by_number(&self, n: U64) -> Option<Block<TxHash>> {
        self.blockchain.block_by_number(n)
    }

    /// Gets the block by its hash
    pub fn get_block_by_hash(&self, hash: H256) -> Option<Block<TxHash>> {
        self.blockchain.block_by_hash(hash)
    }

    fn accounts(&self) -> Vec<Wallet<SigningKey>> {
        self.config.accounts.values().cloned().collect()
    }

    fn account(&self, address: Address) -> Option<Wallet<SigningKey>> {
        self.config.accounts.get(&address).cloned()
    }

    fn default_sender(&self) -> Wallet<SigningKey> {
        self.config
            .accounts
            .values()
            .into_iter()
            .next()
            .cloned()
            .expect("node should have at least one account")
    }
}

impl<S, E> Node<S, E>
where
    E: Evm<S>,
{
    fn get_tx_info(
        &self,
        tx: &TypedTransaction,
    ) -> Result<
        (
            Option<U64>,
            Wallet<SigningKey>,
            U256,
            Bytes,
            U256,
            Option<AccessList>,
            Option<U256>,
            Option<U256>,
        ),
        BoxedError,
    > {
        // Tx signer/sender
        let sender = if let Some(from) = tx.from() {
            if let Some(sender) = self.account(*from) {
                sender
            } else {
                return Err(Box::new("account has not been initialized on the node"))
            }
        } else {
            self.default_sender()
        };

        // sender nonce
        let nonce = tx.nonce().cloned().unwrap_or(self.evm.get_nonce(sender.address()));

        // tx value
        let value = *tx.value().unwrap_or(&U256::zero());

        // tx data (calldata or bytecode)
        let data = match tx.data() {
            Some(data) => data.to_vec(),
            None => vec![],
        };

        // EIP-2930 and EIP-1559 related fields
        let (tx_type, access_list, max_priority_fee_per_gas, max_fee_per_gas) = match tx {
            TypedTransaction::Legacy(ref inner) => (
                None,
                None,
                Some(inner.gas_price.unwrap_or(self.config.gas_price)),
                Some(inner.gas_price.unwrap_or(self.config.gas_price)),
            ),
            TypedTransaction::Eip2930(ref inner) => (
                Some(1.into()),
                Some(inner.access_list.clone()),
                Some(inner.tx.gas_price.unwrap_or(self.config.gas_price)),
                Some(inner.tx.gas_price.unwrap_or(self.config.gas_price)),
            ),
            TypedTransaction::Eip1559(ref inner) => (
                Some(2.into()),
                Some(inner.access_list.clone()),
                inner.max_priority_fee_per_gas,
                inner.max_fee_per_gas,
            ),
        };

        Ok((
            tx_type,
            sender,
            value,
            data.into(),
            nonce,
            access_list,
            max_priority_fee_per_gas,
            max_fee_per_gas,
        ))
    }

    /// Simulates sending a transaction on the blockchain and returns the txhash
    pub fn send_transaction(&mut self, tx: TypedTransaction) -> Result<Transaction, BoxedError> {
        let (
            tx_type,
            sender,
            value,
            calldata,
            nonce,
            access_list,
            max_priority_fee_per_gas,
            max_fee_per_gas,
        ) = self.get_tx_info(&tx)?;

        let to = tx.to().expect("tx.to expected");
        let to = match to {
            NameOrAddress::Address(addr) => *addr,
            NameOrAddress::Name(_) => return Err(Box::new("ENS names unsupported")),
        };
        let gas = tx.gas().cloned().unwrap_or(self.config.gas_limit);

        let signature = sender.sign_transaction_sync(&tx);
        let tx_hash = keccak256(tx.rlp_signed(self.config.chain_id, &signature));

        match self.evm.call_raw(sender.address(), to, calldata.clone().into(), value, false) {
            Ok((retdata, status, gas_used, _logs)) => {
                if U256::from(gas_used) > gas {
                    return Err(Box::new("revert: out-of-gas"))
                }
                if E::is_success(&status) {
                    let transaction = Transaction {
                        hash: tx_hash.into(),
                        nonce,
                        block_hash: None,
                        block_number: None,
                        transaction_index: None,
                        from: sender.address(),
                        to: Some(to),
                        value,
                        input: calldata.into(),
                        v: signature.v.into(),
                        r: signature.r,
                        s: signature.s,
                        gas,
                        gas_price: None,
                        transaction_type: tx_type,
                        access_list,
                        max_priority_fee_per_gas,
                        max_fee_per_gas,
                        chain_id: Some(self.config.chain_id.as_u64().into()),
                    };
                    Ok(transaction)
                } else {
                    Err(Box::new(dapp_utils::decode_revert(retdata.as_ref()).unwrap_or_default()))
                }
            }
            Err(e) => Err(Box::new(e.to_string())),
        }
    }

    /// Sends a transaction that deploys a contract to the blockchain
    pub fn deploy_contract(&mut self, tx: TypedTransaction) -> Result<Transaction, BoxedError> {
        let (
            tx_type,
            sender,
            value,
            bytecode,
            nonce,
            access_list,
            max_priority_fee_per_gas,
            max_fee_per_gas,
        ) = self.get_tx_info(&tx)?;
        let gas = tx.gas().cloned().unwrap_or(self.config.gas_limit);

        let signature = sender.sign_transaction_sync(&tx);
        let tx_hash = keccak256(tx.rlp_signed(self.config.chain_id, &signature));

        match self.evm.deploy(sender.address(), bytecode.clone().into(), value) {
            Ok((retdata, status, gas_used, _logs)) => {
                if U256::from(gas_used) > gas {
                    return Err(Box::new("revert: out-of-gas"))
                }
                if E::is_success(&status) {
                    let transaction = Transaction {
                        hash: tx_hash.into(),
                        nonce,
                        block_hash: None,
                        block_number: None,
                        transaction_index: None,
                        from: sender.address(),
                        to: None,
                        value,
                        input: bytecode.into(),
                        v: signature.v.into(),
                        r: signature.r,
                        s: signature.s,
                        gas,
                        gas_price: None,
                        transaction_type: tx_type,
                        access_list,
                        max_priority_fee_per_gas,
                        max_fee_per_gas,
                        chain_id: Some(self.config.chain_id.as_u64().into()),
                    };
                    Ok(transaction)
                } else {
                    Err(Box::new(dapp_utils::decode_revert(retdata.as_ref()).unwrap_or_default()))
                }
            }
            Err(e) => Err(Box::new(e.to_string())),
        }
    }

    /// Mine a new block
    pub fn mine_block(&mut self) {
        // TODO(rohit): block builder
        // TODO(rohit): consume self.blockchain.pending_txs until block gas limit is reached
        let (block, txs, tx_receipts): (Block<TxHash>, Vec<Transaction>, Vec<TransactionReceipt>) =
            todo!();
        self.insert_block(block);
        for (tx, tx_receipt) in txs.iter().zip(tx_receipts.iter()) {
            self.insert_tx(tx.clone(), tx_receipt.clone());
        }
    }

    fn insert_block(&mut self, block: Block<TxHash>) {
        self.blockchain.blocks_by_hash.insert(
            block.hash.expect("pending block cannot be added"),
            block.number.expect("pending block cannot be added"),
        );
        self.blockchain.blocks.push(block);
    }

    fn insert_tx(&mut self, tx: Transaction, tx_receipt: TransactionReceipt) {
        self.blockchain.txs.insert(tx.hash(), (tx, tx_receipt));
    }
}

async fn handler<E, S>(
    request: Result<Json<JsonRpcRequest>, JsonRejection>,
    Extension(state): Extension<SharedNode<S, E>>,
) -> JsonRpcResponse
where
    E: Evm<S>,
{
    match request {
        Err(_) => Error::INVALID_REQUEST.into(),
        Ok(Json(payload)) => {
            match serde_json::from_str::<EthRequest>(
                &serde_json::to_string(&payload)
                    .expect("deserialized payload should be serializable"),
            ) {
                Ok(msg) => {
                    JsonRpcResponse::new(payload.id(), ResponseContent::success(handle(state, msg)))
                }
                Err(e) => {
                    if e.to_string().contains("unknown variant") {
                        JsonRpcResponse::new(
                            payload.id(),
                            ResponseContent::error(Error::METHOD_NOT_FOUND),
                        )
                    } else {
                        JsonRpcResponse::new(
                            payload.id(),
                            ResponseContent::error(Error::INVALID_PARAMS),
                        )
                    }
                }
            }
        }
    }
}

fn handle<S, E>(state: SharedNode<S, E>, msg: EthRequest) -> EthResponse
where
    E: Evm<S>,
{
    match msg {
        // TODO: think how we can query the EVM state at a past block
        EthRequest::EthGetBalance(account, _block) => {
            EthResponse::EthGetBalance(state.read().unwrap().get_balance(account))
        }
        EthRequest::EthGetTransactionByHash(tx_hash) => {
            EthResponse::EthGetTransactionByHash(state.read().unwrap().get_transaction(tx_hash))
        }
        EthRequest::EthSendTransaction(tx) => {
            let pending_tx = match tx.to() {
                Some(_) => state.write().unwrap().send_transaction(tx),
                None => state.write().unwrap().deploy_contract(tx),
            };

            if let Ok(ref pending_tx) = pending_tx {
                state.write().unwrap().blockchain.insert_pending_tx(pending_tx.clone());
                if state.read().unwrap().config.automine.is_none() {
                    state.write().unwrap().mine_block();
                }
            }

            EthResponse::EthSendTransaction(pending_tx.map(|t| t.hash()))
        }
    }
}
