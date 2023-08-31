use ethers::types::BlockId;
use alloy_primitives::{Address, B256, U256};
use foundry_utils::error::SolError;
use futures::channel::mpsc::{SendError, TrySendError};
use std::{
    convert::Infallible,
    fmt,
    sync::{mpsc::RecvError, Arc},
};

/// Result alias with `DatabaseError` as error
pub type DatabaseResult<T> = Result<T, DatabaseError>;

/// Errors that can happen when working with [`revm::Database`]
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("Failed to fetch AccountInfo {0:?}")]
    MissingAccount(Address),
    #[error("Could should already be loaded: {0:?}")]
    MissingCode(B256),
    #[error(transparent)]
    Recv(#[from] RecvError),
    #[error(transparent)]
    Send(#[from] SendError),
    #[error("{0}")]
    Message(String),
    #[error("Failed to get account for {0:?}: {0:?}")]
    GetAccount(Address, Arc<eyre::Error>),
    #[error("Failed to get storage for {0:?} at {1:?}: {2:?}")]
    GetStorage(Address, U256, Arc<eyre::Error>),
    #[error("Failed to get block hash for {0}: {1:?}")]
    GetBlockHash(u64, Arc<eyre::Error>),
    #[error("Failed to get full block for {0:?}: {1:?}")]
    GetFullBlock(BlockId, Arc<eyre::Error>),
    #[error("Block {0:?} does not exist")]
    BlockNotFound(BlockId),
    #[error("Failed to get transaction {0:?}: {1:?}")]
    GetTransaction(B256, Arc<eyre::Error>),
    #[error("Transaction {0:?} not found")]
    TransactionNotFound(B256),
    #[error(
        "CREATE2 Deployer not present on this chain. [0x4e59b44847b379578588920ca78fbf26c0b4956c]"
    )]
    MissingCreate2Deployer,
    #[error(transparent)]
    JoinError(#[from] tokio::task::JoinError),
}

impl DatabaseError {
    /// Create a new error with a message
    pub fn msg(msg: impl Into<String>) -> Self {
        DatabaseError::Message(msg.into())
    }

    fn get_rpc_error(&self) -> Option<&eyre::Error> {
        match self {
            Self::GetAccount(_, err) => Some(err),
            Self::GetStorage(_, _, err) => Some(err),
            Self::GetBlockHash(_, err) => Some(err),
            Self::GetFullBlock(_, err) => Some(err),
            Self::GetTransaction(_, err) => Some(err),
            // Enumerate explicitly to make sure errors are updated if a new one is added.
            Self::MissingAccount(_) |
            Self::MissingCode(_) |
            Self::Recv(_) |
            Self::Send(_) |
            Self::Message(_) |
            Self::BlockNotFound(_) |
            Self::TransactionNotFound(_) |
            Self::MissingCreate2Deployer |
            Self::JoinError(_) => None,
        }
    }

    /// Whether the error is potentially caused by the user forking from an older block in a
    /// non-archive node.
    pub fn is_possibly_non_archive_node_error(&self) -> bool {
        static GETH_MESSAGE: &str = "missing trie node";

        self.get_rpc_error()
            .map(|err| err.to_string().to_lowercase().contains(GETH_MESSAGE))
            .unwrap_or(false)
    }
}

impl SolError for DatabaseError {}

impl<T> From<TrySendError<T>> for DatabaseError {
    fn from(err: TrySendError<T>) -> Self {
        err.into_send_error().into()
    }
}

impl From<Infallible> for DatabaseError {
    fn from(never: Infallible) -> Self {
        match never {}
    }
}

/// Error thrown when the address is not allowed to execute cheatcodes
///
/// See also [`DatabaseExt`](crate::executor::DatabaseExt)
#[derive(Debug, Clone, Copy)]
pub struct NoCheatcodeAccessError(pub Address);

impl fmt::Display for NoCheatcodeAccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "No cheatcode access granted for: {:?}, see `vm.allowCheatcodes()`", self.0)
    }
}

impl std::error::Error for NoCheatcodeAccessError {}

impl SolError for NoCheatcodeAccessError {}
