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
                        let framed =
                            tokio_util::codec::Decoder::framed(JsonRpcCodec::default(), stream);
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

/// Frames JSON-RPC messages by their brackets, remembering how far it has scanned.
#[derive(Default)]
struct JsonRpcCodec {
    /// Where the scan of the buffered frame stands, so bytes are scanned once as they arrive.
    scan: FrameScan,
}

/// Where the [`JsonRpcCodec`] is within the JSON text it scans.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum JsonLexer {
    /// Outside a string, where brackets change the nesting depth.
    #[default]
    Outside,
    /// Inside a string, which a `"` ends.
    InString,
    /// Inside a string, right after a `\` that escapes the next byte.
    Escaped,
}

/// The progress of the [`JsonRpcCodec`] through the frame at the front of its buffer.
#[derive(Debug, Default)]
struct FrameScan {
    /// Index of the next byte to scan.
    next_idx: usize,
    /// Bracket nesting depth.
    depth: usize,
    /// Whether the next byte is inside a string.
    lexer: JsonLexer,
    /// Index of the frame's first bracket.
    start_idx: usize,
    /// Whitespace bytes in the frame.
    whitespaces: usize,
}

impl FrameScan {
    /// Scans the bytes not scanned yet and returns the index of the frame's last byte, once the
    /// frame is complete.
    fn advance(&mut self, bytes: &[u8]) -> Option<usize> {
        const fn is_whitespace(byte: u8) -> bool {
            matches!(byte, 0x0D | 0x0A | 0x20 | 0x09)
        }

        while let Some(&byte) = bytes.get(self.next_idx) {
            let idx = self.next_idx;
            self.next_idx += 1;
            let mut malformed = false;

            self.lexer = match self.lexer {
                JsonLexer::Outside => match byte {
                    b'{' | b'[' => {
                        if self.depth == 0 {
                            // Whitespace before the frame's first bracket is not part of it.
                            self.start_idx = idx;
                            self.whitespaces = 0;
                        }
                        self.depth += 1;
                        JsonLexer::Outside
                    }
                    b'}' | b']' => {
                        if self.depth == 0 {
                            malformed = true;
                        } else {
                            self.depth -= 1;
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
                self.whitespaces += 1;
            }

            if malformed
                || (self.depth == 0
                    && idx != self.start_idx
                    && idx - self.start_idx + 1 > self.whitespaces)
            {
                return Some(idx);
            }
        }
        None
    }
}

// Adapted from <https://github.com/paritytech/jsonrpc/blob/38af3c9439aa75481805edf6c05c6622a5ab1e70/server-utils/src/stream_codec.rs#L47-L105>
impl tokio_util::codec::Decoder for JsonRpcCodec {
    type Item = String;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<Self::Item>> {
        loop {
            let Some(end) = self.scan.advance(buf.as_ref()) else { return Ok(None) };
            self.scan = FrameScan::default();
            let bts = buf.split_to(end + 1);
            // Drop a frame that is not UTF-8 and frame what follows it, which would otherwise
            // wait in the buffer until the peer sends more bytes.
            if let Ok(val) = String::from_utf8(bts.as_ref().to_vec()) {
                return Ok(Some(val));
            }
        }
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
        let mut codec = JsonRpcCodec::default();
        let mut out = Vec::new();
        while let Some(msg) = codec.decode(&mut buf).unwrap() {
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

    #[test]
    fn leading_whitespace_does_not_hide_short_message() {
        // An empty batch after a CRLF-terminated message must not swallow the next request.
        let next = r#"{"id":1}"#;
        assert_eq!(decode_all(&format!("\r\n[]\r\n{next}")), ["\r\n[]", &format!("\r\n{next}")]);
    }

    #[test]
    fn invalid_utf8_message_does_not_stall_the_next_one() {
        let mut buf = BytesMut::from(&b"{\"\xff\"}{\"id\":1}"[..]);
        assert_eq!(
            JsonRpcCodec::default().decode(&mut buf).unwrap().as_deref(),
            Some(r#"{"id":1}"#)
        );
        assert!(buf.is_empty());
    }

    #[test]
    fn frames_messages_fed_byte_by_byte() {
        let first = r#"{"a":"}]{[","b":"\"}","c":"\\"}"#;
        let second = r#" [1, {"d": "x\\\"]"}]"#;
        let input = format!("{first}\r\n[]{second}\n {{");
        let mut codec = JsonRpcCodec::default();
        let mut buf = BytesMut::new();
        let mut out = Vec::new();
        for &byte in input.as_bytes() {
            buf.extend_from_slice(&[byte]);
            out.extend(codec.decode(&mut buf).unwrap());
        }
        assert_eq!(out, [first, "\r\n[]", second]);
        assert_eq!(&buf[..], b"\n {");
    }
}
