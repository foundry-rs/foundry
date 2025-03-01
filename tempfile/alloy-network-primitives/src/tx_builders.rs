use core::ops::{Deref, DerefMut};

use alloc::vec::Vec;
use alloy_consensus::BlobTransactionSidecar;
use alloy_eips::eip7702::SignedAuthorization;
use alloy_serde::WithOtherFields;

/// Transaction builder type supporting EIP-4844 transaction fields.
pub trait TransactionBuilder4844: Default + Sized + Send + Sync + 'static {
    /// Get the max fee per blob gas for the transaction.
    fn max_fee_per_blob_gas(&self) -> Option<u128>;

    /// Set the max fee per blob gas  for the transaction.
    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128);

    /// Builder-pattern method for setting max fee per blob gas .
    fn with_max_fee_per_blob_gas(mut self, max_fee_per_blob_gas: u128) -> Self {
        self.set_max_fee_per_blob_gas(max_fee_per_blob_gas);
        self
    }

    /// Gets the EIP-4844 blob sidecar of the transaction.
    fn blob_sidecar(&self) -> Option<&BlobTransactionSidecar>;

    /// Sets the EIP-4844 blob sidecar of the transaction.
    ///
    /// Note: This will also set the versioned blob hashes accordingly:
    /// [BlobTransactionSidecar::versioned_hashes]
    fn set_blob_sidecar(&mut self, sidecar: BlobTransactionSidecar);

    /// Builder-pattern method for setting the EIP-4844 blob sidecar of the transaction.
    fn with_blob_sidecar(mut self, sidecar: BlobTransactionSidecar) -> Self {
        self.set_blob_sidecar(sidecar);
        self
    }
}

/// Transaction builder type supporting EIP-7702 transaction fields.
pub trait TransactionBuilder7702: Default + Sized + Send + Sync + 'static {
    /// Get the EIP-7702 authorization list for the transaction.
    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>>;

    /// Sets the EIP-7702 authorization list.
    fn set_authorization_list(&mut self, authorization_list: Vec<SignedAuthorization>);

    /// Builder-pattern method for setting the authorization list.
    fn with_authorization_list(mut self, authorization_list: Vec<SignedAuthorization>) -> Self {
        self.set_authorization_list(authorization_list);
        self
    }
}

impl<T> TransactionBuilder4844 for WithOtherFields<T>
where
    T: TransactionBuilder4844,
{
    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.deref().max_fee_per_blob_gas()
    }

    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
        self.deref_mut().set_max_fee_per_blob_gas(max_fee_per_blob_gas)
    }

    fn blob_sidecar(&self) -> Option<&BlobTransactionSidecar> {
        self.deref().blob_sidecar()
    }

    fn set_blob_sidecar(&mut self, sidecar: BlobTransactionSidecar) {
        self.deref_mut().set_blob_sidecar(sidecar)
    }
}

impl<T> TransactionBuilder7702 for WithOtherFields<T>
where
    T: TransactionBuilder7702,
{
    fn authorization_list(&self) -> Option<&Vec<SignedAuthorization>> {
        self.deref().authorization_list()
    }

    fn set_authorization_list(&mut self, authorization_list: Vec<SignedAuthorization>) {
        self.deref_mut().set_authorization_list(authorization_list)
    }
}
