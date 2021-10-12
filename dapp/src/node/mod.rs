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
    prelude::{Signer, Wallet, U256},
};
use evm_adapters::Evm;

mod helper;
use helper::{SharedState, State};

mod methods;
use methods::{EthRequest, EthResponse};

mod types;
use types::{Error, JsonRpcRequest, JsonRpcResponse, ResponseContent};

pub struct Node<E> {
    evm: Arc<RwLock<E>>,
    sender: Wallet<SigningKey>,
}

impl<E: Send + Sync + 'static> Node<E> {
    pub fn new<S>(evm: E, sender: Wallet<SigningKey>) -> Self
    where
        E: Evm<S>,
    {
        Self { evm: Arc::new(RwLock::new(evm)), sender }
    }

    pub fn init<S>(&mut self, balance: U256)
    where
        E: Evm<S>,
    {
        self.evm.write().unwrap().set_balance(self.sender.address(), balance);
    }

    pub async fn run<S>(&self)
    where
        S: 'static,
        E: Evm<S>,
    {
        let shared_state = Arc::new(RwLock::new(State {
            evm: self.evm.clone(),
            sender: self.sender.clone(),
            blocks: vec![],
            txs: HashMap::new(),
        }));

        let svc = Router::new()
            .route("/", post(handler::<E, S>))
            .layer(AddExtensionLayer::new(shared_state))
            .into_make_service();

        let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

        Server::bind(&addr).serve(svc).await.unwrap();
    }
}

async fn handler<E, S>(
    request: Result<Json<JsonRpcRequest>, JsonRejection>,
    Extension(state): Extension<SharedState<E>>,
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

fn handle<S, E: Evm<S>>(state: SharedState<E>, msg: EthRequest) -> EthResponse {
    match msg {
        // TODO: think how we can query the EVM state at a past block
        EthRequest::EthGetBalance(account, _block) => {
            let balance = state.read().unwrap().evm.read().unwrap().get_balance(account);
            EthResponse::EthGetBalance(balance)
        }
        EthRequest::EthGetTransactionByHash(tx_hash) => {
            let tx = state.read().unwrap().txs.get(&tx_hash).cloned();
            EthResponse::EthGetTransactionByHash(tx)
        }
        EthRequest::EthSendTransaction(tx) => match tx.to() {
            Some(_) => helper::send_transaction(state, tx),
            None => helper::deploy_contract(state, tx),
        },
    }
}
