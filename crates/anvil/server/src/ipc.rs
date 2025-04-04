//! IPC handling

use crate::{error::RequestError, pubsub::PubSubConnection, PubSubRpcHandler};
use anvil_rpc::request::Request;
use bytes::{BufMut, BytesMut};
use futures::{ready, Sink, Stream, StreamExt};
use futures::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt};
use interprocess::local_socket::tokio::LocalSocketListener;
use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio_util::codec::{FramedRead, FramedWrite};

/// An IPC connection for anvil
///
/// A Future that listens for incoming connections and spawns new connections
pub struct IpcEndpoint<Handler> {
    /// the handler for the websocket connection
    handler: Handler,
    /// The path to the socket
    path: String,
}

impl<Handler: PubSubRpcHandler> IpcEndpoint<Handler> {
    /// Creates a new endpoint with the given handler
    pub fn new(handler: Handler, path: String) -> Self {
        Self { handler, path }
    }

    /// Returns a stream of incoming connection handlers.
    ///
    /// This establishes the IPC endpoint, converts the incoming connections into handled
    /// connections.
    #[instrument(target = "ipc", skip_all)]
    pub fn incoming(self) -> io::Result<impl Stream<Item = impl Future<Output = ()>>> {
        let Self { handler, path } = self;

        trace!(%path, "starting IPC server");

        if cfg!(unix) {
            // ensure the file does not exist
            if std::fs::remove_file(&path).is_ok() {
                warn!(%path, "removed existing file");
            }
        }

        let listener = LocalSocketListener::bind(path)?;
        let connections = futures::stream::unfold(listener, |listener| async move {
            let conn = listener.accept().await;
            Some((conn, listener))
        });

        trace!("established connection listener");

        Ok(connections.filter_map(move |stream| {
            let handler = handler.clone();
            async move {
                match stream {
                    Ok(stream) => {
                        trace!("successful incoming IPC connection");
                        Some(PubSubConnection::new(IpcConn(stream), handler))
                    }
                    Err(err) => {
                        trace!(%err, "unsuccessful incoming IPC connection");
                        None
                    }
                }
            }
        }))
    }
}

struct IpcConn(interprocess::local_socket::tokio::LocalSocketStream);

impl Stream for IpcConn {
    type Item = Result<Option<Request>, RequestError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut buf = BytesMut::new();
        match ready!(Pin::new(&mut self.0).poll_read(cx, &mut buf)) {
            Ok(0) => Poll::Ready(None),
            Ok(_) => {
                let text = String::from_utf8_lossy(&buf).to_string();
                Poll::Ready(Some(Ok(Some(serde_json::from_str(&text)?))))
            }
            Err(e) => Poll::Ready(Some(Err(e.into()))),
        }
    }
}

impl Sink<String> for IpcConn {
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, item: String) -> Result<(), Self::Error> {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(item.as_bytes());
        buf.put_u8(b'\n');
        futures::executor::block_on(self.0.write_all(&buf))?;
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.0).poll_close(cx)
    }
}

struct JsonRpcCodec;

// Adapted from <https://github.com/paritytech/jsonrpc/blob/38af3c9439aa75481805edf6c05c6622a5ab1e70/server-utils/src/stream_codec.rs#L47-L105>
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
        // Add newline character
        buf.put_u8(b'\n');
        Ok(())
    }
}
