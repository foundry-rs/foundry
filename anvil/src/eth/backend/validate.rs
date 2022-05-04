//! Support for validating transactions at certain stages

use crate::eth::error::InvalidTransactionError;
use anvil_core::eth::transaction::PendingTransaction;
use foundry_evm::revm::{AccountInfo, Env};

/// A trait for validating transactions
#[auto_impl::auto_impl(&, Box)]
pub trait TransactionValidator {
    /// Validates the transaction's validity when it comes to nonce, payment
    ///
    /// This is intended to be checked before the transaction makes it into the pool and whether it
    /// should rather be outright rejected if the sender has insufficient funds.
    fn validate_pool_transaction(
        &self,
        tx: &PendingTransaction,
    ) -> Result<(), InvalidTransactionError>;

    /// Validates the transaction against a specific account before entering the pool
    fn validate_pool_transaction_for(
        &self,
        tx: &PendingTransaction,
        account: &AccountInfo,
        env: &Env,
    ) -> Result<(), InvalidTransactionError>;

    /// Validates the transaction against a specific account
    ///
    /// This should succeed if the transaction is ready to be executed
    fn validate_for(
        &self,
        tx: &PendingTransaction,
        account: &AccountInfo,
        env: &Env,
    ) -> Result<(), InvalidTransactionError>;
}
