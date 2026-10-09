//! IPC transport with recoverable JSON framing and reth's request dispatch.
//!
//! Reth's server fixes its codec internally. Keep its method and batch dispatch here until the
//! server accepts a codec that emits malformed input instead of waiting indefinitely.

use self::{
    codec::StreamCodec,
    request::call_with_service,
    rpc_service::{RpcService, RpcServiceCfg},
};
use crate::{
    history::PruneHistoryLayer,
    logging::{LoggingState, NodeInfoLayer},
    server::SharedModule,
};
use futures::{SinkExt, StreamExt};
use interprocess::local_socket::{
    GenericFilePath, ListenerOptions, ToFsName,
    tokio::prelude::{LocalSocketListener, LocalSocketStream},
    traits::tokio::{Listener, Stream},
};
use jsonrpsee::{
    BoundedSubscriptions, MethodSink, Methods, RpcModule,
    core::TEN_MB_SIZE_BYTES,
    server::{
        ConnectionGuard, ConnectionPermit, RandomIntegerIdProvider, ServerHandle, StopHandle,
        stop_channel,
    },
};
use std::{io, pin::pin, sync::Arc};
use tokio::{io::AsyncWriteExt, sync::mpsc, task::JoinSet};
use tokio_util::codec::Decoder;

mod codec;
mod request;
mod rpc_service;

// Preserve reth-ipc's default connection, subscription, message, and byte limits.
const MAX_CONNECTIONS: usize = 100;
const MAX_SUBSCRIPTIONS: u32 = 1024;
const MESSAGE_BUFFER_CAPACITY: usize = 1024;

/// Starts the IPC listener and stops its connections with the returned server handle.
pub(crate) fn start(
    path: &str,
    methods: RpcModule<()>,
    logging: LoggingState,
    prune_history: Option<Option<usize>>,
    shared: SharedModule,
) -> io::Result<ServerHandle> {
    if cfg!(unix) {
        // Match reth's replacement of a previous endpoint at this path.
        let _ = std::fs::remove_file(path);
    }
    let listener =
        ListenerOptions::new().name(path.to_fs_name::<GenericFilePath>()?).create_tokio()?;
    let methods = methods.into();
    let (stop, handle) = stop_channel();
    tokio::spawn(run(listener, methods, logging, prune_history, shared, stop));
    Ok(handle)
}

async fn run(
    listener: LocalSocketListener,
    methods: Methods,
    logging: LoggingState,
    prune_history: Option<Option<usize>>,
    shared: SharedModule,
    stop: StopHandle,
) {
    let guard = ConnectionGuard::new(MAX_CONNECTIONS);
    let mut connections = JoinSet::new();
    let mut shutdown = pin!(stop.clone().shutdown());
    let mut id = 0u32;
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            Some(result) = connections.join_next(), if !connections.is_empty() => {
                if let Err(error) = result {
                    tracing::debug!(target: "ipc", %error, "IPC connection task stopped");
                }
            }
            result = listener.accept() => {
                let Ok(stream) = result else { continue };
                if let Some(permit) = guard.try_acquire() {
                    connections.spawn(connection(
                        stream, methods.clone(), logging.clone(), prune_history,
                        shared.clone(), id, permit, stop.clone(),
                    ));
                    id = id.wrapping_add(1);
                } else {
                    let (_, mut writer) = stream.split();
                    let _ = writer.write_all(b"Too many connections. Please try again later.").await;
                }
            }
        }
    }
    connections.shutdown().await;
}

#[expect(clippy::too_many_arguments)]
async fn connection(
    stream: LocalSocketStream,
    methods: Methods,
    logging: LoggingState,
    prune_history: Option<Option<usize>>,
    shared: SharedModule,
    id: u32,
    permit: ConnectionPermit,
    stop: StopHandle,
) {
    let _permit = permit;
    let (mut writer, mut reader) = StreamCodec::stream_incoming().framed(stream).split();
    let (tx, mut rx) = mpsc::channel(MESSAGE_BUFFER_CAPACITY);
    let sink = MethodSink::new_with_limit(tx, TEN_MB_SIZE_BYTES);
    let service = tower::ServiceBuilder::new()
        .layer(NodeInfoLayer::new(logging))
        .layer(PruneHistoryLayer::new(prune_history, shared))
        .service(RpcService::new(
            methods,
            TEN_MB_SIZE_BYTES as usize,
            id.into(),
            RpcServiceCfg {
                bounded_subscriptions: BoundedSubscriptions::new(MAX_SUBSCRIPTIONS),
                sink: sink.clone(),
                id_provider: Arc::new(RandomIntegerIdProvider),
            },
        ));
    let write = async move {
        while let Some(response) = rx.recv().await {
            writer.send(String::from(Box::<str>::from(response))).await?;
        }
        Ok::<_, io::Error>(())
    };
    let read = async move {
        let mut calls = JoinSet::new();
        loop {
            tokio::select! {
                Some(_) = calls.join_next(), if !calls.is_empty() => {},
                request = reader.next() => {
                    let Some(Ok(request)) = request else { break };
                    let service = service.clone();
                    let sink = sink.clone();
                    calls.spawn(async move {
                        if let Some(response) = call_with_service(
                            request, service, TEN_MB_SIZE_BYTES as usize, TEN_MB_SIZE_BYTES as usize,
                        ).await {
                            let _ = sink.send(response).await;
                        }
                    });
                }
            }
        }
        calls.shutdown().await;
    };
    // Read and write independently, so a busy subscription does not block request reception.
    tokio::select! {
        _ = stop.shutdown() => {},
        _ = write => {},
        _ = read => {},
    }
}
