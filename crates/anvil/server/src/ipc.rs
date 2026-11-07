//! IPC handling

use crate::{PubSubRpcHandler, error::RequestError, pubsub::PubSubConnection};
use anvil_rpc::request::Request;
use bytes::{BufMut, BytesMut};
use futures::{Sink, Stream, StreamExt, ready};
use interprocess::local_socket::{self as ls, tokio::prelude::*};
use std::{
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
    /// The path to the socket
    path: String,
}

impl<Handler: PubSubRpcHandler> IpcEndpoint<Handler> {
    /// Creates a new endpoint with the given handler
    pub const fn new(handler: Handler, path: String) -> Self {
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

        let name = to_name(path.as_ref())?;
        let listener = ls::ListenerOptions::new().name(name).create_tokio()?;
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
                        let framed = tokio_util::codec::Decoder::framed(JsonRpcCodec, stream);
                        Some(PubSubConnection::new(IpcConn(framed), handler))
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

/// Where the [`JsonRpcCodec`] is within the JSON text it scans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonLexer {
    /// Outside a string, where brackets change the nesting depth.
    Outside,
    /// Inside a string, which a `"` ends.
    InString,
    /// Inside a string, right after a `\` that escapes the next byte.
    Escaped,
}

// Adapted from <https://github.com/paritytech/jsonrpc/blob/38af3c9439aa75481805edf6c05c6622a5ab1e70/server-utils/src/stream_codec.rs#L47-L105>
impl tokio_util::codec::Decoder for JsonRpcCodec {
    type Item = String;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<Self::Item>> {
        const fn is_whitespace(byte: u8) -> bool {
            matches!(byte, 0x0D | 0x0A | 0x20 | 0x09)
        }

        let mut depth = 0;
        let mut lexer = JsonLexer::Outside;
        let mut start_idx = 0;
        let mut whitespaces = 0;

        for idx in 0..buf.as_ref().len() {
            let byte = buf.as_ref()[idx];
            let mut malformed = false;

            lexer = match lexer {
                JsonLexer::Outside => match byte {
                    b'{' | b'[' => {
                        if depth == 0 {
                            start_idx = idx;
                        }
                        depth += 1;
                        JsonLexer::Outside
                    }
                    b'}' | b']' => {
                        if depth == 0 {
                            malformed = true;
                        } else {
                            depth -= 1;
                        }
                        JsonLexer::Outside
                    }
                    b'"' => JsonLexer::InString,
                    _ => JsonLexer::Outside,
                },
                JsonLexer::InString => match byte {
                    b'"' => JsonLexer::Outside,
                    b'\\' => JsonLexer::Escaped,
                    _ => JsonLexer::InString,
                },
                JsonLexer::Escaped => JsonLexer::InString,
            };
            if is_whitespace(byte) {
                whitespaces += 1;
            }

            if malformed || (depth == 0 && idx != start_idx && idx - start_idx + 1 > whitespaces) {
                let bts = buf.split_to(idx + 1);
                return match String::from_utf8(bts.as_ref().to_vec()) {
                    Ok(val) => Ok(Some(val)),
                    Err(_) => Ok(None),
                };
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

fn to_name(path: &std::ffi::OsStr) -> io::Result<ls::Name<'_>> {
    if cfg!(windows) && !path.as_encoded_bytes().starts_with(br"\\.\pipe\") {
        ls::ToNsName::to_ns_name::<ls::GenericNamespaced>(path)
    } else {
        ls::ToFsName::to_fs_name::<ls::GenericFilePath>(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::codec::Decoder;

    fn decode_all(input: &str) -> Vec<String> {
        let mut buf = BytesMut::from(input);
        let mut out = Vec::new();
        while let Some(msg) = JsonRpcCodec.decode(&mut buf).unwrap() {
            out.push(msg);
        }
        out
    }

    #[test]
    fn splits_messages_ignoring_brackets_and_quotes_in_strings() {
        let first = r#"{"a":"}]{[","b":"\"}","c":"\\"}"#;
        let second = r#" [1, {"d": "x\\\"]"}]"#;
        assert_eq!(decode_all(&format!("{first}{second}\n {{")), [first, second]);
    }

    #[test]
    fn closing_bracket_without_opening_is_returned() {
        assert_eq!(decode_all(" ]{}"), [" ]", "{}"]);
    }
}
