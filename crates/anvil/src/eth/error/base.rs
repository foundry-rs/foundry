//! Base-specific error conversions.

use super::{BlockchainError, InvalidTransactionError};
use base_common_evm::BaseTransactionError;
use revm::context_interface::result::{EVMError, InvalidHeader};

impl<T> From<EVMError<T, BaseTransactionError>> for BlockchainError
where
    T: Into<Self>,
{
    fn from(error: EVMError<T, BaseTransactionError>) -> Self {
        match error {
            EVMError::Transaction(error) => match error {
                BaseTransactionError::Base(error) => InvalidTransactionError::from(error).into(),
                BaseTransactionError::DepositSystemTxPostRegolith
                | BaseTransactionError::HaltedDepositPostRegolith => {
                    Self::DepositTransactionUnsupported
                }
                BaseTransactionError::MissingEnvelopedTx => {
                    Self::InvalidTransaction(InvalidTransactionError::MissingEnvelopedTx)
                }
                BaseTransactionError::Eip8130(reason) => Self::Eip8130TransactionRejected(reason),
            },
            EVMError::Header(error) => match error {
                InvalidHeader::ExcessBlobGasNotSet => Self::ExcessBlobGasNotSet,
                InvalidHeader::PrevrandaoNotSet => Self::PrevrandaoNotSet,
            },
            EVMError::Database(error) => error.into(),
            EVMError::Custom(error) => Self::Message(error),
            EVMError::CustomAny(error) => Self::Message(error.to_string()),
        }
    }
}
