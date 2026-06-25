//! Transaction related types
use alloy_consensus::{
    BlobTransactionSidecarVariant, Transaction, Typed2718,
    crypto::RecoveryError,
    transaction::{SignerRecoverable, TxHashRef},
};

use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{Address, B256, Bytes, TxHash};
use alloy_rlp::{Decodable, Encodable};
use bytes::BufMut;
use foundry_evm::traces::CallTraceNode;
use foundry_primitives::FoundryTxEnvelope;
use revm::interpreter::InstructionResult;
use serde::{Deserialize, Serialize};
use std::ops::Deref;

/// A wrapper for a transaction envelope that allows impersonating accounts.
///
/// This is a helper that carries the `impersonated` sender so that the right hash
/// can be created.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaybeImpersonatedTransaction<T> {
    transaction: T,
    impersonated_sender: Option<Address>,
}

impl<T: Typed2718> Typed2718 for MaybeImpersonatedTransaction<T> {
    fn ty(&self) -> u8 {
        self.transaction.ty()
    }
}

impl<T> MaybeImpersonatedTransaction<T> {
    /// Creates a new wrapper for the given transaction
    pub const fn new(transaction: T) -> Self {
        Self { transaction, impersonated_sender: None }
    }

    /// Creates a new impersonated transaction wrapper using the given sender
    pub const fn impersonated(transaction: T, impersonated_sender: Address) -> Self {
        Self { transaction, impersonated_sender: Some(impersonated_sender) }
    }

    /// Returns whether the transaction is impersonated
    pub const fn is_impersonated(&self) -> bool {
        self.impersonated_sender.is_some()
    }

    /// Returns the inner transaction.
    pub fn into_inner(self) -> T {
        self.transaction
    }
}

impl MaybeImpersonatedTransaction<FoundryTxEnvelope> {
    /// Removes the blob sidecar from the wrapped transaction so it can be included in a block
    /// body in its canonical (non-pooled) form, returning the sidecar, if any.
    ///
    /// Impersonated transactions are left untouched: their synthetic hash is derived from the
    /// encoded transaction (see [`Self::hash`]), so stripping the sidecar would change the hash
    /// users received at submission, and changing the hash derivation would break loading state
    /// files written by earlier versions (which key mined transactions by the stored synthetic
    /// hash). Their sidecar is still returned so callers can index it.
    pub fn strip_blob_sidecar(self) -> (Self, Option<BlobTransactionSidecarVariant>) {
        let Self { transaction, impersonated_sender } = self;
        if impersonated_sender.is_some() {
            let sidecar = transaction.sidecar().map(|tx| tx.sidecar.clone());
            (Self { transaction, impersonated_sender }, sidecar)
        } else {
            let (transaction, sidecar) = transaction.strip_blob_sidecar();
            (Self { transaction, impersonated_sender }, sidecar)
        }
    }

    /// Reattaches a blob sidecar to the wrapped transaction, returning the pooled (sidecarful)
    /// form. No-op for non-EIP-4844 transactions and transactions that already carry a sidecar,
    /// so the hash is unchanged in all cases (see [`FoundryTxEnvelope::with_blob_sidecar`]).
    pub fn with_blob_sidecar(self, sidecar: BlobTransactionSidecarVariant) -> Self {
        let Self { transaction, impersonated_sender } = self;
        Self { transaction: transaction.with_blob_sidecar(sidecar), impersonated_sender }
    }
}

impl<T: SignerRecoverable + TxHashRef + Encodable> MaybeImpersonatedTransaction<T> {
    /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, RecoveryError> {
        if let Some(sender) = self.impersonated_sender {
            return Ok(sender);
        }
        self.transaction.recover_signer()
    }

    /// Returns the hash of the transaction.
    ///
    /// If the transaction is impersonated, returns a unique hash derived by appending the
    /// impersonated sender address to the encoded transaction before hashing.
    pub fn hash(&self) -> B256 {
        if let Some(sender) = self.impersonated_sender {
            let mut buffer = Vec::new();
            self.transaction.encode(&mut buffer);
            buffer.extend_from_slice(sender.as_ref());
            return B256::from_slice(alloy_primitives::utils::keccak256(&buffer).as_slice());
        }
        *self.transaction.tx_hash()
    }
}

impl<T: Encodable2718> Encodable2718 for MaybeImpersonatedTransaction<T> {
    fn encode_2718_len(&self) -> usize {
        self.transaction.encode_2718_len()
    }

    fn encode_2718(&self, out: &mut dyn BufMut) {
        self.transaction.encode_2718(out)
    }
}

impl<T: Encodable> Encodable for MaybeImpersonatedTransaction<T> {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        self.transaction.encode(out)
    }
}

impl From<MaybeImpersonatedTransaction<Self>> for FoundryTxEnvelope {
    fn from(value: MaybeImpersonatedTransaction<Self>) -> Self {
        value.transaction
    }
}

impl<T> From<T> for MaybeImpersonatedTransaction<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: Decodable> Decodable for MaybeImpersonatedTransaction<T> {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        T::decode(buf).map(Self::new)
    }
}

impl<T> AsRef<T> for MaybeImpersonatedTransaction<T> {
    fn as_ref(&self) -> &T {
        &self.transaction
    }
}

