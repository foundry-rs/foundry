//! Transaction types.

use crate::Signed;
use alloc::vec::Vec;
use alloy_eips::{eip2930::AccessList, eip7702::SignedAuthorization};
use alloy_primitives::{keccak256, Address, Bytes, ChainId, TxKind, B256, U256};
use core::{any, fmt};

mod eip1559;
pub use eip1559::TxEip1559;

mod eip2930;
pub use eip2930::TxEip2930;

mod eip7702;
pub use eip7702::TxEip7702;

/// [EIP-4844] constants, helpers, and types.
pub mod eip4844;
pub mod pooled;
pub use pooled::PooledTransaction;

use alloy_eips::eip4844::DATA_GAS_PER_BLOB;
pub use alloy_eips::eip4844::{
    builder::{SidecarBuilder, SidecarCoder, SimpleCoder},
    utils as eip4844_utils, Blob, BlobTransactionSidecar, Bytes48,
};
#[cfg(feature = "kzg")]
pub use eip4844::BlobTransactionValidationError;
pub use eip4844::{TxEip4844, TxEip4844Variant, TxEip4844WithSidecar};

mod envelope;
pub use envelope::{TxEnvelope, TxType};

mod legacy;
pub use legacy::{from_eip155_value, to_eip155_value, TxLegacy};

mod rlp;
#[doc(hidden)]
pub use rlp::RlpEcdsaTx;

mod typed;
pub use typed::TypedTransaction;

mod meta;
pub use meta::{TransactionInfo, TransactionMeta};

mod recovered;
pub use recovered::{Recovered, SignerRecoverable};

#[cfg(feature = "serde")]
pub use legacy::signed_legacy_serde;

/// Bincode-compatible serde implementations for transaction types.
#[cfg(all(feature = "serde", feature = "serde-bincode-compat"))]
pub mod serde_bincode_compat {
    pub use super::{
        eip1559::serde_bincode_compat::*, eip2930::serde_bincode_compat::*,
        eip7702::serde_bincode_compat::*, legacy::serde_bincode_compat::*,
    };
}

use alloy_eips::Typed2718;

