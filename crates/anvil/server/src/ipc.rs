//! IPC handling

use crate::{error::RequestError, pubsub::PubSubConnection, PubSubRpcHandler};
use anvil_rpc::request::Request;
use bytes::BytesMut;
use futures::{ready, Sink, Stream, StreamExt};
use parity_tokio_ipc::Endpoint;
use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};

/// An IPC connection for anvil
///
/// A Future that listens for incoming connections and spawns new connections
pub struct IpcEndpoint<Handler> {
    /// the handler for the websocket connection
    handler: Handler,
    /// The endpoint we listen for incoming transactions
    endpoint: Endpoint,
}

impl<Handler: PubSubRpcHandler> IpcEndpoint<Handler> {
    /// Creates a new endpoint with the given handler
    pub fn new(handler: Handler, endpoint: impl Into<String>) -> Self {
        Self { handler, endpoint: Endpoint::new(endpoint.into()) }
    }

    /// Returns a stream of incoming connection handlers
    ///
    /// This establishes the ipc endpoint, converts the incoming connections into handled eth
    /// connections, See [`PubSubConnection`] that should be spawned
    #[instrument(target = "ipc", skip_all)]
    pub fn incoming(self) -> io::Result<impl Stream<Item = impl Future<Output = ()>>> {
        let IpcEndpoint { handler, endpoint } = self;
        trace!(endpoint=?endpoint.path(), "starting IPC server" );

        if cfg!(unix) {
            // ensure the file does not exist
            if std::fs::remove_file(endpoint.path()).is_ok() {
                warn!(endpoint=?endpoint.path(), "removed existing file");
            }
        }

        let connections = match endpoint.incoming() {
            Ok(connections) => connections,
            Err(err) => {
                error!(%err, "Failed to create IPC listener");
                return Err(err)
            }
        };

        trace!("established connection listener");

        let connections = connections.filter_map(move |stream| {
            let handler = handler.clone();
            Box::pin(async move {
                if let Ok(stream) = stream {
                    trace!("successful incoming IPC connection");
                    let framed = tokio_util::codec::Decoder::framed(JsonRpcCodec, stream);
                    Some(PubSubConnection::new(IpcConn(framed), handler))
                } else {
                    None
                }
            })
        });
        Ok(connections)
    }
}

#[pin_project::pin_project]
struct IpcConn<T>(#[pin] T);

impl<T> Stream for IpcConn<T>
where
    T: Stream<Item = io::Result<String>>,
{
    type Item = Result<Option<Request>, RequestError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        fn on_request(msg: io::Result<String>) -> Result<Option<Request>, RequestError> {
            let text = msg?;
            Ok(Some(serde_json::from_str(&text)?))
        }
        match ready!(self.project().0.poll_next(cx)) {
            Some(req) => Poll::Ready(Some(on_request(req))),
            _ => Poll::Ready(None),
        }
    }
}

impl<T> Sink<String> for IpcConn<T>
where
    T: Sink<String, Error = io::Error>,
{
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // NOTE: we always flush here this prevents any backpressure buffer in the underlying
        // `Framed` impl that would cause stalled requests
        self.project().0.poll_flush(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: String) -> Result<(), Self::Error> {
        self.project().0.start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().0.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().0.poll_close(cx)
    }
}

struct JsonRpcCodec;

// Adapted from <https://github.dev/paritytech/jsonrpc/blob/38af3c9439aa75481805edf6c05c6622a5ab1e70/server-utils/src/stream_codec.rs#L47-L105>
impl tokio_util::codec::Decoder for JsonRpcCodec {
    type Item = String;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<Self::Item>> {
        const fn is_whitespace(byte: u8) -> bool {
            matches!(byte, 0x0D | 0x0A | 0x20 | 0x09)
        }

        let mut depth = 0;
        let mut in_str = false;
        let mut is_escaped = false;
        let mut start_idx = 0;
        let mut whitespaces = 0;

        for idx in 0..buf.as_ref().len() {
            let byte = buf.as_ref()[idx];

            if (byte == b'{' || byte == b'[') && !in_str {
                if depth == 0 {
                    start_idx = idx;
                }
                depth += 1;
            } else if (byte == b'}' || byte == b']') && !in_str {
                depth -= 1;
            } else if byte == b'"' && !is_escaped {
                in_str = !in_str;
            } else if is_whitespace(byte) {
                whitespaces += 1;
            }
            is_escaped = byte == b'\\' && !is_escaped && in_str;

            if depth == 0 && idx != start_idx && idx - start_idx + 1 > whitespaces {
                let bts = buf.split_to(idx + 1);
                return match String::from_utf8(bts.as_ref().to_vec()) {
                    Ok(val) => Ok(Some(val)),
                    Err(_) => Ok(None),
                }
            }
        }
        Ok(None)
    }
}

impl tokio_util::codec::Encoder<String> for JsonRpcCodec {
    type Error = io::Error;

    fn encode(&mut self, msg: String, buf: &mut BytesMut) -> io::Result<()> {
        buf.extend_from_slice(msg.as_bytes());
        Ok(())
    }
}