impl<T> Deref for MaybeImpersonatedTransaction<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.transaction
    }
}

/// Queued transaction
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingTransaction<T> {
    /// The actual transaction
    pub transaction: MaybeImpersonatedTransaction<T>,
    /// the recovered sender of this transaction
    sender: Address,
    /// hash of `transaction`, so it can easily be reused with encoding and hashing again
    hash: TxHash,
}

impl<T> PendingTransaction<T> {
    pub const fn hash(&self) -> &TxHash {
        &self.hash
    }

    pub const fn sender(&self) -> &Address {
        &self.sender
    }
}

impl<T: SignerRecoverable + TxHashRef + Encodable> PendingTransaction<T> {
    pub fn new(transaction: T) -> Result<Self, RecoveryError> {
        let transaction = MaybeImpersonatedTransaction::new(transaction);
        let sender = transaction.recover()?;
        let hash = transaction.hash();
        Ok(Self { transaction, sender, hash })
    }

    pub fn with_impersonated(transaction: T, sender: Address) -> Self {
        let transaction = MaybeImpersonatedTransaction::impersonated(transaction, sender);
        let hash = transaction.hash();
        Self { transaction, sender, hash }
    }

    /// Converts a [`MaybeImpersonatedTransaction`] into a [`PendingTransaction`].
    pub fn from_maybe_impersonated(
        transaction: MaybeImpersonatedTransaction<T>,
    ) -> Result<Self, RecoveryError> {
        if let Some(impersonated) = transaction.impersonated_sender {
            Ok(Self::with_impersonated(transaction.transaction, impersonated))
        } else {
            Self::new(transaction.transaction)
        }
    }
}

impl<T: Transaction> PendingTransaction<T> {
    pub fn nonce(&self) -> u64 {
        self.transaction.nonce()
    }
}

/// Represents all relevant information of an executed transaction
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionInfo {
    pub transaction_hash: B256,
    pub transaction_index: u64,
    pub from: Address,
    pub to: Option<Address>,
    pub contract_address: Option<Address>,
    pub traces: Vec<CallTraceNode>,
    pub exit: InstructionResult,
    pub out: Option<Bytes>,
    pub nonce: u64,
    pub gas_used: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{
        BlobTransactionSidecar, SignableTransaction, TxEip4844,
        transaction::eip4844::TxEip4844Variant,
    };
    use alloy_primitives::{Signature, U256};

    fn sidecarful_4844_envelope() -> (FoundryTxEnvelope, BlobTransactionSidecarVariant) {
        let tx = TxEip4844 {
            chain_id: 1,
            nonce: 0,
            gas_limit: 21_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 1,
            to: Address::ZERO,
            value: U256::ZERO,
            access_list: Default::default(),
            blob_versioned_hashes: vec![B256::ZERO],
            max_fee_per_blob_gas: 1,
            input: Bytes::default(),
        };
        let sidecar = BlobTransactionSidecarVariant::Eip4844(BlobTransactionSidecar::new(
            vec![Default::default()],
            vec![Default::default()],
            vec![Default::default()],
        ));
        let signature = Signature::new(U256::from(1), U256::from(1), false);
        let variant = TxEip4844Variant::TxEip4844WithSidecar(tx.with_sidecar(sidecar.clone()));
        (FoundryTxEnvelope::Eip4844(variant.into_signed(signature)), sidecar)
    }

    // Impersonated transactions are not stripped (their synthetic hash is derived from the
    // encoded transaction), but the sidecar is still returned so callers can index it.
    #[test]
    fn impersonated_blob_strip_keeps_sidecar_and_hash() {
        let (transaction, sidecar) = sidecarful_4844_envelope();
        let transaction =
            MaybeImpersonatedTransaction::impersonated(transaction, Address::with_last_byte(1));
        let hash = transaction.hash();

        assert!(transaction.as_ref().sidecar().is_some());

        let (kept, returned_sidecar) = transaction.strip_blob_sidecar();

        assert_eq!(returned_sidecar, Some(sidecar));
        assert!(kept.as_ref().sidecar().is_some());
        assert_eq!(kept.hash(), hash);
    }

    // Properly-signed transactions are stripped to their canonical form, hash unchanged.
    #[test]
    fn signed_blob_strip_removes_sidecar_and_preserves_hash() {
        let (transaction, sidecar) = sidecarful_4844_envelope();
        let transaction = MaybeImpersonatedTransaction::new(transaction);
        let hash = transaction.hash();

        let (stripped, stripped_sidecar) = transaction.strip_blob_sidecar();

        assert_eq!(stripped_sidecar, Some(sidecar));
        assert!(stripped.as_ref().sidecar().is_none());
        assert_eq!(stripped.hash(), hash);
    }

    #[test]
    fn impersonated_hash_includes_sender() {
        let (transaction, _) = sidecarful_4844_envelope();

        let first = MaybeImpersonatedTransaction::impersonated(
            transaction.clone(),
            Address::with_last_byte(1),
        );
        let second =
            MaybeImpersonatedTransaction::impersonated(transaction, Address::with_last_byte(2));

        assert_ne!(first.hash(), second.hash());
    }
}
