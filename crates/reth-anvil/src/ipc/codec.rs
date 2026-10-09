//! Streaming JSON framing from reth-ipc, with recovery after an unmatched closing delimiter.

// Copyright (c) 2015-2017 Parity Technologies Limited
//
// Permission is hereby granted, free of charge, to any
// person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the
// Software without restriction, including without
// limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software
// is furnished to do so, subject to the following
// conditions:
//
// The above copyright notice and this permission notice
// shall be included in all copies or substantial portions
// of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
// ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
// TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
// PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
// SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
// CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
// OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
// IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

// This basis of this file has been taken from the deprecated jsonrpc codebase:
// https://github.com/paritytech/jsonrpc

use alloy_primitives::bytes::{Buf, BufMut, BytesMut};
use std::{io, str};

/// Separator for enveloping messages in streaming codecs.
#[derive(Debug, Clone)]
pub(super) enum Separator {
    /// No envelope is expected between messages. Decoder will try to figure out
    /// message boundaries by accumulating incoming bytes until valid JSON is formed.
    /// Encoder will send messages without any boundaries between requests.
    Empty,
    /// Byte is used as a sentinel between messages.
    Byte(u8),
}

impl Default for Separator {
    fn default() -> Self {
        Self::Byte(b'\n')
    }
}

/// Stream codec for streaming protocols (IPC, TCP).
#[derive(Debug, Default)]
pub(super) struct StreamCodec {
    incoming_separator: Separator,
    outgoing_separator: Separator,
    scan: ScanState,
}

impl StreamCodec {
    /// Default codec with streaming input data. Input can be both enveloped and not.
    pub fn stream_incoming() -> Self {
        Self::new(Separator::Empty, Default::default())
    }

    /// New custom stream codec.
    pub const fn new(incoming_separator: Separator, outgoing_separator: Separator) -> Self {
        Self { incoming_separator, outgoing_separator, scan: ScanState::new() }
    }
}

/// Scan state of the [`Separator::Empty`] decoder, carried across `decode` calls so bytes
/// scanned by a previous call are not scanned again.
///
/// Indices refer to the current read buffer and must be reset whenever bytes are consumed from
/// it.
#[derive(Debug)]
struct ScanState {
    depth: i32,
    in_str: bool,
    is_escaped: bool,
    start_idx: usize,
    whitespaces: usize,
    cursor: usize,
}

impl ScanState {
    const fn new() -> Self {
        Self { depth: 0, in_str: false, is_escaped: false, start_idx: 0, whitespaces: 0, cursor: 0 }
    }
}

impl Default for ScanState {
    fn default() -> Self {
        Self::new()
    }
}

impl tokio_util::codec::Decoder for StreamCodec {
    type Item = String;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<Self::Item>> {
        if let Separator::Byte(separator) = self.incoming_separator {
            if let Some(i) = buf.as_ref().iter().position(|&b| b == separator) {
                let line = buf.split_to(i);
                let _ = buf.split_to(1);

                match str::from_utf8(line.as_ref()) {
                    Ok(s) => Ok(Some(s.to_string())),
                    Err(_) => Err(io::Error::other("invalid UTF-8")),
                }
            } else {
                Ok(None)
            }
        } else {
            // Resume scanning at the byte the previous call stopped at.
            while self.scan.cursor < buf.len() {
                let idx = self.scan.cursor;
                let byte = buf[idx];

                if (byte == b'{' || byte == b'[') && !self.scan.in_str {
                    if self.scan.depth == 0 {
                        self.scan.start_idx = idx;
                        self.scan.whitespaces = 0;
                    }
                    self.scan.depth += 1;
                } else if (byte == b'}' || byte == b']') && !self.scan.in_str {
                    self.scan.depth -= 1;
                } else if byte == b'"' && !self.scan.is_escaped {
                    self.scan.in_str = !self.scan.in_str;
                } else if is_whitespace(byte) {
                    self.scan.whitespaces += 1;
                }
                self.scan.is_escaped = byte == b'\\' && !self.scan.is_escaped && self.scan.in_str;

                if self.scan.depth < 0
                    || (self.scan.depth == 0
                        && idx != self.scan.start_idx
                        && idx - self.scan.start_idx + 1 > self.scan.whitespaces)
                {
                    let start = self.scan.start_idx;
                    let end = idx + 1;
                    // Reset before advancing the buffer because the stored indices go stale.
                    self.scan = ScanState::new();
                    if start > 0 {
                        buf.advance(start);
                    }
                    let bts = buf.split_to(end - start);
                    return Ok(String::from_utf8(bts.into()).ok());
                }

                self.scan.cursor += 1;
            }
            Ok(None)
        }
    }
}

impl tokio_util::codec::Encoder<String> for StreamCodec {
    type Error = io::Error;

    fn encode(&mut self, msg: String, buf: &mut BytesMut) -> io::Result<()> {
        match self.outgoing_separator {
            Separator::Byte(separator) => {
                buf.reserve(msg.len() + 1);
                buf.extend_from_slice(msg.as_bytes());
                buf.put_u8(separator);
            }
            Separator::Empty => buf.extend_from_slice(msg.as_bytes()),
        }
        Ok(())
    }
}

#[inline]
const fn is_whitespace(byte: u8) -> bool {
    matches!(byte, 0x0D | 0x0A | 0x20 | 0x09)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::codec::Decoder;

    #[test]
    fn malformed_frames_preserve_following_requests() {
        let request = r#"{"jsonrpc":"2.0","method":"echo","params":["}\"["],"id":1}"#;
        let mut codec = StreamCodec::stream_incoming();
        let mut input = BytesMut::from(format!("}}]{request}").as_bytes());
        assert_eq!(codec.decode(&mut input).unwrap().as_deref(), Some("}"));
        assert_eq!(codec.decode(&mut input).unwrap().as_deref(), Some("]"));
        assert_eq!(codec.decode(&mut input).unwrap().as_deref(), Some(request));
        assert!(input.is_empty());
    }

    #[test]
    fn leading_whitespace_does_not_delay_a_complete_request() {
        let mut codec = StreamCodec::stream_incoming();
        let mut input = BytesMut::from(&b" \t\r\n{}{}"[..]);
        assert_eq!(codec.decode(&mut input).unwrap().as_deref(), Some("{}"));
        assert_eq!(codec.decode(&mut input).unwrap().as_deref(), Some("{}"));
        assert!(input.is_empty());
    }

    #[test]
    fn split_requests_keep_nested_and_escaped_delimiters() {
        let request = r#"{"jsonrpc":"2.0","method":"echo","params":["}\"["],"id":1}"#;
        for split in 1..request.len() {
            let mut codec = StreamCodec::stream_incoming();
            let mut input = BytesMut::from(&request.as_bytes()[..split]);
            assert!(codec.decode(&mut input).unwrap().is_none());
            input.extend_from_slice(&request.as_bytes()[split..]);
            input.extend_from_slice(b"{}");
            assert_eq!(codec.decode(&mut input).unwrap().as_deref(), Some(request));
            assert_eq!(codec.decode(&mut input).unwrap().as_deref(), Some("{}"));
        }
    }
}
