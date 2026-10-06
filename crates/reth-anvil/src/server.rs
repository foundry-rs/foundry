//! The node's RPC server.
//!
//! The server outlives the reth node behind it, so `anvil_reset` to another fork and
//! `anvil_setChainId` can replace the node while the endpoint, the open connections, and the
//! in-process API keep working. Every method forwards to the module of the current node.

use crate::logging::{LoggingState, NodeInfoLayer};
use eyre::{Result, WrapErr};
use jsonrpsee::{
    RpcModule,
    core::{
        server::{MethodCallback, PendingSubscriptionSink, SubscriptionMessage},
        traits::ToRpcParams,
    },
    server::{ServerBuilder, ServerConfig, ServerHandle, middleware::rpc::RpcServiceBuilder},
    types::{ErrorObjectOwned, Params, Response, ResponsePayload, error::INTERNAL_ERROR_CODE},
};
use parking_lot::RwLock;
use reth_ethereum::rpc::builder::{IpcRpcServiceBuilder, IpcServerBuilder};
use serde_json::value::RawValue;
use std::{net::SocketAddr, sync::Arc};
use tower_http::cors::CorsLayer;

/// The RPC module of the current node, replaced on a relaunch.
pub type SharedModule = Arc<RwLock<RpcModule<()>>>;

const MAX_REQUEST_SIZE: u32 = 15 * 1024 * 1024;
const MAX_RESPONSE_SIZE: u32 = 160 * 1024 * 1024;
const MAX_CONNECTIONS: u32 = 500;

/// The HTTP, WebSocket, and IPC servers.
#[derive(Debug)]
pub struct RpcServer {
    address: SocketAddr,
    http: ServerHandle,
    ipc: Option<ServerHandle>,
}

impl RpcServer {
    /// Starts the servers. The methods are those of the module in `shared`; every call forwards to
    /// whichever module `shared` holds at the time.
    pub async fn start(
        address: SocketAddr,
        ipc_path: Option<String>,
        shared: SharedModule,
        logging: LoggingState,
    ) -> Result<Self> {
        let methods = forwarding_module(&shared.read().clone(), shared.clone())?;
        let config = ServerConfig::builder()
            .max_request_body_size(MAX_REQUEST_SIZE)
            .max_response_body_size(MAX_RESPONSE_SIZE)
            .max_connections(MAX_CONNECTIONS)
            .build();
        let server = ServerBuilder::default()
            .set_config(config)
            .set_http_middleware(tower::ServiceBuilder::new().layer(CorsLayer::permissive()))
            .set_rpc_middleware(RpcServiceBuilder::new().layer(NodeInfoLayer::new(logging.clone())))
            .build(address)
            .await
            .wrap_err_with(|| format!("failed to bind the rpc server to {address}"))?;
        let address = server.local_addr()?;
        let http = server.start(methods.clone());
        let ipc = match ipc_path {
            Some(path) => Some(
                IpcServerBuilder::default()
                    .set_rpc_middleware(
                        IpcRpcServiceBuilder::new().layer(NodeInfoLayer::new(logging)),
                    )
                    .build(path)
                    .start(methods)
                    .await?,
            ),
            None => None,
        };
        Ok(Self { address, http, ipc })
    }

    /// Returns the address the HTTP and WebSocket server listens on.
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// Stops the servers.
    pub fn stop(self) {
        let _ = self.http.stop();
        if let Some(ipc) = self.ipc {
            let _ = ipc.stop();
        }
    }
}

/// Builds a module with the method names of `current` whose handlers forward to `shared`.
///
/// Of the subscriptions, only `eth_subscribe` is forwarded; anvil serves no other.
fn forwarding_module(current: &RpcModule<()>, shared: SharedModule) -> Result<RpcModule<()>> {
    let mut module = RpcModule::new(());
    for name in current.method_names() {
        match current.method(name) {
            Some(MethodCallback::Sync(_) | MethodCallback::Async(_)) => {
                let shared = shared.clone();
                module.register_async_method(name, move |params, _ctx, _extensions| {
                    let shared = shared.clone();
                    async move { forward(&shared, name, params).await }
                })?;
            }
            Some(MethodCallback::Subscription(_)) if name == "eth_subscribe" => {
                let shared = shared.clone();
                module.register_subscription(
                    "eth_subscribe",
                    "eth_subscription",
                    "eth_unsubscribe",
                    move |params, pending, _ctx, _extensions| {
                        let shared = shared.clone();
                        async move { forward_subscription(&shared, params, pending).await }
                    },
                )?;
            }
            _ => {}
        }
    }
    Ok(module)
}

/// Calls `name` on the current module and returns its raw result or error.
async fn forward(
    shared: &SharedModule,
    name: &str,
    params: Params<'static>,
) -> Result<Box<RawValue>, ErrorObjectOwned> {
    let module = shared.read().clone();
    let params = params.as_str().unwrap_or("[]");
    let request = format!(r#"{{"jsonrpc":"2.0","id":0,"method":"{name}","params":{params}}}"#);
    let internal = |error: serde_json::Error| {
        ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, error.to_string(), None::<()>)
    };
    let (response, _notifications) =
        module.raw_json_request(&request, 1).await.map_err(internal)?;
    let response: Response<'_, Box<RawValue>> =
        serde_json::from_str(response.get()).map_err(internal)?;
    match response.payload {
        ResponsePayload::Success(result) => Ok(result.into_owned()),
        ResponsePayload::Error(error) => Err(error.into_owned()),
    }
}

/// Subscribes on the current module and relays its notifications to the sink.
async fn forward_subscription(
    shared: &SharedModule,
    params: Params<'static>,
    pending: PendingSubscriptionSink,
) {
    let module = shared.read().clone();
    let params = RawParams(params.as_str().unwrap_or("[]").to_string());
    let mut subscription = match module.subscribe_unbounded("eth_subscribe", params).await {
        Ok(subscription) => subscription,
        Err(error) => {
            pending
                .reject(ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, error.to_string(), None::<()>))
                .await;
            return;
        }
    };
    let Ok(sink) = pending.accept().await else { return };
    loop {
        tokio::select! {
            item = subscription.next::<Box<RawValue>>() => {
                let Some(Ok((value, _))) = item else { break };
                if sink.send(SubscriptionMessage::from(value)).await.is_err() {
                    break;
                }
            }
            _ = sink.closed() => break,
        }
    }
}

/// Already encoded JSON parameters.
struct RawParams(String);

impl ToRpcParams for RawParams {
    fn to_rpc_params(self) -> Result<Option<Box<RawValue>>, serde_json::Error> {
        RawValue::from_string(self.0).map(Some)
    }
}
