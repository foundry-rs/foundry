//! The node's RPC server.
//!
//! The server outlives the reth node behind it, so `anvil_reset` to another fork and
//! `anvil_setChainId` can replace the node while the endpoint, the open connections, and the
//! in-process API keep working. Every method forwards to the module of the current node.

use crate::{
    beacon::BeaconLayer,
    config::NodeConfig,
    history::PruneHistoryLayer,
    logging::{LoggingState, NodeInfoLayer},
};
use eyre::{Result, WrapErr};
use http_body_util::BodyExt;
use hyper::{StatusCode, body::Body as _};
use jsonrpsee::{
    RpcModule,
    core::{
        server::{MethodCallback, PendingSubscriptionSink, SubscriptionMessage},
        traits::ToRpcParams,
    },
    server::{
        HttpBody, HttpRequest, HttpResponse, ServerBuilder, ServerConfig, ServerHandle,
        middleware::rpc::RpcServiceBuilder,
    },
    types::{ErrorObjectOwned, Params, Response, ResponsePayload, error::INTERNAL_ERROR_CODE},
};
use parking_lot::RwLock;
use reth_ethereum::rpc::builder::{IpcRpcServiceBuilder, IpcServerBuilder};
use serde_json::value::RawValue;
use std::{
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tower::{Layer, Service};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

/// The RPC module of the current node, replaced on a relaunch.
pub type SharedModule = Arc<RwLock<RpcModule<()>>>;

const MAX_REQUEST_SIZE: u32 = 15 * 1024 * 1024;
const MAX_RESPONSE_SIZE: u32 = 160 * 1024 * 1024;
const MAX_CONNECTIONS: u32 = 500;

/// The server options of the node config.
#[derive(Clone, Debug)]
pub struct ServerSettings {
    /// The IPC endpoint, if any.
    pub ipc_path: Option<String>,
    /// The origins allowed by CORS: `*` or a comma-separated list.
    pub allow_origin: String,
    /// Whether to send no CORS headers at all.
    pub no_cors: bool,
    /// Whether to lift the request body size limit.
    pub no_request_size_limit: bool,
    /// `--prune-history`: the number of past states kept, if any, when pruning.
    pub prune_history: Option<Option<usize>>,
}

impl ServerSettings {
    /// Reads the settings from the node config.
    pub fn from_config(config: &NodeConfig) -> Self {
        Self {
            ipc_path: config.ipc_path.clone(),
            allow_origin: config.allow_origin.clone(),
            no_cors: config.no_cors,
            no_request_size_limit: config.no_request_size_limit,
            prune_history: config.prune_history,
        }
    }

    /// Returns the CORS layer, if CORS is on.
    fn cors(&self) -> Result<Option<CorsLayer>> {
        if self.no_cors {
            return Ok(None);
        }
        let origin = if self.allow_origin.trim() == "*" {
            AllowOrigin::any()
        } else {
            let origins = self
                .allow_origin
                .split(',')
                .map(|origin| origin.trim().parse())
                .collect::<Result<Vec<_>, _>>()
                .wrap_err("invalid --allow-origin")?;
            AllowOrigin::list(origins)
        };
        Ok(Some(CorsLayer::new().allow_origin(origin).allow_methods(Any).allow_headers(Any)))
    }
}

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
        settings: ServerSettings,
        shared: SharedModule,
        logging: LoggingState,
    ) -> Result<Self> {
        let methods = forwarding_module(&shared.read().clone(), shared.clone())?;
        let config = ServerConfig::builder()
            .max_request_body_size(if settings.no_request_size_limit {
                u32::MAX
            } else {
                MAX_REQUEST_SIZE
            })
            .max_response_body_size(MAX_RESPONSE_SIZE)
            .max_connections(MAX_CONNECTIONS)
            .build();
        let server = ServerBuilder::default()
            .set_config(config)
            .set_http_middleware(
                tower::ServiceBuilder::new()
                    .option_layer(settings.cors()?)
                    .layer(NotificationLayer)
                    .layer(BeaconLayer::new(shared.clone())),
            )
            .set_rpc_middleware(
                RpcServiceBuilder::new()
                    .layer(NodeInfoLayer::new(logging.clone()))
                    .layer(PruneHistoryLayer::new(settings.prune_history, shared.clone())),
            )
            .build(address)
            .await
            .wrap_err_with(|| format!("failed to bind the rpc server to {address}"))?;
        let address = server.local_addr()?;
        let http = server.start(methods.clone());
        let ipc = match settings.ipc_path {
            Some(path) => Some(
                IpcServerBuilder::default()
                    .set_rpc_middleware(
                        IpcRpcServiceBuilder::new()
                            .layer(NodeInfoLayer::new(logging))
                            .layer(PruneHistoryLayer::new(settings.prune_history, shared.clone())),
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

/// Answers a JSON-RPC notification with `204 No Content`, as anvil does; jsonrpsee answers it
/// with `200 OK` and a bare `null`.
#[derive(Clone, Copy, Debug)]
pub struct NotificationLayer;

impl<S> Layer<S> for NotificationLayer {
    type Service = NotificationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        NotificationService { inner }
    }
}

/// The service [`NotificationLayer`] wraps the JSON-RPC server with.
#[derive(Clone, Debug)]
pub struct NotificationService<S> {
    inner: S,
}

impl<S, B> Service<HttpRequest<B>> for NotificationService<S>
where
    S: Service<HttpRequest<B>, Response = HttpResponse>,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = HttpResponse;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: HttpRequest<B>) -> Self::Future {
        let response = self.inner.call(request);
        Box::pin(async move {
            let response = response.await?;
            // jsonrpsee answers a notification with a bare `null`; a call always answers with a
            // response object, so a longer body passes through unread.
            if response.status() != StatusCode::OK
                || response.body().size_hint().exact().is_some_and(|size| size > 4)
            {
                return Ok(response);
            }
            let (mut parts, body) = response.into_parts();
            let Ok(collected) = body.collect().await else {
                return Ok(HttpResponse::from_parts(parts, HttpBody::empty()));
            };
            let bytes = collected.to_bytes();
            if bytes.is_empty() || bytes.as_ref() == b"null" {
                parts.status = StatusCode::NO_CONTENT;
                parts.headers.remove(hyper::header::CONTENT_TYPE);
                return Ok(HttpResponse::from_parts(parts, HttpBody::empty()));
            }
            Ok(HttpResponse::from_parts(parts, HttpBody::from(bytes.to_vec())))
        })
    }
}
