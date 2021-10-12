use std::{
    collections::HashMap,
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
    prelude::{Address, Signer, Wallet, U256, U64},
};
use evm_adapters::Evm;

mod blockchain;
use blockchain::Blockchain;

mod methods;
use methods::{EthRequest, EthResponse};

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

pub struct Node<E> {
    evm: E,
    config: NodeConfig,
    blockchain: Blockchain,
}

pub type SharedNode<E> = Arc<RwLock<Node<E>>>;

impl<E: Send + Sync + 'static> Node<E> {
    pub async fn init_and_run<S>(mut evm: E, config: NodeConfig)
    where
        E: Evm<S>,
        S: 'static,
    {
        for account in config.genesis_accounts.iter() {
            evm.set_balance(account.address(), config.genesis_balance);
        }
        let shared_node =
            Arc::new(RwLock::new(Node { evm, config, blockchain: Blockchain::default() }));

        let svc = Router::new()
            .route("/", post(handler::<E, S>))
            .layer(AddExtensionLayer::new(shared_node))
            .into_make_service();

        let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

        Server::bind(&addr).serve(svc).await.unwrap();
    }

    pub fn accounts(&self) -> Vec<Wallet<SigningKey>> {
        self.config.accounts.values().cloned().collect()
    }

    pub fn account(&self, address: Address) -> Option<Wallet<SigningKey>> {
        self.config.accounts.get(&address).cloned()
    }

    pub fn default_sender(&self) -> Wallet<SigningKey> {
        self.config
            .accounts
            .values()
            .into_iter()
            .next()
            .cloned()
            .expect("node should have at least one account")
    }
}

async fn handler<E, S>(
    request: Result<Json<JsonRpcRequest>, JsonRejection>,
    Extension(state): Extension<SharedNode<E>>,
) -> JsonRpcResponse
where
    E: Evm<S> + Send + Sync + 'static,
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

fn handle<S, E>(state: SharedNode<E>, msg: EthRequest) -> EthResponse
where
    E: Evm<S> + Send + Sync + 'static,
{
    match msg {
        // TODO: think how we can query the EVM state at a past block
        EthRequest::EthGetBalance(account, _block) => {
            EthResponse::EthGetBalance(blockchain::get_balance(state, account))
        }
        EthRequest::EthGetTransactionByHash(tx_hash) => {
            EthResponse::EthGetTransactionByHash(blockchain::get_transaction(state, tx_hash))
        }
        EthRequest::EthSendTransaction(tx) => match tx.to() {
            Some(_) => blockchain::send_transaction(state, tx),
            None => blockchain::deploy_contract(state, tx),
        },
    }
}
