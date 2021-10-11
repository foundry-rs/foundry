use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{rejection::JsonRejection, Extension, Json},
    handler::post,
    AddExtensionLayer, Router, Server,
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
        S: 'static,
        E: Evm<S>,
    {
        Self { evm: Arc::new(evm) }
    }

    pub async fn run<S>(&self)
    where
        S: 'static,
        E: Evm<S>,
    {
        let shared_evm = Arc::clone(&self.evm);

        let svc = Router::new()
            .route("/", post(handler::<E, S>))
            .layer(AddExtensionLayer::new(shared_evm))
            .into_make_service();

        let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

        Server::bind(&addr).serve(svc).await.unwrap();
    }
}

async fn handler<E, S>(
    request: Result<Json<JsonRpcRequest>, JsonRejection>,
    Extension(state): Extension<Arc<E>>,
) -> JsonRpcResponse
where
    S: 'static,
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

fn handle<S, E: Evm<S>>(_evm: Arc<E>, msg: EthRequest) -> EthResponse {
    match msg {
        EthRequest::EthGetBalance(_addr, _block) => {
            todo!();
        }
        EthRequest::EthGetTransactionByHash(_tx_hash) => {
            todo!();
        }
    }
}