/// Represents a minimal EVM transaction.
#[doc(alias = "Tx")]
#[auto_impl::auto_impl(&, Arc)]
pub trait Transaction: Typed2718 + fmt::Debug + any::Any + Send + Sync + 'static {
    /// Get `chain_id`.
    fn chain_id(&self) -> Option<ChainId>;

    /// Get `nonce`.
    fn nonce(&self) -> u64;

    /// Get `gas_limit`.
    fn gas_limit(&self) -> u64;

    /// Get `gas_price`.
    fn gas_price(&self) -> Option<u128>;

    /// Returns the EIP-1559 the maximum fee per gas the caller is willing to pay.
    ///
    /// For legacy transactions this is `gas_price`.
    ///
    /// This is also commonly referred to as the "Gas Fee Cap".
    fn max_fee_per_gas(&self) -> u128;

    /// Returns the EIP-1559 Priority fee the caller is paying to the block author.
    ///
    /// This will return `None` for non-EIP1559 transactions
    fn max_priority_fee_per_gas(&self) -> Option<u128>;

    /// Max fee per blob gas for EIP-4844 transaction.
    ///
    /// Returns `None` for non-eip4844 transactions.
    ///
    /// This is also commonly referred to as the "Blob Gas Fee Cap".
    fn max_fee_per_blob_gas(&self) -> Option<u128>;

    /// Return the max priority fee per gas if the transaction is an EIP-1559 transaction, and
    /// otherwise return the gas price.
    ///
    /// # Warning
    ///
    /// This is different than the `max_priority_fee_per_gas` method, which returns `None` for
    /// non-EIP-1559 transactions.
    fn priority_fee_or_price(&self) -> u128;

    /// Returns the effective gas price for the given base fee.
    ///
    /// If the transaction is a legacy or EIP2930 transaction, the gas price is returned.
    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128;

    /// Returns the effective tip for this transaction.
    ///
    /// For EIP-1559 transactions: `min(max_fee_per_gas - base_fee, max_priority_fee_per_gas)`.
    /// For legacy transactions: `gas_price - base_fee`.
    fn effective_tip_per_gas(&self, base_fee: u64) -> Option<u128> {
        let base_fee = base_fee as u128;

        let max_fee_per_gas = self.max_fee_per_gas();

        // Check if max_fee_per_gas is less than base_fee
        if max_fee_per_gas < base_fee {
            return None;
        }

        // Calculate the difference between max_fee_per_gas and base_fee
        let fee = max_fee_per_gas - base_fee;

        // Compare the fee with max_priority_fee_per_gas (or gas price for non-EIP1559 transactions)
        self.max_priority_fee_per_gas()
            .map_or(Some(fee), |priority_fee| Some(fee.min(priority_fee)))
    }

    /// Returns `true` if the transaction supports dynamic fees.
    fn is_dynamic_fee(&self) -> bool;

    /// Returns the transaction kind.
    fn kind(&self) -> TxKind;

    /// Returns true if the transaction is a contract creation.
    /// We don't provide a default implementation via `kind` as it copies the 21-byte
    /// [`TxKind`] for this simple check. A proper implementation shouldn't allocate.
    fn is_create(&self) -> bool;

    /// Get the transaction's address of the contract that will be called, or the address that will
    /// receive the transfer.
    ///
    /// Returns `None` if this is a `CREATE` transaction.
    fn to(&self) -> Option<Address> {
        self.kind().to().copied()
    }

    /// Get `value`.
    fn value(&self) -> U256;

    /// Get `data`.
    fn input(&self) -> &Bytes;

    /// Returns the EIP-2930 `access_list` for the particular transaction type. Returns `None` for
    /// older transaction types.
    fn access_list(&self) -> Option<&AccessList>;

    /// Blob versioned hashes for eip4844 transaction. For previous transaction types this is
    /// `None`.
    fn blob_versioned_hashes(&self) -> Option<&[B256]>;

    /// Returns the number of blobs of this transaction.
    ///
    /// This is convenience function for `len(blob_versioned_hashes)`.
    ///
    /// Returns `None` for non-eip4844 transactions.
    fn blob_count(&self) -> Option<u64> {
        self.blob_versioned_hashes().map(|h| h.len() as u64)
    }

    /// Returns the total gas for all blobs in this transaction.
    ///
    /// Returns `None` for non-eip4844 transactions.
    #[inline]
    fn blob_gas_used(&self) -> Option<u64> {
        // SAFETY: we don't expect u64::MAX / DATA_GAS_PER_BLOB hashes in a single transaction
        self.blob_count().map(|blobs| blobs * DATA_GAS_PER_BLOB)
    }

    /// Returns the [`SignedAuthorization`] list of the transaction.
    ///
    /// Returns `None` if this transaction is not EIP-7702.
    fn authorization_list(&self) -> Option<&[SignedAuthorization]>;

    /// Returns the number of blobs of [`SignedAuthorization`] in this transactions
    ///
    /// This is convenience function for `len(authorization_list)`.
    ///
    /// Returns `None` for non-eip7702 transactions.
    fn authorization_count(&self) -> Option<u64> {
        self.authorization_list().map(|auths| auths.len() as u64)
    }
}

/// A signable transaction.
///
/// A transaction can have multiple signature types. This is usually
/// [`alloy_primitives::PrimitiveSignature`], however, it may be different for future EIP-2718
/// transaction types, or in other networks. For example, in Optimism, the deposit transaction
/// signature is the unit type `()`.
#[doc(alias = "SignableTx", alias = "TxSignable")]
pub trait SignableTransaction<Signature>: Transaction {
    /// Sets `chain_id`.
    ///
    /// Prefer [`set_chain_id_checked`](Self::set_chain_id_checked).
    fn set_chain_id(&mut self, chain_id: ChainId);

