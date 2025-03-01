//! Node-related types and constants.

use alloy_primitives::hex;
use std::time::Duration;
use thiserror::Error;

/// How long we will wait for the node to indicate that it is ready.
pub const NODE_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for waiting for the node to add a peer.
pub const NODE_DIAL_LOOP_TIMEOUT: Duration = Duration::from_secs(20);

/// Errors that can occur when working with the node instance.
#[derive(Debug, Error)]
pub enum NodeError {
    /// No stderr was captured from the child process.
    #[error("no stderr was captured from the process")]
    NoStderr,
    /// No stdout was captured from the child process.
    #[error("no stdout was captured from the process")]
    NoStdout,
    /// Timed out waiting for the node to start.
    #[error("timed out waiting for node to spawn; is the node binary installed?")]
    Timeout,
    /// Encountered a fatal error.
    #[error("fatal error: {0}")]
    Fatal(String),
    /// A line could not be read from the node stderr.
    #[error("could not read line from node stderr: {0}")]
    ReadLineError(std::io::Error),

    /// The chain id was not set.
    #[error("the chain ID was not set")]
    ChainIdNotSet,
    /// Could not create the data directory.
    #[error("could not create directory: {0}")]
    CreateDirError(std::io::Error),

    /// Genesis error
    #[error("genesis error occurred: {0}")]
    GenesisError(String),
    /// Node init error
    #[error("node init error occurred")]
    InitError,
    /// Spawn node error
    #[error("could not spawn node: {0}")]
    SpawnError(std::io::Error),
    /// Wait error
    #[error("could not wait for node to exit: {0}")]
    WaitError(std::io::Error),

    /// Clique private key error
    #[error("clique address error: {0}")]
    CliqueAddressError(String),

    /// The private key could not be parsed.
    #[error("could not parse private key")]
    ParsePrivateKeyError,
    /// An error occurred while deserializing a private key.
    #[error("could not deserialize private key from bytes")]
    DeserializePrivateKeyError,
    /// An error occurred while parsing a hex string.
    #[error(transparent)]
    FromHexError(#[from] hex::FromHexError),
    /// No keys available this node instance.
    #[error("no keys available in this node instance")]
    NoKeysAvailable,
}
