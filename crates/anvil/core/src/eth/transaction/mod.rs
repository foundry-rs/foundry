//! Transaction related types
use alloy_consensus::{
    Signed, Transaction, TxEnvelope, Typed2718, crypto::RecoveryError, transaction::Recovered,
};

use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{Address, B256, Bytes, TxHash};
use alloy_rlp::{Decodable, Encodable};
use alloy_rpc_types::Transaction as RpcTransaction;
use bytes::BufMut;
use foundry_evm::traces::CallTraceNode;
use foundry_primitives::FoundryTxEnvelope;
use revm::interpreter::InstructionResult;
use serde::{Deserialize, Serialize};
use std::ops::Deref;

/// A wrapper for [FoundryTxEnvelope] that allows impersonating accounts.
///
/// This is a helper that carries the `impersonated` sender so that the right hash
/// [FoundryTxEnvelope::impersonated_hash] can be created.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaybeImpersonatedTransaction {
    transaction: FoundryTxEnvelope,
    impersonated_sender: Option<Address>,
}

impl Typed2718 for MaybeImpersonatedTransaction {
    fn ty(&self) -> u8 {
        self.transaction.ty()
    }
}

impl MaybeImpersonatedTransaction {
    /// Creates a new wrapper for the given transaction
    pub fn new(transaction: FoundryTxEnvelope) -> Self {
        Self { transaction, impersonated_sender: None }
    }

    /// Creates a new impersonated transaction wrapper using the given sender
    pub fn impersonated(transaction: FoundryTxEnvelope, impersonated_sender: Address) -> Self {
        Self { transaction, impersonated_sender: Some(impersonated_sender) }
    }

    /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, RecoveryError> {
        if let Some(sender) = self.impersonated_sender {
            return Ok(sender);
        }
        self.transaction.recover()
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
        self.transaction.hash()
    }

    /// Converts the transaction into an [`RpcTransaction`]
    pub fn into_rpc_transaction(self) -> RpcTransaction {
        let hash = self.hash();
        let from = self.recover().unwrap_or_default();
        let envelope = self.transaction.try_into_eth().expect("cant build deposit transactions");

        // NOTE: we must update the hash because the tx can be impersonated, this requires forcing
        // the hash
        let inner_envelope = match envelope {
            TxEnvelope::Legacy(t) => {
                let (tx, sig, _) = t.into_parts();
                TxEnvelope::Legacy(Signed::new_unchecked(tx, sig, hash))
            }
            TxEnvelope::Eip2930(t) => {
                let (tx, sig, _) = t.into_parts();
                TxEnvelope::Eip2930(Signed::new_unchecked(tx, sig, hash))
            }
            TxEnvelope::Eip1559(t) => {
                let (tx, sig, _) = t.into_parts();
                TxEnvelope::Eip1559(Signed::new_unchecked(tx, sig, hash))
            }
            TxEnvelope::Eip4844(t) => {
                let (tx, sig, _) = t.into_parts();
                TxEnvelope::Eip4844(Signed::new_unchecked(tx, sig, hash))
            }
            TxEnvelope::Eip7702(t) => {
                let (tx, sig, _) = t.into_parts();
                TxEnvelope::Eip7702(Signed::new_unchecked(tx, sig, hash))
            }
        };

        RpcTransaction {
            block_hash: None,
            block_number: None,
            transaction_index: None,
            effective_gas_price: None,
            inner: Recovered::new_unchecked(inner_envelope, from),
        }
    }
}

impl Encodable2718 for MaybeImpersonatedTransaction {
    fn encode_2718_len(&self) -> usize {
        self.transaction.encode_2718_len()
    }

    fn encode_2718(&self, out: &mut dyn BufMut) {
        self.transaction.encode_2718(out)
    }
}

impl Encodable for MaybeImpersonatedTransaction {
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

impl Decodable for MaybeImpersonatedTransaction {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        FoundryTxEnvelope::decode(buf).map(Self::new)
    }
}

impl AsRef<FoundryTxEnvelope> for MaybeImpersonatedTransaction {
    fn as_ref(&self) -> &FoundryTxEnvelope {
        &self.transaction
    }
}

impl Deref for MaybeImpersonatedTransaction {
    type Target = FoundryTxEnvelope;

    fn deref(&self) -> &Self::Target {
        &self.transaction
    }
}

impl From<MaybeImpersonatedTransaction> for RpcTransaction {
    fn from(value: MaybeImpersonatedTransaction) -> Self {
        value.into_rpc_transaction()
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
        let hash = transaction.impersonated_hash(sender);
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
