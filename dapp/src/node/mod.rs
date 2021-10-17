use std::{
    collections::HashMap,
    marker::PhantomData,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use axum::{
    extract::{rejection::JsonRejection, Extension, Json},
    handler::post,
    AddExtensionLayer, Router, Server,
};
use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::{
        transaction::eip2718::TypedTransaction, Address, Block, NameOrAddress, Signer, Transaction,
        TransactionReceipt, TxHash, Wallet, H256, U256, U64,
    },
    utils::keccak256,
};
use evm_adapters::Evm;

mod methods;
use methods::{BoxedError, EthRequest, EthResponse};

mod types;
use types::{Error, JsonRpcRequest, JsonRpcResponse, ResponseContent};

// TODO: impl builder style for node config
pub struct NodeConfig {
    chain_id: U64,
    genesis_accounts: Vec<Wallet<SigningKey>>,
    genesis_balance: U256,
    accounts: HashMap<Address, Wallet<SigningKey>>,
}

impl NodeConfig {
    pub fn new<U: Into<U64>>(
        chain_id: U,
        genesis_accounts: Vec<Wallet<SigningKey>>,
        balance: U256,
    ) -> Self {
        let mut accounts = HashMap::with_capacity(genesis_accounts.len());
        genesis_accounts.iter().cloned().for_each(|w| {
            accounts.insert(w.address(), w);
        });
        Self { chain_id: chain_id.into(), genesis_accounts, genesis_balance: balance, accounts }
    }
}

pub struct Node<S, E: Evm<S>> {
    evm: E,
    config: NodeConfig,
    blockchain: Blockchain,
    _s: PhantomData<S>,
}

#[derive(Default)]
struct Blockchain {
    blocks_by_number: HashMap<U64, Block<TxHash>>,
    blocks_by_hash: HashMap<H256, Block<TxHash>>,
    txs: HashMap<TxHash, Transaction>,
    tx_receipts: HashMap<TxHash, TransactionReceipt>,
}

type SharedNode<S, E> = Arc<RwLock<Node<S, E>>>;

impl<S, E> Node<S, E>
where
    S: Send + Sync + 'static,
    E: Evm<S> + Send + Sync + 'static,
{
    pub async fn init_and_run(mut evm: E, config: NodeConfig) {
        for account in config.genesis_accounts.iter() {
            evm.set_balance(account.address(), config.genesis_balance);
        }
        let node = Arc::new(RwLock::new(Node {
            evm,
            config,
            blockchain: Blockchain::default(),
            _s: PhantomData,
        }));

        // TODO: spawn a new tokio thread with the auto-miner
        tokio::spawn(async move {
            // TODO: set the interval based on block-time
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                // TODO: mine a new block
            }
        });

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
    pub fn get_balance(&self, account: Address) -> U256 {
        self.evm.get_balance(account)
    }

    pub fn get_transaction(&self, tx_hash: TxHash) -> Option<Transaction> {
        self.blockchain.txs.get(&tx_hash).cloned()
    }

    pub fn get_tx_receipt(&self, tx_hash: TxHash) -> Option<TransactionReceipt> {
        self.blockchain.tx_receipts.get(&tx_hash).cloned()
    }

    pub fn get_block_by_number(&self, number: U64) -> Option<Block<TxHash>> {
        self.blockchain.blocks_by_number.get(&number).cloned()
    }

    pub fn get_block_by_hash(&self, hash: H256) -> Option<Block<TxHash>> {
        self.blockchain.blocks_by_hash.get(&hash).cloned()
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
    pub fn send_transaction(&mut self, tx: TypedTransaction) -> Result<TxHash, BoxedError> {
        let sender = if let Some(from) = tx.from() {
            if let Some(sender) = self.account(*from) {
                sender
            } else {
                unimplemented!("handle: tx.from != node.sender");
            }
        } else {
            self.default_sender()
        };
        let value = *tx.value().unwrap_or(&U256::zero());
        let calldata = match tx.data() {
            Some(data) => data.to_vec(),
            None => vec![],
        };
        let to = tx.to().unwrap();

        let to = match to {
            NameOrAddress::Address(addr) => *addr,
            NameOrAddress::Name(_) => unimplemented!("handle: tx.to is an ENS name"),
        };
        // FIXME(rohit): node.(and node) must need the chainID
        let tx_hash = keccak256(tx.rlp_signed(1, &sender.sign_transaction_sync(&tx)));

        match self.evm.call_raw(sender.address(), to, calldata.into(), value, false) {
            Ok((retdata, status, _gas_used)) => {
                if E::is_success(&status) {
                    Ok(tx_hash.into())
                } else {
                    Err(Box::new(dapp_utils::decode_revert(retdata.as_ref()).unwrap_or_default()))
                }
            }
            Err(e) => Err(Box::new(e.to_string())),
        }
    }

    pub fn deploy_contract(&mut self, tx: TypedTransaction) -> Result<TxHash, BoxedError> {
        let sender = if let Some(from) = tx.from() {
            if let Some(sender) = self.account(*from) {
                sender
            } else {
                unimplemented!("handle: tx.from != node.sender");
            }
        } else {
            self.default_sender()
        };
        let value = *tx.value().unwrap_or(&U256::zero());
        let bytecode = match tx.data() {
            Some(data) => data.to_vec(),
            None => vec![],
        };
        let tx_hash = keccak256(tx.rlp_signed(1, &sender.sign_transaction_sync(&tx)));
        match self.evm.deploy(sender.address(), bytecode.into(), value) {
            Ok((retdata, status, _gas_used)) => {
                if E::is_success(&status) {
                    Ok(tx_hash.into())
                } else {
                    Err(Box::new(dapp_utils::decode_revert(retdata.as_ref()).unwrap_or_default()))
                }
            }
            Err(e) => Err(Box::new(e.to_string())),
        }
    }

    pub fn add_block(&mut self, block: Block<TxHash>) {
        self.blockchain
            .blocks_by_number
            .insert(block.number.expect("pending block cannot be added"), block.clone());
        self.blockchain
            .blocks_by_hash
            .insert(block.hash.expect("pending block cannot be added"), block);
    }

    pub fn add_transaction(&mut self, tx: Transaction) {
        self.blockchain.txs.insert(tx.hash(), tx);
    }

    pub fn add_tx_receipt(&mut self, tx_receipt: TransactionReceipt) {
        self.blockchain.tx_receipts.insert(tx_receipt.transaction_hash, tx_receipt);
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
        EthRequest::EthSendTransaction(tx) => EthResponse::EthSendTransaction(match tx.to() {
            Some(_) => state.write().unwrap().send_transaction(tx),
            None => state.write().unwrap().deploy_contract(tx),
        }),
    }
}
