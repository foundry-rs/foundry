use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{rejection::JsonRejection, Extension, Json},
    handler::post,
    AddExtensionLayer, Router, Server,
};
use ethers::{
    core::k256::ecdsa::SigningKey,
    prelude::{Block, Signer, Transaction, TxHash, Wallet, U256},
};
use evm_adapters::Evm;

mod methods;
use methods::{EthRequest, EthResponse};

mod types;
use types::{Error, JsonRpcRequest, JsonRpcResponse, ResponseContent};

pub struct Node<E> {
    evm: Arc<E>,
}

impl<E: Send + Sync + 'static> Node<E> {
    pub fn new<S>(evm: E) -> Self
    where
        E: Evm<S>,
    {
        Self { evm: Arc::new(evm) }
    }

    pub fn init<S>(&mut self, account: Wallet<SigningKey>, balance: U256)
    where
        E: Evm<S>,
    {
        if let Some(evm) = Arc::get_mut(&mut self.evm) {
            evm.set_balance(account.address(), balance);
        }
    }

    pub async fn run<S>(&mut self)
    where
        S: 'static,
        E: Evm<S>,
    {
        let state = Arc::new(State { evm: self.evm.clone(), blocks: vec![], txs: HashMap::new() });

        let svc = Router::new()
            .route("/", post(handler::<E, S>))
            .layer(AddExtensionLayer::new(state))
            .into_make_service();

        let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

        Server::bind(&addr).serve(svc).await.unwrap();
    }
}

#[allow(dead_code)]
struct State<E> {
    evm: Arc<E>,
    blocks: Vec<Block<TxHash>>,
    txs: HashMap<TxHash, Transaction>,
}

async fn handler<E, S>(
    request: Result<Json<JsonRpcRequest>, JsonRejection>,
    Extension(state): Extension<Arc<State<E>>>,
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

fn handle<S, E: Evm<S>>(state: Arc<State<E>>, msg: EthRequest) -> EthResponse {
    match msg {
        EthRequest::EthGetBalance(account, _block) => {
            let balance = state.clone().evm.get_balance(account);
            EthResponse::EthGetBalance(balance)
        }
        EthRequest::EthGetTransactionByHash(_tx_hash) => {
            todo!();
        }
        EthRequest::EthSendTransaction(_tx) => {
            todo!();
        }
    }
}