    /// Set `chain_id` if it is not already set. Checks that the provided `chain_id` matches the
    /// existing `chain_id` if it is already set, returning `false` if they do not match.
    fn set_chain_id_checked(&mut self, chain_id: ChainId) -> bool {
        match self.chain_id() {
            Some(tx_chain_id) => {
                if tx_chain_id != chain_id {
                    return false;
                }
                self.set_chain_id(chain_id);
            }
            None => {
                self.set_chain_id(chain_id);
            }
        }
        true
    }

    /// RLP-encodes the transaction for signing.
    fn encode_for_signing(&self, out: &mut dyn alloy_rlp::BufMut);

    /// Outputs the length of the signature RLP encoding for the transaction.
    fn payload_len_for_signature(&self) -> usize;

    /// RLP-encodes the transaction for signing it. Used to calculate `signature_hash`.
    ///
    /// See [`SignableTransaction::encode_for_signing`].
    fn encoded_for_signing(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.payload_len_for_signature());
        self.encode_for_signing(&mut buf);
        buf
    }

    /// Calculate the signing hash for the transaction.
    fn signature_hash(&self) -> B256 {
        keccak256(self.encoded_for_signing())
    }

    /// Convert to a signed transaction by adding a signature and computing the
    /// hash.
    fn into_signed(self, signature: Signature) -> Signed<Self, Signature>
    where
        Self: Sized;
}

// TODO: Remove in favor of dyn trait upcasting (TBD, see https://github.com/rust-lang/rust/issues/65991#issuecomment-1903120162)
#[doc(hidden)]
impl<S: 'static> dyn SignableTransaction<S> {
    pub fn __downcast_ref<T: any::Any>(&self) -> Option<&T> {
        if any::Any::type_id(self) == any::TypeId::of::<T>() {
            unsafe { Some(&*(self as *const _ as *const T)) }
        } else {
            None
        }
    }
}

#[cfg(feature = "serde")]
impl<T: Transaction> Transaction for alloy_serde::WithOtherFields<T> {
    #[inline]
    fn chain_id(&self) -> Option<ChainId> {
        self.inner.chain_id()
    }

    #[inline]
    fn nonce(&self) -> u64 {
        self.inner.nonce()
    }

    #[inline]
    fn gas_limit(&self) -> u64 {
        self.inner.gas_limit()
    }

    #[inline]
    fn gas_price(&self) -> Option<u128> {
        self.inner.gas_price()
    }

    #[inline]
    fn max_fee_per_gas(&self) -> u128 {
        self.inner.max_fee_per_gas()
    }

    #[inline]
    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.inner.max_priority_fee_per_gas()
    }

    #[inline]
    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        self.inner.max_fee_per_blob_gas()
    }

    #[inline]
    fn priority_fee_or_price(&self) -> u128 {
        self.inner.priority_fee_or_price()
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        self.inner.effective_gas_price(base_fee)
    }

    #[inline]
    fn is_dynamic_fee(&self) -> bool {
        self.inner.is_dynamic_fee()
    }

    #[inline]
    fn kind(&self) -> TxKind {
        self.inner.kind()
    }

    #[inline]
    fn is_create(&self) -> bool {
        self.inner.is_create()
    }

    #[inline]
    fn value(&self) -> U256 {
        self.inner.value()
    }

    #[inline]
    fn input(&self) -> &Bytes {
        self.inner.input()
    }

    #[inline]
    fn access_list(&self) -> Option<&AccessList> {
        self.inner.access_list()
    }

    #[inline]
    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        self.inner.blob_versioned_hashes()
    }

    #[inline]
    fn authorization_list(&self) -> Option<&[SignedAuthorization]> {
        self.inner.authorization_list()
    }
}
