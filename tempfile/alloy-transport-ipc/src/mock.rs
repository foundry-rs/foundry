//! Mock IPC server.

use alloy_json_rpc::Response;
use interprocess::local_socket::tokio::prelude::*;
use serde::Serialize;
use std::{collections::VecDeque, path::PathBuf};
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Mock IPC server.
///
/// Currently unix socket only, due to use of namedtempfile.
///
/// ## Example:
///
/// ```
/// use alloy_transport_ipc::MockIpcServer;
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Instantiate a new mock server.
/// let mut server = MockIpcServer::new();
/// // Get the path to the socket.
/// let path = server.path();
/// // Add a reply to the server. Can also use `add_raw_reply` to add a raw
/// // byte vector, or `add_response` to add a json-rpc response.
/// server.add_reply("hello");
/// // Run the server. The first request will get "hello" as a response.
/// MockIpcServer::new().spawn();
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct MockIpcServer {
    /// Replies to send, in order
    replies: VecDeque<Vec<u8>>,
    /// Path to the socket
    path: NamedTempFile,
}

impl Default for MockIpcServer {
    fn default() -> Self {
        Self::new()
    }
}

impl MockIpcServer {
    /// Create a new mock IPC server.
    pub fn new() -> Self {
        Self { replies: VecDeque::new(), path: NamedTempFile::new().unwrap() }
    }

    /// Add a raw reply to the server.
    pub fn add_raw_reply(&mut self, reply: Vec<u8>) {
        self.replies.push_back(reply);
    }

    /// Add a reply to the server.
    pub fn add_reply<S: Serialize>(&mut self, s: S) {
        let reply = serde_json::to_vec(&s).unwrap();
        self.add_raw_reply(reply);
    }

    /// Add a json-rpc response to the server.
    pub fn add_response<S: Serialize>(&mut self, response: Response<S>) {
        self.add_reply(response);
    }

    /// Get the path to the socket.
    pub fn path(&self) -> PathBuf {
        self.path.path().to_owned()
    }

    /// Run the server.
    pub async fn spawn(mut self) {
        tokio::spawn(async move {
            let tmp = self.path.into_temp_path();
            let name = crate::connect::to_name(tmp.as_os_str()).unwrap();
            let socket = LocalSocketStream::connect(name).await.unwrap();

            let (mut reader, mut writer) = socket.split();

            let mut buf = [0u8; 4096];
            loop {
                let _ = reader.read(&mut buf).await.unwrap();
                let reply = self.replies.pop_front().unwrap_or_default();
                writer.write_all(&reply).await.unwrap();
            }
        });
    }
}
