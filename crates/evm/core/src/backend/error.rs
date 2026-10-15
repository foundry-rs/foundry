use alloy_primitives::Address;
pub use foundry_fork_db::{DatabaseError, DatabaseResult};
use std::convert::Infallible;

pub type BackendResult<T> = Result<T, BackendError>;

/// Errors of [`Backend`](super::Backend) operations.
#[derive(Debug, thiserror::Error)]
#[expect(missing_docs)]
pub enum BackendError {
    #[error("{0}")]
    Message(String),
    #[error("cheatcodes are not enabled for {0}; see `vm.allowCheatcodes(address)`")]
    NoCheats(Address),
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error(transparent)]
    State(#[from] evm2::DatabaseError),
    #[error("failed to fetch account info for {0}")]
    MissingAccount(Address),
    #[error(
        "CREATE2 Deployer (0x4e59b44847b379578588920ca78fbf26c0b4956c) not present on this chain.\n\
         For a production environment, you can deploy it using the pre-signed transaction from \
         https://github.com/Arachnid/deterministic-deployment-proxy.\n\
         For a test environment, you can use `etch` to place the required bytecode at that address."
    )]
    MissingCreate2Deployer,
}

impl BackendError {
    /// Create a new error with a message
    pub fn msg(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }

    /// Create a new error with a message
    pub fn display(msg: impl std::fmt::Display) -> Self {
        Self::Message(msg.to_string())
    }
}

impl From<tokio::task::JoinError> for BackendError {
    fn from(value: tokio::task::JoinError) -> Self {
        Self::display(value)
    }
}

impl From<Infallible> for BackendError {
    fn from(value: Infallible) -> Self {
        match value {}
    }
}
