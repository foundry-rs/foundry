//! Aggregated error type for this module

use crate::eth::pool::transactions::PoolTransaction;

pub(crate) type Result<T> = std::result::Result<T, BlockchainError>;

#[derive(thiserror::Error, Debug)]
pub enum BlockchainError {
    #[error(transparent)]
    Pool(#[from] PoolError),
}

/// Errors that can occur in the transaction pool
#[derive(thiserror::Error, Debug)]
pub enum PoolError {
    #[error("Transaction with cyclic dependent transactions")]
    CyclicTransaction,
    #[error("Invalid transaction")]
    InvalidTransaction(),
    #[error("Tx: [{0:?}] already imported")]
    AlreadyImported(Box<PoolTransaction>),
}
