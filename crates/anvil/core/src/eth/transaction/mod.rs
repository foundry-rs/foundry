//! Transaction related types
use alloy_consensus::{
    Transaction, Typed2718,
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
pub struct MaybeImpersonatedTransaction<T = FoundryTxEnvelope> {
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
    pub fn new(transaction: T) -> Self {
        Self { transaction, impersonated_sender: None }
    }

    /// Creates a new impersonated transaction wrapper using the given sender
    pub fn impersonated(transaction: T, impersonated_sender: Address) -> Self {
        Self { transaction, impersonated_sender: Some(impersonated_sender) }
    }

    /// Returns whether the transaction is impersonated
    pub fn is_impersonated(&self) -> bool {
        self.impersonated_sender.is_some()
    }

    /// Returns the inner transaction.
    pub fn into_inner(self) -> T {
        self.transaction
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
        let sender = transaction.recover()?;
        let hash = transaction.hash();
        Ok(Self { transaction: MaybeImpersonatedTransaction::new(transaction), sender, hash })
    }

    pub fn with_impersonated(transaction: FoundryTxEnvelope, sender: Address) -> Self {
        let transaction = MaybeImpersonatedTransaction::impersonated(transaction, sender);
        let hash = transaction.hash();
        Self { transaction, sender, hash }
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
