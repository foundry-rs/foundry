use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{rejection::JsonRejection, Json},
    handler::post,
    AddExtensionLayer, Router, Server,
};
use evm_adapters::Evm;

mod methods;
use methods::JsonRpcMethods;

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

    pub async fn run(&self) {
        let shared_evm = Arc::clone(&self.evm);

        let svc = Router::new()
            .route("/", post(handler))
            .layer(AddExtensionLayer::new(shared_evm))
            .into_make_service();

        let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

        Server::bind(&addr).serve(svc).await.unwrap();
    }
}

async fn handler(request: Result<Json<JsonRpcRequest>, JsonRejection>) -> JsonRpcResponse {
    match request {
        Err(_) => Error::INVALID_REQUEST.into(),
        Ok(Json(payload)) => {
            match serde_json::from_str::<JsonRpcMethods>(
                &serde_json::to_string(&payload)
                    .expect("deserialized payload should be serializable"),
            ) {
                Ok(_m) => JsonRpcResponse::new(payload.id(), ResponseContent::success("passed")),
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
