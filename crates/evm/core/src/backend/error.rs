use alloy_primitives::Address;
use revm::{context_interface::result::EVMError, database::DBErrorMarker};
use std::convert::Infallible;

pub type BackendResult<T> = Result<T, BackendError>;

/// Result alias with [`DatabaseError`] as error.
pub type DatabaseResult<T> = Result<T, DatabaseError>;

/// An error of Foundry's revm databases: a fork cache error, which revm can report.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct DatabaseError(#[from] pub foundry_fork_db::DatabaseError);

impl DatabaseError {
    /// Creates an error for a failed remote request.
    pub const fn any_request(err: std::sync::Arc<eyre::Error>) -> Self {
        Self(foundry_fork_db::DatabaseError::AnyRequest(err))
    }
}

impl DBErrorMarker for DatabaseError {}

impl From<Infallible> for DatabaseError {
    fn from(value: Infallible) -> Self {
        match value {}
    }
}

/// Errors that can happen when working with [`revm::Database`]
#[derive(Debug, thiserror::Error)]
#[expect(missing_docs)]
pub enum BackendError {
    #[error("{0}")]
    Message(String),
    #[error("cheatcodes are not enabled for {0}; see `vm.allowCheatcodes(address)`")]
    NoCheats(Address),
    #[error(transparent)]
    Database(#[from] DatabaseError),
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

// Note: this is mostly necessary to use some revm internals that return an [EVMError]
impl<T: Into<Self>> From<EVMError<T>> for BackendError {
    fn from(err: EVMError<T>) -> Self {
        match err {
            EVMError::Database(err) => err.into(),
            EVMError::Custom(err) => Self::msg(err),
            EVMError::Header(err) => Self::msg(err.to_string()),
            EVMError::Transaction(err) => Self::msg(err.to_string()),
            EVMError::CustomAny(err) => Self::msg(err.to_string()),
        }
    }
}
