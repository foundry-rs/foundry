use alloy_primitives::{Address, B256, U256};
use alloy_rpc_types::BlockId;
use futures::channel::mpsc::{SendError, TrySendError};
use std::{
    convert::Infallible,
    sync::{mpsc::RecvError, Arc},
};

/// Result alias with `DatabaseError` as error
pub type DatabaseResult<T> = Result<T, DatabaseError>;

/// Errors that can happen when working with [`revm::Database`]
#[derive(Debug, thiserror::Error)]
#[allow(missing_docs)]
pub enum DatabaseError {
    #[error("missing bytecode for code hash {0}")]
    MissingCode(B256),
    #[error(transparent)]
    Recv(#[from] RecvError),
    #[error(transparent)]
    Send(#[from] SendError),
    #[error("failed to get account for {0}: {1}")]
    GetAccount(Address, Arc<eyre::Error>),
    #[error("failed to get storage for {0} at {1}: {2}")]
    GetStorage(Address, U256, Arc<eyre::Error>),
    #[error("failed to get block hash for {0}: {1}")]
    GetBlockHash(u64, Arc<eyre::Error>),
    #[error("failed to get full block for {0:?}: {1}")]
    GetFullBlock(BlockId, Arc<eyre::Error>),
    #[error("block {0:?} does not exist")]
    BlockNotFound(BlockId),
    #[error("failed to get transaction {0}: {1}")]
    GetTransaction(B256, Arc<eyre::Error>),
    #[error("failed to process AnyRequest: {0}")]
    AnyRequest(Arc<eyre::Error>),
}

impl DatabaseError {
    fn get_rpc_error(&self) -> Option<&eyre::Error> {
        match self {
            Self::GetAccount(_, err) => Some(err),
            Self::GetStorage(_, _, err) => Some(err),
            Self::GetBlockHash(_, err) => Some(err),
            Self::GetFullBlock(_, err) => Some(err),
            Self::GetTransaction(_, err) => Some(err),
            Self::AnyRequest(err) => Some(err),
            // Enumerate explicitly to make sure errors are updated if a new one is added.
            Self::MissingCode(_) | Self::Recv(_) | Self::Send(_) | Self::BlockNotFound(_) => None,
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

impl<T> From<TrySendError<T>> for DatabaseError {
    fn from(value: TrySendError<T>) -> Self {
        value.into_send_error().into()
    }
}

impl From<Infallible> for DatabaseError {
    fn from(value: Infallible) -> Self {
        match value {}
    }
}
