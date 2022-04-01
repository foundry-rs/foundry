//! Aggregated error type for this module

use crate::eth::pool::transactions::PoolTransaction;
use ethers::types::SignatureError;

pub(crate) type Result<T> = std::result::Result<T, BlockchainError>;

#[derive(thiserror::Error, Debug)]
pub enum BlockchainError {
    #[error(transparent)]
    Pool(#[from] PoolError),
    #[error("No signer available")]
    NoSignerAvailable,
    #[error("Chain Id not available")]
    ChainIdNotAvailable,
    #[error("Invalid Transaction")]
    InvalidTransaction,
    #[error("Transaction data is empty")]
    EmptyRawTransactionData,
    #[error("Failed to decode signed transaction")]
    FailedToDecodeSignedTransaction,
    #[error(transparent)]
    SignatureError(#[from] SignatureError),
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
