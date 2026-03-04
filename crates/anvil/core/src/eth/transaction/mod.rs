//! Transaction related types
use alloy_consensus::{Transaction, Typed2718, crypto::RecoveryError};

use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{Address, B256, Bytes, TxHash};
use alloy_rlp::{Decodable, Encodable};
use bytes::BufMut;
use foundry_evm::traces::CallTraceNode;
use foundry_primitives::FoundryTxEnvelope;
use revm::interpreter::InstructionResult;
use serde::{Deserialize, Serialize};
use std::ops::Deref;

/// Trait for transaction envelopes that support impersonation.
///
/// Provides the methods needed by [`MaybeImpersonatedTransaction`] to compute hashes
/// and recover signers, abstracting over the concrete envelope type.
pub trait ImpersonatedTxEnvelope {
    /// Returns the hash of the transaction.
    fn tx_hash(&self) -> B256;

    /// Recovers the address which was used to sign the transaction.
    fn recover_signer(&self) -> Result<Address, RecoveryError>;

    /// Returns a modified hash that makes impersonated transactions unique.
    ///
    /// This appends the `sender` address to the encoded transaction before hashing.
    fn impersonated_hash(&self, sender: Address) -> B256;
}

impl ImpersonatedTxEnvelope for FoundryTxEnvelope {
    fn tx_hash(&self) -> B256 {
        self.hash()
    }

    fn recover_signer(&self) -> Result<Address, RecoveryError> {
        self.recover()
    }

    fn impersonated_hash(&self, sender: Address) -> B256 {
        Self::impersonated_hash(self, sender)
    }
}

/// Anvil's concrete impersonated transaction type.
pub type MaybeImpersonatedTransaction = ImpersonatedTransaction<FoundryTxEnvelope>;

/// A wrapper for a transaction envelope that allows impersonating accounts.
///
/// This is a helper that carries the `impersonated` sender so that the right hash
/// can be created via [`ImpersonatedTxEnvelope::impersonated_hash`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImpersonatedTransaction<T> {
    transaction: T,
    impersonated_sender: Option<Address>,
}

impl<T: Typed2718> Typed2718 for ImpersonatedTransaction<T> {
    fn ty(&self) -> u8 {
        self.transaction.ty()
    }
}

impl<T: ImpersonatedTxEnvelope> ImpersonatedTransaction<T> {
    /// Creates a new wrapper for the given transaction
    pub fn new(transaction: T) -> Self {
        Self { transaction, impersonated_sender: None }
    }

    /// Creates a new impersonated transaction wrapper using the given sender
    pub fn impersonated(transaction: T, impersonated_sender: Address) -> Self {
        Self { transaction, impersonated_sender: Some(impersonated_sender) }
    }

    /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, RecoveryError> {
        if let Some(sender) = self.impersonated_sender {
            return Ok(sender);
        }
        self.transaction.recover_signer()
    }

    /// Returns whether the transaction is impersonated
    pub fn is_impersonated(&self) -> bool {
        self.impersonated_sender.is_some()
    }

    /// Returns the hash of the transaction
    pub fn hash(&self) -> B256 {
        if let Some(sender) = self.impersonated_sender {
            return self.transaction.impersonated_hash(sender);
        }
        self.transaction.tx_hash()
    }

    /// Returns the inner transaction.
    pub fn into_inner(self) -> T {
        self.transaction
    }
}

impl<T: Encodable2718> Encodable2718 for ImpersonatedTransaction<T> {
    fn encode_2718_len(&self) -> usize {
        self.transaction.encode_2718_len()
    }

    fn encode_2718(&self, out: &mut dyn BufMut) {
        self.transaction.encode_2718(out)
    }
}

impl<T: Encodable> Encodable for ImpersonatedTransaction<T> {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        self.transaction.encode(out)
    }
}

impl From<MaybeImpersonatedTransaction> for FoundryTxEnvelope {
    fn from(value: MaybeImpersonatedTransaction) -> Self {
        value.transaction
    }
}

impl From<FoundryTxEnvelope> for MaybeImpersonatedTransaction {
    fn from(value: FoundryTxEnvelope) -> Self {
        Self::new(value)
    }
}

impl<T: Decodable + ImpersonatedTxEnvelope> Decodable for ImpersonatedTransaction<T> {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        T::decode(buf).map(Self::new)
    }
}

impl<T> AsRef<T> for ImpersonatedTransaction<T> {
    fn as_ref(&self) -> &T {
        &self.transaction
    }
}

impl<T> Deref for ImpersonatedTransaction<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.transaction
    }
}

/// Queued transaction
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingTransaction {
    /// The actual transaction
    pub transaction: MaybeImpersonatedTransaction,
    /// the recovered sender of this transaction
    sender: Address,
    /// hash of `transaction`, so it can easily be reused with encoding and hashing again
    hash: TxHash,
}

impl PendingTransaction {
    pub fn new(transaction: FoundryTxEnvelope) -> Result<Self, RecoveryError> {
        let sender = transaction.recover_signer()?;
        let hash = transaction.tx_hash();
        Ok(Self { transaction: MaybeImpersonatedTransaction::new(transaction), sender, hash })
    }

    pub fn with_impersonated(transaction: FoundryTxEnvelope, sender: Address) -> Self {
        let hash = ImpersonatedTxEnvelope::impersonated_hash(&transaction, sender);
        Self {
            transaction: MaybeImpersonatedTransaction::impersonated(transaction, sender),
            sender,
            hash,
        }
    }

    /// Converts a [`MaybeImpersonatedTransaction`] into a [`PendingTransaction`].
    pub fn from_maybe_impersonated(
        transaction: MaybeImpersonatedTransaction,
    ) -> Result<Self, RecoveryError> {
        if let Some(impersonated) = transaction.impersonated_sender {
            Ok(Self::with_impersonated(transaction.transaction, impersonated))
        } else {
            Self::new(transaction.transaction)
        }
    }

    pub fn nonce(&self) -> u64 {
        self.transaction.nonce()
    }

    pub fn hash(&self) -> &TxHash {
        &self.hash
    }

    pub fn sender(&self) -> &Address {
        &self.sender
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
