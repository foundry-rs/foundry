//! Optimism-specific error conversions for [`BlockchainError`] and
//! [`InvalidTransactionError`].

use super::{BlockchainError, InvalidTransactionError};
use op_revm::OpTransactionError;
use revm::context_interface::result::{EVMError, InvalidHeader};

impl<T> From<EVMError<T, OpTransactionError>> for BlockchainError
where
    T: Into<Self>,
{
    fn from(err: EVMError<T, OpTransactionError>) -> Self {
        match err {
            EVMError::Transaction(err) => match err {
                OpTransactionError::Base(err) => InvalidTransactionError::from(err).into(),
                OpTransactionError::DepositSystemTxPostRegolith => {
                    Self::DepositTransactionUnsupported
                }
                OpTransactionError::HaltedDepositPostRegolith => {
                    Self::DepositTransactionUnsupported
                }
                OpTransactionError::MissingEnvelopedTx => Self::InvalidTransaction(err.into()),
            },
            EVMError::Header(err) => match err {
                InvalidHeader::ExcessBlobGasNotSet => Self::ExcessBlobGasNotSet,
                InvalidHeader::PrevrandaoNotSet => Self::PrevrandaoNotSet,
            },
            EVMError::Database(err) => err.into(),
            EVMError::Custom(err) => Self::Message(err),
            EVMError::CustomAny(err) => Self::Message(err.to_string()),
        }
    }
}

impl<T> From<EVMError<T, alloy_op_evm::OpTxError>> for BlockchainError
where
    T: Into<Self>,
{
    fn from(err: EVMError<T, alloy_op_evm::OpTxError>) -> Self {
        match err {
            EVMError::Transaction(err) => {
                let op_err: OpTransactionError = err.0;
                EVMError::<T, OpTransactionError>::Transaction(op_err).into()
            }
            EVMError::Header(err) => EVMError::<T, OpTransactionError>::Header(err).into(),
            EVMError::Database(err) => err.into(),
            EVMError::Custom(err) => Self::Message(err),
            EVMError::CustomAny(err) => Self::Message(err.to_string()),
        }
    }
}

impl From<OpTransactionError> for InvalidTransactionError {
    fn from(value: OpTransactionError) -> Self {
        match value {
            OpTransactionError::Base(err) => err.into(),
            OpTransactionError::DepositSystemTxPostRegolith
            | OpTransactionError::HaltedDepositPostRegolith => Self::DepositTxErrorPostRegolith,
            OpTransactionError::MissingEnvelopedTx => Self::MissingEnvelopedTx,
        }
    }
}
