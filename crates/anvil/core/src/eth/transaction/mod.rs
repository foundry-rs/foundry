//! Transaction related types

use crate::eth::transaction::optimism::{DepositTransaction, DepositTransactionRequest};
use alloy_consensus::{
    transaction::{
        eip4844::{TxEip4844, TxEip4844Variant, TxEip4844WithSidecar},
        TxEip7702,
    },
    AnyReceiptEnvelope, Receipt, ReceiptEnvelope, ReceiptWithBloom, Signed, TxEip1559, TxEip2930,
    TxEnvelope, TxLegacy, TxReceipt, TxType,
};
use alloy_eips::eip2718::{Decodable2718, Eip2718Error, Encodable2718};
use alloy_primitives::{
    Address, Bloom, Bytes, Log, Parity, Signature, TxHash, TxKind, B256, U256, U64,
};
use alloy_rlp::{length_of_length, Decodable, Encodable, Header};
use alloy_rpc_types::{
    request::TransactionRequest, trace::otterscan::OtsReceipt, AccessList, AnyTransactionReceipt,
    ConversionError, Signature as RpcSignature, Transaction as RpcTransaction, TransactionReceipt,
};
use alloy_serde::{OtherFields, WithOtherFields};
use bytes::BufMut;
use foundry_evm::traces::CallTraceNode;
use revm::{
    interpreter::InstructionResult,
    primitives::{OptimismFields, TxEnv},
};
use serde::{Deserialize, Serialize};
use std::ops::{Deref, Mul};

pub mod optimism;

/// Converts a [TransactionRequest] into a [TypedTransactionRequest].
/// Should be removed once the call builder abstraction for providers is in place.
pub fn transaction_request_to_typed(
    tx: WithOtherFields<TransactionRequest>,
) -> Option<TypedTransactionRequest> {
    let WithOtherFields::<TransactionRequest> {
        inner:
            TransactionRequest {
                from,
                to,
                gas_price,
                max_fee_per_gas,
                max_priority_fee_per_gas,
                max_fee_per_blob_gas,
                blob_versioned_hashes,
                gas,
                value,
                input,
                nonce,
                access_list,
                sidecar,
                transaction_type,
                ..
            },
        other,
    } = tx;

    // Special case: OP-stack deposit tx
    if transaction_type == Some(0x7E) || has_optimism_fields(&other) {
        return Some(TypedTransactionRequest::Deposit(DepositTransactionRequest {
            from: from.unwrap_or_default(),
            source_hash: other.get_deserialized::<B256>("sourceHash")?.ok()?,
            kind: to.unwrap_or_default(),
            mint: other.get_deserialized::<U256>("mint")?.ok()?,
            value: value.unwrap_or_default(),
            gas_limit: gas.unwrap_or_default(),
            is_system_tx: other.get_deserialized::<bool>("isSystemTx")?.ok()?,
            input: input.into_input().unwrap_or_default(),
        }));
    }

    match (
        transaction_type,
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        access_list.as_ref(),
        max_fee_per_blob_gas,
        blob_versioned_hashes.as_ref(),
        sidecar,
        to,
    ) {
        // legacy transaction
        (Some(0), _, None, None, None, None, None, None, _) |
        (None, Some(_), None, None, None, None, None, None, _) => {
            Some(TypedTransactionRequest::Legacy(TxLegacy {
                nonce: nonce.unwrap_or_default(),
                gas_price: gas_price.unwrap_or_default(),
                gas_limit: gas.unwrap_or_default(),
                value: value.unwrap_or(U256::ZERO),
                input: input.into_input().unwrap_or_default(),
                to: to.unwrap_or_default(),
                chain_id: None,
            }))
        }
        // EIP2930
        (Some(1), _, None, None, _, None, None, None, _) |
        (None, _, None, None, Some(_), None, None, None, _) => {
            Some(TypedTransactionRequest::EIP2930(TxEip2930 {
                nonce: nonce.unwrap_or_default(),
                gas_price: gas_price.unwrap_or_default(),
                gas_limit: gas.unwrap_or_default(),
                value: value.unwrap_or(U256::ZERO),
                input: input.into_input().unwrap_or_default(),
                to: to.unwrap_or_default(),
                chain_id: 0,
                access_list: access_list.unwrap_or_default(),
            }))
        }
        // EIP1559
        (Some(2), None, _, _, _, _, None, None, _) |
        (None, None, Some(_), _, _, _, None, None, _) |
        (None, None, _, Some(_), _, _, None, None, _) |
        (None, None, None, None, None, _, None, None, _) => {
            // Empty fields fall back to the canonical transaction schema.
            Some(TypedTransactionRequest::EIP1559(TxEip1559 {
                nonce: nonce.unwrap_or_default(),
                max_fee_per_gas: max_fee_per_gas.unwrap_or_default(),
                max_priority_fee_per_gas: max_priority_fee_per_gas.unwrap_or_default(),
                gas_limit: gas.unwrap_or_default(),
                value: value.unwrap_or(U256::ZERO),
                input: input.into_input().unwrap_or_default(),
                to: to.unwrap_or_default(),
                chain_id: 0,
                access_list: access_list.unwrap_or_default(),
            }))
        }
        // EIP4844
        (Some(3), None, _, _, _, _, Some(_), Some(sidecar), to) => {
            let tx = TxEip4844 {
                nonce: nonce.unwrap_or_default(),
                max_fee_per_gas: max_fee_per_gas.unwrap_or_default(),
                max_priority_fee_per_gas: max_priority_fee_per_gas.unwrap_or_default(),
                max_fee_per_blob_gas: max_fee_per_blob_gas.unwrap_or_default(),
                gas_limit: gas.unwrap_or_default(),
                value: value.unwrap_or(U256::ZERO),
                input: input.into_input().unwrap_or_default(),
                to: match to.unwrap_or(TxKind::Create) {
                    TxKind::Call(to) => to,
                    TxKind::Create => Address::ZERO,
                },
                chain_id: 0,
                access_list: access_list.unwrap_or_default(),
                blob_versioned_hashes: blob_versioned_hashes.unwrap_or_default(),
            };
            Some(TypedTransactionRequest::EIP4844(TxEip4844Variant::TxEip4844WithSidecar(
                TxEip4844WithSidecar::from_tx_and_sidecar(tx, sidecar),
            )))
        }
        _ => None,
    }
}

fn has_optimism_fields(other: &OtherFields) -> bool {
    other.contains_key("sourceHash") &&
        other.contains_key("mint") &&
        other.contains_key("isSystemTx")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypedTransactionRequest {
    Legacy(TxLegacy),
    EIP2930(TxEip2930),
    EIP1559(TxEip1559),
    EIP4844(TxEip4844Variant),
    Deposit(DepositTransactionRequest),
}

/// A wrapper for [TypedTransaction] that allows impersonating accounts.
///
/// This is a helper that carries the `impersonated` sender so that the right hash
/// [TypedTransaction::impersonated_hash] can be created.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaybeImpersonatedTransaction {
    pub transaction: TypedTransaction,
    pub impersonated_sender: Option<Address>,
}

impl MaybeImpersonatedTransaction {
    /// Creates a new wrapper for the given transaction
    pub fn new(transaction: TypedTransaction) -> Self {
        Self { transaction, impersonated_sender: None }
    }

    /// Creates a new impersonated transaction wrapper using the given sender
    pub fn impersonated(transaction: TypedTransaction, impersonated_sender: Address) -> Self {
        Self { transaction, impersonated_sender: Some(impersonated_sender) }
    }

    /// Recovers the Ethereum address which was used to sign the transaction.
    ///
    /// Note: this is feature gated so it does not conflict with the `Deref`ed
    /// [TypedTransaction::recover] function by default.
    #[cfg(feature = "impersonated-tx")]
    pub fn recover(&self) -> Result<Address, alloy_primitives::SignatureError> {
        if let Some(sender) = self.impersonated_sender {
            return Ok(sender);
        }
        self.transaction.recover()
    }

    /// Returns whether the transaction is impersonated
    ///
    /// Note: this is feature gated so it does not conflict with the `Deref`ed
    /// [TypedTransaction::hash] function by default.
    #[cfg(feature = "impersonated-tx")]
    pub fn is_impersonated(&self) -> bool {
        self.impersonated_sender.is_some()
    }

    /// Returns the hash of the transaction
    ///
    /// Note: this is feature gated so it does not conflict with the `Deref`ed
    /// [TypedTransaction::hash] function by default.
    #[cfg(feature = "impersonated-tx")]
    pub fn hash(&self) -> B256 {
        if let Some(sender) = self.impersonated_sender {
            return self.transaction.impersonated_hash(sender)
        }
        self.transaction.hash()
    }
}

impl Encodable for MaybeImpersonatedTransaction {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        self.transaction.encode(out)
    }
}

impl From<MaybeImpersonatedTransaction> for TypedTransaction {
    fn from(value: MaybeImpersonatedTransaction) -> Self {
        value.transaction
    }
}

impl From<TypedTransaction> for MaybeImpersonatedTransaction {
    fn from(value: TypedTransaction) -> Self {
        Self::new(value)
    }
}

impl Decodable for MaybeImpersonatedTransaction {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        TypedTransaction::decode(buf).map(Self::new)
    }
}

impl AsRef<TypedTransaction> for MaybeImpersonatedTransaction {
    fn as_ref(&self) -> &TypedTransaction {
        &self.transaction
    }
}

impl Deref for MaybeImpersonatedTransaction {
    type Target = TypedTransaction;

    fn deref(&self) -> &Self::Target {
        &self.transaction
    }
}

impl From<MaybeImpersonatedTransaction> for RpcTransaction {
    fn from(value: MaybeImpersonatedTransaction) -> Self {
        let hash = value.hash();
        let sender = value.recover().unwrap_or_default();
        to_alloy_transaction_with_hash_and_sender(value.transaction, hash, sender)
    }
}

pub fn to_alloy_transaction_with_hash_and_sender(
    transaction: TypedTransaction,
    hash: B256,
    from: Address,
) -> RpcTransaction {
    match transaction {
        TypedTransaction::Legacy(t) => RpcTransaction {
            hash,
            nonce: t.tx().nonce,
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: t.tx().to.to().copied(),
            value: t.tx().value,
            gas_price: Some(t.tx().gas_price),
            max_fee_per_gas: Some(t.tx().gas_price),
            max_priority_fee_per_gas: Some(t.tx().gas_price),
            gas: t.tx().gas_limit,
            input: t.tx().input.clone(),
            chain_id: t.tx().chain_id,
            signature: Some(RpcSignature {
                r: t.signature().r(),
                s: t.signature().s(),
                v: U256::from(t.signature().v().to_u64()),
                y_parity: None,
            }),
            access_list: None,
            transaction_type: None,
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: None,
            authorization_list: None,
        },
        TypedTransaction::EIP2930(t) => RpcTransaction {
            hash,
            nonce: t.tx().nonce,
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: t.tx().to.to().copied(),
            value: t.tx().value,
            gas_price: Some(t.tx().gas_price),
            max_fee_per_gas: Some(t.tx().gas_price),
            max_priority_fee_per_gas: Some(t.tx().gas_price),
            gas: t.tx().gas_limit,
            input: t.tx().input.clone(),
            chain_id: Some(t.tx().chain_id),
            signature: Some(RpcSignature {
                r: t.signature().r(),
                s: t.signature().s(),
                v: U256::from(t.signature().v().to_u64()),
                y_parity: Some(alloy_rpc_types::Parity::from(t.signature().v().y_parity())),
            }),
            access_list: Some(t.tx().access_list.clone()),
            transaction_type: Some(1),
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: None,
            authorization_list: None,
        },
        TypedTransaction::EIP1559(t) => RpcTransaction {
            hash,
            nonce: t.tx().nonce,
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: t.tx().to.to().copied(),
            value: t.tx().value,
            gas_price: None,
            max_fee_per_gas: Some(t.tx().max_fee_per_gas),
            max_priority_fee_per_gas: Some(t.tx().max_priority_fee_per_gas),
            gas: t.tx().gas_limit,
            input: t.tx().input.clone(),
            chain_id: Some(t.tx().chain_id),
            signature: Some(RpcSignature {
                r: t.signature().r(),
                s: t.signature().s(),
                v: U256::from(t.signature().v().to_u64()),
                y_parity: Some(alloy_rpc_types::Parity::from(t.signature().v().y_parity())),
            }),
            access_list: Some(t.tx().access_list.clone()),
            transaction_type: Some(2),
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: None,
            authorization_list: None,
        },
        TypedTransaction::EIP4844(t) => RpcTransaction {
            hash,
            nonce: t.tx().tx().nonce,
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: Some(t.tx().tx().to),
            value: t.tx().tx().value,
            gas_price: Some(t.tx().tx().max_fee_per_gas),
            max_fee_per_gas: Some(t.tx().tx().max_fee_per_gas),
            max_priority_fee_per_gas: Some(t.tx().tx().max_priority_fee_per_gas),
            gas: t.tx().tx().gas_limit,
            input: t.tx().tx().input.clone(),
            chain_id: Some(t.tx().tx().chain_id),
            signature: Some(RpcSignature {
                r: t.signature().r(),
                s: t.signature().s(),
                v: U256::from(t.signature().v().to_u64()),
                y_parity: Some(alloy_rpc_types::Parity::from(t.signature().v().y_parity())),
            }),
            access_list: Some(t.tx().tx().access_list.clone()),
            transaction_type: Some(3),
            max_fee_per_blob_gas: Some(t.tx().tx().max_fee_per_blob_gas),
            blob_versioned_hashes: Some(t.tx().tx().blob_versioned_hashes.clone()),
            authorization_list: None,
        },
        TypedTransaction::EIP7702(t) => RpcTransaction {
            hash,
            nonce: t.tx().nonce,
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: Some(t.tx().to),
            value: t.tx().value,
            gas_price: Some(t.tx().max_fee_per_gas),
            max_fee_per_gas: Some(t.tx().max_fee_per_gas),
            max_priority_fee_per_gas: Some(t.tx().max_priority_fee_per_gas),
            gas: t.tx().gas_limit,
            input: t.tx().input.clone(),
            chain_id: Some(t.tx().chain_id),
            signature: Some(RpcSignature {
                r: t.signature().r(),
                s: t.signature().s(),
                v: U256::from(t.signature().v().to_u64()),
                y_parity: Some(alloy_rpc_types::Parity::from(t.signature().v().y_parity())),
            }),
            access_list: Some(t.tx().access_list.clone()),
            transaction_type: Some(4),
            authorization_list: Some(t.tx().authorization_list.clone()),
            ..Default::default()
        },
        TypedTransaction::Deposit(t) => RpcTransaction {
            hash,
            nonce: t.nonce,
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: t.kind.to().copied(),
            value: t.value,
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            gas: t.gas_limit,
            input: t.input.clone().0.into(),
            chain_id: t.chain_id().map(u64::from),
            signature: None,
            access_list: None,
            transaction_type: None,
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: None,
            authorization_list: None,
        },
    }
}

/// Queued transaction
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingTransaction {
    /// The actual transaction
    pub transaction: MaybeImpersonatedTransaction,
    /// the recovered sender of this transaction
    sender: Address,
    /// hash of `transaction`, so it can easily be reused with encoding and hashing agan
    hash: TxHash,
}

impl PendingTransaction {
    pub fn new(transaction: TypedTransaction) -> Result<Self, alloy_primitives::SignatureError> {
        let sender = transaction.recover()?;
        let hash = transaction.hash();
        Ok(Self { transaction: MaybeImpersonatedTransaction::new(transaction), sender, hash })
    }

    #[cfg(feature = "impersonated-tx")]
    pub fn with_impersonated(transaction: TypedTransaction, sender: Address) -> Self {
        let hash = transaction.impersonated_hash(sender);
        Self {
            transaction: MaybeImpersonatedTransaction::impersonated(transaction, sender),
            sender,
            hash,
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

    /// Converts the [PendingTransaction] into the [TxEnv] context that [`revm`](foundry_evm)
    /// expects.
    pub fn to_revm_tx_env(&self) -> TxEnv {
        fn transact_to(kind: &TxKind) -> TxKind {
            match kind {
                TxKind::Call(c) => TxKind::Call(*c),
                TxKind::Create => TxKind::Create,
            }
        }

        let caller = *self.sender();
        match &self.transaction.transaction {
            TypedTransaction::Legacy(tx) => {
                let chain_id = tx.tx().chain_id;
                let TxLegacy { nonce, gas_price, gas_limit, value, to, input, .. } = tx.tx();
                TxEnv {
                    caller,
                    transact_to: transact_to(to),
                    data: input.clone(),
                    chain_id,
                    nonce: Some(*nonce),
                    value: (*value),
                    gas_price: U256::from(*gas_price),
                    gas_priority_fee: None,
                    gas_limit: *gas_limit as u64,
                    access_list: vec![],
                    ..Default::default()
                }
            }
            TypedTransaction::EIP2930(tx) => {
                let TxEip2930 {
                    chain_id,
                    nonce,
                    gas_price,
                    gas_limit,
                    to,
                    value,
                    input,
                    access_list,
                    ..
                } = tx.tx();
                TxEnv {
                    caller,
                    transact_to: transact_to(to),
                    data: input.clone(),
                    chain_id: Some(*chain_id),
                    nonce: Some(*nonce),
                    value: *value,
                    gas_price: U256::from(*gas_price),
                    gas_priority_fee: None,
                    gas_limit: *gas_limit as u64,
                    access_list: access_list.clone().into(),
                    ..Default::default()
                }
            }
            TypedTransaction::EIP1559(tx) => {
                let TxEip1559 {
                    chain_id,
                    nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit,
                    to,
                    value,
                    input,
                    access_list,
                    ..
                } = tx.tx();
                TxEnv {
                    caller,
                    transact_to: transact_to(to),
                    data: input.clone(),
                    chain_id: Some(*chain_id),
                    nonce: Some(*nonce),
                    value: *value,
                    gas_price: U256::from(*max_fee_per_gas),
                    gas_priority_fee: Some(U256::from(*max_priority_fee_per_gas)),
                    gas_limit: *gas_limit as u64,
                    access_list: access_list.clone().into(),
                    ..Default::default()
                }
            }
            TypedTransaction::EIP4844(tx) => {
                let TxEip4844 {
                    chain_id,
                    nonce,
                    max_fee_per_blob_gas,
                    max_fee_per_gas,
                    max_priority_fee_per_gas,
                    gas_limit,
                    to,
                    value,
                    input,
                    access_list,
                    blob_versioned_hashes,
                    ..
                } = tx.tx().tx();
                TxEnv {
                    caller,
                    transact_to: TxKind::Call(*to),
                    data: input.clone(),
                    chain_id: Some(*chain_id),
                    nonce: Some(*nonce),
                    value: *value,
                    gas_price: U256::from(*max_fee_per_gas),
                    gas_priority_fee: Some(U256::from(*max_priority_fee_per_gas)),
                    max_fee_per_blob_gas: Some(U256::from(*max_fee_per_blob_gas)),
                    blob_hashes: blob_versioned_hashes.clone(),
                    gas_limit: *gas_limit as u64,
                    access_list: access_list.clone().into(),
                    ..Default::default()
                }
            }
            TypedTransaction::EIP7702(tx) => {
                let TxEip7702 {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee_per_gas,
                    max_priority_fee_per_gas,
                    to,
                    value,
                    access_list,
                    authorization_list,
                    input,
                } = tx.tx();
                TxEnv {
                    caller,
                    transact_to: TxKind::Call(*to),
                    data: input.clone(),
                    chain_id: Some(*chain_id),
                    nonce: Some(*nonce),
                    value: *value,
                    gas_price: U256::from(*max_fee_per_gas),
                    gas_priority_fee: Some(U256::from(*max_priority_fee_per_gas)),
                    gas_limit: *gas_limit as u64,
                    access_list: access_list.clone().into(),
                    authorization_list: Some(authorization_list.clone().into()),
                    ..Default::default()
                }
            }
            TypedTransaction::Deposit(tx) => {
                let chain_id = tx.chain_id();
                let DepositTransaction {
                    nonce,
                    source_hash,
                    gas_limit,
                    value,
                    kind,
                    mint,
                    input,
                    is_system_tx,
                    ..
                } = tx;
                TxEnv {
                    caller,
                    transact_to: transact_to(kind),
                    data: input.clone(),
                    chain_id,
                    nonce: Some(*nonce),
                    value: *value,
                    gas_price: U256::ZERO,
                    gas_priority_fee: None,
                    gas_limit: *gas_limit as u64,
                    access_list: vec![],
                    optimism: OptimismFields {
                        source_hash: Some(*source_hash),
                        mint: Some(mint.to::<u128>()),
                        is_system_transaction: Some(*is_system_tx),
                        enveloped_tx: None,
                    },
                    ..Default::default()
                }
            }
        }
    }
}

/// Container type for signed, typed transactions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypedTransaction {
    /// Legacy transaction type
    Legacy(Signed<TxLegacy>),
    /// EIP-2930 transaction
    EIP2930(Signed<TxEip2930>),
    /// EIP-1559 transaction
    EIP1559(Signed<TxEip1559>),
    /// EIP-4844 transaction
    EIP4844(Signed<TxEip4844Variant>),
    /// EIP-7702 transaction
    EIP7702(Signed<TxEip7702>),
    /// op-stack deposit transaction
    Deposit(DepositTransaction),
}

/// This is a function that demotes TypedTransaction to TransactionRequest for greater flexibility
/// over the type.
///
/// This function is purely for convenience and specific use cases, e.g. RLP encoded transactions
/// decode to TypedTransactions where the API over TypedTransctions is quite strict.
impl TryFrom<TypedTransaction> for TransactionRequest {
    type Error = ConversionError;

    fn try_from(value: TypedTransaction) -> Result<Self, Self::Error> {
        let from = value.recover().map_err(|_| ConversionError::InvalidSignature)?;
        let essentials = value.essentials();
        let tx_type = value.r#type();
        Ok(Self {
            from: Some(from),
            to: Some(value.kind()),
            gas_price: essentials.gas_price,
            max_fee_per_gas: essentials.max_fee_per_gas,
            max_priority_fee_per_gas: essentials.max_priority_fee_per_gas,
            max_fee_per_blob_gas: essentials.max_fee_per_blob_gas,
            gas: Some(essentials.gas_limit),
            value: Some(essentials.value),
            input: essentials.input.into(),
            nonce: Some(essentials.nonce),
            chain_id: essentials.chain_id,
            transaction_type: tx_type,
            ..Default::default()
        })
    }
}

impl TypedTransaction {
    /// Returns true if the transaction uses dynamic fees: EIP1559 or EIP4844
    pub fn is_dynamic_fee(&self) -> bool {
        matches!(self, Self::EIP1559(_) | Self::EIP4844(_) | Self::EIP7702(_))
    }

    pub fn gas_price(&self) -> u128 {
        match self {
            Self::Legacy(tx) => tx.tx().gas_price,
            Self::EIP2930(tx) => tx.tx().gas_price,
            Self::EIP1559(tx) => tx.tx().max_fee_per_gas,
            Self::EIP4844(tx) => tx.tx().tx().max_fee_per_gas,
            Self::EIP7702(tx) => tx.tx().max_fee_per_gas,
            Self::Deposit(_) => 0,
        }
    }

    pub fn gas_limit(&self) -> u128 {
        match self {
            Self::Legacy(tx) => tx.tx().gas_limit,
            Self::EIP2930(tx) => tx.tx().gas_limit,
            Self::EIP1559(tx) => tx.tx().gas_limit,
            Self::EIP4844(tx) => tx.tx().tx().gas_limit,
            Self::EIP7702(tx) => tx.tx().gas_limit,
            Self::Deposit(tx) => tx.gas_limit,
        }
    }

    pub fn value(&self) -> U256 {
        U256::from(match self {
            Self::Legacy(tx) => tx.tx().value,
            Self::EIP2930(tx) => tx.tx().value,
            Self::EIP1559(tx) => tx.tx().value,
            Self::EIP4844(tx) => tx.tx().tx().value,
            Self::EIP7702(tx) => tx.tx().value,
            Self::Deposit(tx) => tx.value,
        })
    }

    pub fn data(&self) -> &Bytes {
        match self {
            Self::Legacy(tx) => &tx.tx().input,
            Self::EIP2930(tx) => &tx.tx().input,
            Self::EIP1559(tx) => &tx.tx().input,
            Self::EIP4844(tx) => &tx.tx().tx().input,
            Self::EIP7702(tx) => &tx.tx().input,
            Self::Deposit(tx) => &tx.input,
        }
    }

    /// Returns the transaction type
    pub fn r#type(&self) -> Option<u8> {
        match self {
            Self::Legacy(_) => None,
            Self::EIP2930(_) => Some(1),
            Self::EIP1559(_) => Some(2),
            Self::EIP4844(_) => Some(3),
            Self::EIP7702(_) => Some(4),
            Self::Deposit(_) => Some(0x7E),
        }
    }

    /// Max cost of the transaction
    /// It is the gas limit multiplied by the gas price,
    /// and if the transaction is EIP-4844, the result of (total blob gas cost * max fee per blob
    /// gas) is also added
    pub fn max_cost(&self) -> u128 {
        let mut max_cost = self.gas_limit().saturating_mul(self.gas_price());

        if self.is_eip4844() {
            max_cost = max_cost.saturating_add(
                self.blob_gas().unwrap_or(0).mul(self.max_fee_per_blob_gas().unwrap_or(0)),
            )
        }

        max_cost
    }

    pub fn blob_gas(&self) -> Option<u128> {
        match self {
            Self::EIP4844(tx) => Some(tx.tx().tx().blob_gas() as u128),
            _ => None,
        }
    }

    pub fn max_fee_per_blob_gas(&self) -> Option<u128> {
        match self {
            Self::EIP4844(tx) => Some(tx.tx().tx().max_fee_per_blob_gas),
            _ => None,
        }
    }

    /// Returns a helper type that contains commonly used values as fields
    pub fn essentials(&self) -> TransactionEssentials {
        match self {
            Self::Legacy(t) => TransactionEssentials {
                kind: t.tx().to,
                input: t.tx().input.clone(),
                nonce: t.tx().nonce,
                gas_limit: t.tx().gas_limit,
                gas_price: Some(t.tx().gas_price),
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
                max_fee_per_blob_gas: None,
                blob_versioned_hashes: None,
                value: t.tx().value,
                chain_id: t.tx().chain_id,
                access_list: Default::default(),
            },
            Self::EIP2930(t) => TransactionEssentials {
                kind: t.tx().to,
                input: t.tx().input.clone(),
                nonce: t.tx().nonce,
                gas_limit: t.tx().gas_limit,
                gas_price: Some(t.tx().gas_price),
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
                max_fee_per_blob_gas: None,
                blob_versioned_hashes: None,
                value: t.tx().value,
                chain_id: Some(t.tx().chain_id),
                access_list: t.tx().access_list.clone(),
            },
            Self::EIP1559(t) => TransactionEssentials {
                kind: t.tx().to,
                input: t.tx().input.clone(),
                nonce: t.tx().nonce,
                gas_limit: t.tx().gas_limit,
                gas_price: None,
                max_fee_per_gas: Some(t.tx().max_fee_per_gas),
                max_priority_fee_per_gas: Some(t.tx().max_priority_fee_per_gas),
                max_fee_per_blob_gas: None,
                blob_versioned_hashes: None,
                value: t.tx().value,
                chain_id: Some(t.tx().chain_id),
                access_list: t.tx().access_list.clone(),
            },
            Self::EIP4844(t) => TransactionEssentials {
                kind: TxKind::Call(t.tx().tx().to),
                input: t.tx().tx().input.clone(),
                nonce: t.tx().tx().nonce,
                gas_limit: t.tx().tx().gas_limit,
                gas_price: Some(t.tx().tx().max_fee_per_blob_gas),
                max_fee_per_gas: Some(t.tx().tx().max_fee_per_gas),
                max_priority_fee_per_gas: Some(t.tx().tx().max_priority_fee_per_gas),
                max_fee_per_blob_gas: Some(t.tx().tx().max_fee_per_blob_gas),
                blob_versioned_hashes: Some(t.tx().tx().blob_versioned_hashes.clone()),
                value: t.tx().tx().value,
                chain_id: Some(t.tx().tx().chain_id),
                access_list: t.tx().tx().access_list.clone(),
            },
            Self::EIP7702(t) => TransactionEssentials {
                kind: TxKind::Call(t.tx().to),
                input: t.tx().input.clone(),
                nonce: t.tx().nonce,
                gas_limit: t.tx().gas_limit,
                gas_price: Some(t.tx().max_fee_per_gas),
                max_fee_per_gas: Some(t.tx().max_fee_per_gas),
                max_priority_fee_per_gas: Some(t.tx().max_priority_fee_per_gas),
                max_fee_per_blob_gas: None,
                blob_versioned_hashes: None,
                value: t.tx().value,
                chain_id: Some(t.tx().chain_id),
                access_list: t.tx().access_list.clone(),
            },
            Self::Deposit(t) => TransactionEssentials {
                kind: t.kind,
                input: t.input.clone(),
                nonce: t.nonce,
                gas_limit: t.gas_limit,
                gas_price: Some(0),
                max_fee_per_gas: None,
                max_priority_fee_per_gas: None,
                max_fee_per_blob_gas: None,
                blob_versioned_hashes: None,
                value: t.value,
                chain_id: t.chain_id(),
                access_list: Default::default(),
            },
        }
    }

    pub fn nonce(&self) -> u64 {
        match self {
            Self::Legacy(t) => t.tx().nonce,
            Self::EIP2930(t) => t.tx().nonce,
            Self::EIP1559(t) => t.tx().nonce,
            Self::EIP4844(t) => t.tx().tx().nonce,
            Self::EIP7702(t) => t.tx().nonce,
            Self::Deposit(t) => t.nonce,
        }
    }

    pub fn chain_id(&self) -> Option<u64> {
        match self {
            Self::Legacy(t) => t.tx().chain_id,
            Self::EIP2930(t) => Some(t.tx().chain_id),
            Self::EIP1559(t) => Some(t.tx().chain_id),
            Self::EIP4844(t) => Some(t.tx().tx().chain_id),
            Self::EIP7702(t) => Some(t.tx().chain_id),
            Self::Deposit(t) => t.chain_id(),
        }
    }

    pub fn as_legacy(&self) -> Option<&Signed<TxLegacy>> {
        match self {
            Self::Legacy(tx) => Some(tx),
            _ => None,
        }
    }

    /// Returns true whether this tx is a legacy transaction
    pub fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy(_))
    }

    /// Returns true whether this tx is a EIP1559 transaction
    pub fn is_eip1559(&self) -> bool {
        matches!(self, Self::EIP1559(_))
    }

    /// Returns true whether this tx is a EIP2930 transaction
    pub fn is_eip2930(&self) -> bool {
        matches!(self, Self::EIP2930(_))
    }

    /// Returns true whether this tx is a EIP4844 transaction
    pub fn is_eip4844(&self) -> bool {
        matches!(self, Self::EIP4844(_))
    }

    /// Returns the hash of the transaction.
    ///
    /// Note: If this transaction has the Impersonated signature then this returns a modified unique
    /// hash. This allows us to treat impersonated transactions as unique.
    pub fn hash(&self) -> B256 {
        match self {
            Self::Legacy(t) => *t.hash(),
            Self::EIP2930(t) => *t.hash(),
            Self::EIP1559(t) => *t.hash(),
            Self::EIP4844(t) => *t.hash(),
            Self::EIP7702(t) => *t.hash(),
            Self::Deposit(t) => t.hash(),
        }
    }

    /// Returns the hash if the transaction is impersonated (using a fake signature)
    ///
    /// This appends the `address` before hashing it
    #[cfg(feature = "impersonated-tx")]
    pub fn impersonated_hash(&self, sender: Address) -> B256 {
        let mut buffer = Vec::new();
        Encodable::encode(self, &mut buffer);
        buffer.extend_from_slice(sender.as_ref());
        B256::from_slice(alloy_primitives::utils::keccak256(&buffer).as_slice())
    }

    /// Recovers the Ethereum address which was used to sign the transaction.
    pub fn recover(&self) -> Result<Address, alloy_primitives::SignatureError> {
        match self {
            Self::Legacy(tx) => tx.recover_signer(),
            Self::EIP2930(tx) => tx.recover_signer(),
            Self::EIP1559(tx) => tx.recover_signer(),
            Self::EIP4844(tx) => tx.recover_signer(),
            Self::EIP7702(tx) => tx.recover_signer(),
            Self::Deposit(tx) => tx.recover(),
        }
    }

    /// Returns what kind of transaction this is
    pub fn kind(&self) -> TxKind {
        match self {
            Self::Legacy(tx) => tx.tx().to,
            Self::EIP2930(tx) => tx.tx().to,
            Self::EIP1559(tx) => tx.tx().to,
            Self::EIP4844(tx) => TxKind::Call(tx.tx().tx().to),
            Self::EIP7702(tx) => TxKind::Call(tx.tx().to),
            Self::Deposit(tx) => tx.kind,
        }
    }

    /// Returns the callee if this transaction is a call
    pub fn to(&self) -> Option<Address> {
        self.kind().to().copied()
    }

    /// Returns the Signature of the transaction
    pub fn signature(&self) -> Signature {
        match self {
            Self::Legacy(tx) => *tx.signature(),
            Self::EIP2930(tx) => *tx.signature(),
            Self::EIP1559(tx) => *tx.signature(),
            Self::EIP4844(tx) => *tx.signature(),
            Self::EIP7702(tx) => *tx.signature(),
            Self::Deposit(_) => Signature::from_scalars_and_parity(
                B256::with_last_byte(1),
                B256::with_last_byte(1),
                Parity::Parity(false),
            )
            .unwrap(),
        }
    }
}

impl TryFrom<WithOtherFields<RpcTransaction>> for TypedTransaction {
    type Error = ConversionError;

    fn try_from(tx: WithOtherFields<RpcTransaction>) -> Result<Self, Self::Error> {
        if tx.transaction_type.is_some_and(|t| t == 0x7E) {
            let mint = tx
                .other
                .get_deserialized::<U256>("mint")
                .ok_or(ConversionError::Custom("MissingMint".to_string()))?
                .map_err(|_| ConversionError::Custom("Cannot deserialize mint".to_string()))?;

            let source_hash = tx
                .other
                .get_deserialized::<B256>("sourceHash")
                .ok_or(ConversionError::Custom("MissingSourceHash".to_string()))?
                .map_err(|_| {
                    ConversionError::Custom("Cannot deserialize source hash".to_string())
                })?;

            let deposit = DepositTransaction {
                nonce: tx.nonce,
                is_system_tx: true,
                from: tx.from,
                kind: tx.to.map(TxKind::Call).unwrap_or(TxKind::Create),
                value: tx.value,
                gas_limit: tx.gas,
                input: tx.input.clone(),
                mint,
                source_hash,
            };

            return Ok(Self::Deposit(deposit));
        }

        let tx = tx.inner;
        match tx.transaction_type.unwrap_or_default().try_into()? {
            TxType::Legacy => {
                let legacy = TxLegacy {
                    chain_id: tx.chain_id,
                    nonce: tx.nonce,
                    gas_price: tx.gas_price.ok_or(ConversionError::MissingGasPrice)?,
                    gas_limit: tx.gas,
                    value: tx.value,
                    input: tx.input,
                    to: tx.to.map_or(TxKind::Create, TxKind::Call),
                };
                let signature = tx
                    .signature
                    .ok_or(ConversionError::MissingSignature)?
                    .try_into()
                    .map_err(ConversionError::SignatureError)?;
                Ok(Self::Legacy(Signed::new_unchecked(legacy, signature, tx.hash)))
            }
            TxType::Eip1559 => {
                let eip1559 = TxEip1559 {
                    chain_id: tx.chain_id.ok_or(ConversionError::MissingChainId)?,
                    nonce: tx.nonce,
                    max_fee_per_gas: tx
                        .max_fee_per_gas
                        .ok_or(ConversionError::MissingMaxFeePerGas)?,
                    max_priority_fee_per_gas: tx
                        .max_priority_fee_per_gas
                        .ok_or(ConversionError::MissingMaxPriorityFeePerGas)?,
                    gas_limit: tx.gas,
                    value: tx.value,
                    input: tx.input,
                    to: tx.to.map_or(TxKind::Create, TxKind::Call),
                    access_list: tx.access_list.ok_or(ConversionError::MissingAccessList)?,
                };
                let signature = tx
                    .signature
                    .ok_or(ConversionError::MissingSignature)?
                    .try_into()
                    .map_err(ConversionError::SignatureError)?;
                Ok(Self::EIP1559(Signed::new_unchecked(eip1559, signature, tx.hash)))
            }
            TxType::Eip2930 => {
                let eip2930 = TxEip2930 {
                    chain_id: tx.chain_id.ok_or(ConversionError::MissingChainId)?,
                    nonce: tx.nonce,
                    gas_price: tx.gas_price.ok_or(ConversionError::MissingGasPrice)?,
                    gas_limit: tx.gas,
                    value: tx.value,
                    input: tx.input,
                    to: tx.to.map_or(TxKind::Create, TxKind::Call),
                    access_list: tx.access_list.ok_or(ConversionError::MissingAccessList)?,
                };
                let signature = tx
                    .signature
                    .ok_or(ConversionError::MissingSignature)?
                    .try_into()
                    .map_err(ConversionError::SignatureError)?;
                Ok(Self::EIP2930(Signed::new_unchecked(eip2930, signature, tx.hash)))
            }
            TxType::Eip4844 => {
                let eip4844 = TxEip4844 {
                    chain_id: tx.chain_id.ok_or(ConversionError::MissingChainId)?,
                    nonce: tx.nonce,
                    gas_limit: tx.gas,
                    max_fee_per_gas: tx.gas_price.ok_or(ConversionError::MissingGasPrice)?,
                    max_priority_fee_per_gas: tx
                        .max_priority_fee_per_gas
                        .ok_or(ConversionError::MissingMaxPriorityFeePerGas)?,
                    max_fee_per_blob_gas: tx
                        .max_fee_per_blob_gas
                        .ok_or(ConversionError::MissingMaxFeePerBlobGas)?,
                    to: tx.to.ok_or(ConversionError::MissingTo)?,
                    value: tx.value,
                    access_list: tx.access_list.ok_or(ConversionError::MissingAccessList)?,
                    blob_versioned_hashes: tx
                        .blob_versioned_hashes
                        .ok_or(ConversionError::MissingBlobVersionedHashes)?,
                    input: tx.input,
                };
                Ok(Self::EIP4844(Signed::new_unchecked(
                    TxEip4844Variant::TxEip4844(eip4844),
                    tx.signature
                        .ok_or(ConversionError::MissingSignature)?
                        .try_into()
                        .map_err(ConversionError::SignatureError)?,
                    tx.hash,
                )))
            }
            TxType::Eip7702 => {
                let eip7702 = TxEip7702 {
                    chain_id: tx.chain_id.ok_or(ConversionError::MissingChainId)?,
                    nonce: tx.nonce,
                    gas_limit: tx.gas,
                    max_fee_per_gas: tx.gas_price.ok_or(ConversionError::MissingGasPrice)?,
                    max_priority_fee_per_gas: tx
                        .max_priority_fee_per_gas
                        .ok_or(ConversionError::MissingMaxPriorityFeePerGas)?,
                    to: tx.to.ok_or(ConversionError::MissingTo)?,
                    value: tx.value,
                    access_list: tx.access_list.ok_or(ConversionError::MissingAccessList)?,
                    input: tx.input,
                    authorization_list: tx.authorization_list.unwrap_or_default(),
                };
                let signature = tx
                    .signature
                    .ok_or(ConversionError::MissingSignature)?
                    .try_into()
                    .map_err(ConversionError::SignatureError)?;
                Ok(Self::EIP7702(Signed::new_unchecked(eip7702, signature, tx.hash)))
            }
        }
    }
}

impl Encodable for TypedTransaction {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        if !self.is_legacy() {
            Header { list: false, payload_length: self.encode_2718_len() }.encode(out);
        }

        self.encode_2718(out);
    }
}

impl Decodable for TypedTransaction {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let mut h_decode_copy = *buf;
        let header = alloy_rlp::Header::decode(&mut h_decode_copy)?;

        // Legacy TX
        if header.list {
            return Ok(TxEnvelope::decode(buf)?.into());
        }

        // Check byte after header
        let ty = *h_decode_copy.first().ok_or(alloy_rlp::Error::Custom("empty slice"))?;

        if ty != 0x7E {
            Ok(TxEnvelope::decode(buf)?.into())
        } else {
            Ok(Self::Deposit(DepositTransaction::decode_2718(buf)?))
        }
    }
}

impl Encodable2718 for TypedTransaction {
    fn type_flag(&self) -> Option<u8> {
        self.r#type()
    }

    fn encode_2718_len(&self) -> usize {
        match self {
            Self::Legacy(tx) => TxEnvelope::from(tx.clone()).encode_2718_len(),
            Self::EIP2930(tx) => TxEnvelope::from(tx.clone()).encode_2718_len(),
            Self::EIP1559(tx) => TxEnvelope::from(tx.clone()).encode_2718_len(),
            Self::EIP4844(tx) => TxEnvelope::from(tx.clone()).encode_2718_len(),
            Self::EIP7702(tx) => {
                let payload_length = tx.tx().fields_len() + tx.signature().rlp_vrs_len();
                Header { list: true, payload_length }.length() + payload_length + 1
            }
            Self::Deposit(tx) => 1 + tx.length(),
        }
    }

    fn encode_2718(&self, out: &mut dyn BufMut) {
        match self {
            Self::Legacy(tx) => TxEnvelope::from(tx.clone()).encode_2718(out),
            Self::EIP2930(tx) => TxEnvelope::from(tx.clone()).encode_2718(out),
            Self::EIP1559(tx) => TxEnvelope::from(tx.clone()).encode_2718(out),
            Self::EIP4844(tx) => TxEnvelope::from(tx.clone()).encode_2718(out),
            Self::EIP7702(tx) => tx.tx().encode_with_signature(tx.signature(), out, false),
            Self::Deposit(tx) => {
                tx.encode_2718(out);
            }
        }
    }
}

impl Decodable2718 for TypedTransaction {
    fn typed_decode(ty: u8, buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        match ty {
            0x04 => return Ok(Self::EIP7702(TxEip7702::decode_signed_fields(buf)?)),
            0x7E => return Ok(Self::Deposit(DepositTransaction::decode(buf)?)),
            _ => {}
        }
        match TxEnvelope::typed_decode(ty, buf)? {
            TxEnvelope::Eip2930(tx) => Ok(Self::EIP2930(tx)),
            TxEnvelope::Eip1559(tx) => Ok(Self::EIP1559(tx)),
            TxEnvelope::Eip4844(tx) => Ok(Self::EIP4844(tx)),
            _ => unreachable!(),
        }
    }

    fn fallback_decode(buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        match TxEnvelope::fallback_decode(buf)? {
            TxEnvelope::Legacy(tx) => Ok(Self::Legacy(tx)),
            _ => unreachable!(),
        }
    }
}

impl From<TxEnvelope> for TypedTransaction {
    fn from(value: TxEnvelope) -> Self {
        match value {
            TxEnvelope::Legacy(tx) => Self::Legacy(tx),
            TxEnvelope::Eip2930(tx) => Self::EIP2930(tx),
            TxEnvelope::Eip1559(tx) => Self::EIP1559(tx),
            TxEnvelope::Eip4844(tx) => Self::EIP4844(tx),
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionEssentials {
    pub kind: TxKind,
    pub input: Bytes,
    pub nonce: u64,
    pub gas_limit: u128,
    pub gas_price: Option<u128>,
    pub max_fee_per_gas: Option<u128>,
    pub max_priority_fee_per_gas: Option<u128>,
    pub max_fee_per_blob_gas: Option<u128>,
    pub blob_versioned_hashes: Option<Vec<B256>>,
    pub value: U256,
    pub chain_id: Option<u64>,
    pub access_list: AccessList,
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
    pub gas_used: u128,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DepositReceipt<T = alloy_primitives::Log> {
    #[serde(flatten)]
    pub inner: ReceiptWithBloom<T>,
    #[serde(default, with = "alloy_serde::num::u64_opt_via_ruint")]
    pub deposit_nonce: Option<u64>,
    #[serde(default, with = "alloy_serde::num::u64_opt_via_ruint")]
    pub deposit_receipt_version: Option<u64>,
}

impl DepositReceipt {
    fn payload_len(&self) -> usize {
        self.inner.receipt.status.length() +
            self.inner.receipt.cumulative_gas_used.length() +
            self.inner.logs_bloom.length() +
            self.inner.receipt.logs.length() +
            self.deposit_nonce.map_or(0, |n| n.length()) +
            self.deposit_receipt_version.map_or(0, |n| n.length())
    }

    /// Returns the rlp header for the receipt payload.
    fn receipt_rlp_header(&self) -> alloy_rlp::Header {
        alloy_rlp::Header { list: true, payload_length: self.payload_len() }
    }

    /// Encodes the receipt data.
    fn encode_fields(&self, out: &mut dyn BufMut) {
        self.receipt_rlp_header().encode(out);
        self.inner.receipt.status.encode(out);
        self.inner.receipt.cumulative_gas_used.encode(out);
        self.inner.logs_bloom.encode(out);
        self.inner.receipt.logs.encode(out);
        if let Some(n) = self.deposit_nonce {
            n.encode(out);
        }
        if let Some(n) = self.deposit_receipt_version {
            n.encode(out);
        }
    }

    /// Decodes the receipt payload
    fn decode_receipt(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let b: &mut &[u8] = &mut &**buf;
        let rlp_head = alloy_rlp::Header::decode(b)?;
        if !rlp_head.list {
            return Err(alloy_rlp::Error::UnexpectedString);
        }
        let started_len = b.len();
        let remaining = |b: &[u8]| rlp_head.payload_length - (started_len - b.len()) > 0;

        let status = Decodable::decode(b)?;
        let cumulative_gas_used = Decodable::decode(b)?;
        let logs_bloom = Decodable::decode(b)?;
        let logs = Decodable::decode(b)?;
        let deposit_nonce = remaining(b).then(|| alloy_rlp::Decodable::decode(b)).transpose()?;
        let deposit_nonce_version =
            remaining(b).then(|| alloy_rlp::Decodable::decode(b)).transpose()?;

        let this = Self {
            inner: ReceiptWithBloom {
                receipt: Receipt { status, cumulative_gas_used, logs },
                logs_bloom,
            },
            deposit_nonce,
            deposit_receipt_version: deposit_nonce_version,
        };

        let consumed = started_len - b.len();
        if consumed != rlp_head.payload_length {
            return Err(alloy_rlp::Error::ListLengthMismatch {
                expected: rlp_head.payload_length,
                got: consumed,
            });
        }

        *buf = *b;
        Ok(this)
    }
}

impl alloy_rlp::Encodable for DepositReceipt {
    fn encode(&self, out: &mut dyn BufMut) {
        self.encode_fields(out);
    }

    fn length(&self) -> usize {
        let payload_length = self.payload_len();
        payload_length + length_of_length(payload_length)
    }
}

impl alloy_rlp::Decodable for DepositReceipt {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        Self::decode_receipt(buf)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TypedReceipt<T = alloy_primitives::Log> {
    #[serde(rename = "0x0", alias = "0x00")]
    Legacy(ReceiptWithBloom<T>),
    #[serde(rename = "0x1", alias = "0x01")]
    EIP2930(ReceiptWithBloom<T>),
    #[serde(rename = "0x2", alias = "0x02")]
    EIP1559(ReceiptWithBloom<T>),
    #[serde(rename = "0x3", alias = "0x03")]
    EIP4844(ReceiptWithBloom<T>),
    #[serde(rename = "0x4", alias = "0x04")]
    EIP7702(ReceiptWithBloom<T>),
    #[serde(rename = "0x7E", alias = "0x7e")]
    Deposit(DepositReceipt<T>),
}

impl<T> TypedReceipt<T> {
    pub fn as_receipt_with_bloom(&self) -> &ReceiptWithBloom<T> {
        match self {
            Self::Legacy(r) |
            Self::EIP1559(r) |
            Self::EIP2930(r) |
            Self::EIP4844(r) |
            Self::EIP7702(r) => r,
            Self::Deposit(r) => &r.inner,
        }
    }
}

impl<T> From<TypedReceipt<T>> for ReceiptWithBloom<T> {
    fn from(value: TypedReceipt<T>) -> Self {
        match value {
            TypedReceipt::Legacy(r) |
            TypedReceipt::EIP1559(r) |
            TypedReceipt::EIP2930(r) |
            TypedReceipt::EIP4844(r) |
            TypedReceipt::EIP7702(r) => r,
            TypedReceipt::Deposit(r) => r.inner,
        }
    }
}

impl From<TypedReceipt<alloy_rpc_types::Log>> for OtsReceipt {
    fn from(value: TypedReceipt<alloy_rpc_types::Log>) -> Self {
        let r#type = match value {
            TypedReceipt::Legacy(_) => 0x00,
            TypedReceipt::EIP2930(_) => 0x01,
            TypedReceipt::EIP1559(_) => 0x02,
            TypedReceipt::EIP4844(_) => 0x03,
            TypedReceipt::EIP7702(_) => 0x04,
            TypedReceipt::Deposit(_) => 0x7E,
        } as u8;
        let receipt = ReceiptWithBloom::<alloy_rpc_types::Log>::from(value);
        let status = receipt.status();
        let cumulative_gas_used = receipt.cumulative_gas_used() as u64;
        let logs = receipt.logs().to_vec();
        let logs_bloom = receipt.logs_bloom;

        Self { status, cumulative_gas_used, logs: Some(logs), logs_bloom: Some(logs_bloom), r#type }
    }
}

impl TypedReceipt {
    pub fn cumulative_gas_used(&self) -> u128 {
        self.as_receipt_with_bloom().cumulative_gas_used()
    }

    pub fn logs_bloom(&self) -> &Bloom {
        &self.as_receipt_with_bloom().logs_bloom
    }

    pub fn logs(&self) -> &[Log] {
        self.as_receipt_with_bloom().logs()
    }
}

impl From<ReceiptEnvelope<alloy_rpc_types::Log>> for TypedReceipt<alloy_rpc_types::Log> {
    fn from(value: ReceiptEnvelope<alloy_rpc_types::Log>) -> Self {
        match value {
            ReceiptEnvelope::Legacy(r) => Self::Legacy(r),
            ReceiptEnvelope::Eip2930(r) => Self::EIP2930(r),
            ReceiptEnvelope::Eip1559(r) => Self::EIP1559(r),
            ReceiptEnvelope::Eip4844(r) => Self::EIP4844(r),
            _ => unreachable!(),
        }
    }
}

impl Encodable for TypedReceipt {
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        match self {
            Self::Legacy(r) => r.encode(out),
            receipt => {
                let payload_len = match receipt {
                    Self::EIP2930(r) => r.length() + 1,
                    Self::EIP1559(r) => r.length() + 1,
                    Self::EIP4844(r) => r.length() + 1,
                    Self::Deposit(r) => r.length() + 1,
                    _ => unreachable!("receipt already matched"),
                };

                match receipt {
                    Self::EIP2930(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        1u8.encode(out);
                        r.encode(out);
                    }
                    Self::EIP1559(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        2u8.encode(out);
                        r.encode(out);
                    }
                    Self::EIP4844(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        3u8.encode(out);
                        r.encode(out);
                    }
                    Self::Deposit(r) => {
                        Header { list: true, payload_length: payload_len }.encode(out);
                        0x7Eu8.encode(out);
                        r.encode(out);
                    }
                    _ => unreachable!("receipt already matched"),
                }
            }
        }
    }
}

impl Decodable for TypedReceipt {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        use bytes::Buf;
        use std::cmp::Ordering;

        // a receipt is either encoded as a string (non legacy) or a list (legacy).
        // We should not consume the buffer if we are decoding a legacy receipt, so let's
        // check if the first byte is between 0x80 and 0xbf.
        let rlp_type = *buf
            .first()
            .ok_or(alloy_rlp::Error::Custom("cannot decode a receipt from empty bytes"))?;

        match rlp_type.cmp(&alloy_rlp::EMPTY_LIST_CODE) {
            Ordering::Less => {
                // strip out the string header
                let _header = Header::decode(buf)?;
                let receipt_type = *buf.first().ok_or(alloy_rlp::Error::Custom(
                    "typed receipt cannot be decoded from an empty slice",
                ))?;
                if receipt_type == 0x01 {
                    buf.advance(1);
                    <ReceiptWithBloom as Decodable>::decode(buf).map(TypedReceipt::EIP2930)
                } else if receipt_type == 0x02 {
                    buf.advance(1);
                    <ReceiptWithBloom as Decodable>::decode(buf).map(TypedReceipt::EIP1559)
                } else if receipt_type == 0x03 {
                    buf.advance(1);
                    <ReceiptWithBloom as Decodable>::decode(buf).map(TypedReceipt::EIP4844)
                } else if receipt_type == 0x7E {
                    buf.advance(1);
                    <DepositReceipt as Decodable>::decode(buf).map(TypedReceipt::Deposit)
                } else {
                    Err(alloy_rlp::Error::Custom("invalid receipt type"))
                }
            }
            Ordering::Equal => {
                Err(alloy_rlp::Error::Custom("an empty list is not a valid receipt encoding"))
            }
            Ordering::Greater => {
                <ReceiptWithBloom as Decodable>::decode(buf).map(TypedReceipt::Legacy)
            }
        }
    }
}

impl Encodable2718 for TypedReceipt {
    fn type_flag(&self) -> Option<u8> {
        match self {
            Self::Legacy(_) => None,
            Self::EIP2930(_) => Some(1),
            Self::EIP1559(_) => Some(2),
            Self::EIP4844(_) => Some(3),
            Self::EIP7702(_) => Some(4),
            Self::Deposit(_) => Some(0x7E),
        }
    }

    fn encode_2718_len(&self) -> usize {
        match self {
            Self::Legacy(r) => ReceiptEnvelope::Legacy(r.clone()).encode_2718_len(),
            Self::EIP2930(r) => ReceiptEnvelope::Eip2930(r.clone()).encode_2718_len(),
            Self::EIP1559(r) => ReceiptEnvelope::Eip1559(r.clone()).encode_2718_len(),
            Self::EIP4844(r) => ReceiptEnvelope::Eip4844(r.clone()).encode_2718_len(),
            Self::EIP7702(r) => 1 + r.length(),
            Self::Deposit(r) => 1 + r.length(),
        }
    }

    fn encode_2718(&self, out: &mut dyn BufMut) {
        if let Some(ty) = self.type_flag() {
            out.put_u8(ty);
        }
        match self {
            Self::Legacy(r) |
            Self::EIP2930(r) |
            Self::EIP1559(r) |
            Self::EIP4844(r) |
            Self::EIP7702(r) => r.encode(out),
            Self::Deposit(r) => r.encode(out),
        }
    }
}

impl Decodable2718 for TypedReceipt {
    fn typed_decode(ty: u8, buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        if ty == 0x7E {
            return Ok(Self::Deposit(DepositReceipt::decode(buf)?));
        }
        match ReceiptEnvelope::typed_decode(ty, buf)? {
            ReceiptEnvelope::Eip2930(tx) => Ok(Self::EIP2930(tx)),
            ReceiptEnvelope::Eip1559(tx) => Ok(Self::EIP1559(tx)),
            ReceiptEnvelope::Eip4844(tx) => Ok(Self::EIP4844(tx)),
            _ => unreachable!(),
        }
    }

    fn fallback_decode(buf: &mut &[u8]) -> Result<Self, Eip2718Error> {
        match ReceiptEnvelope::fallback_decode(buf)? {
            ReceiptEnvelope::Legacy(tx) => Ok(Self::Legacy(tx)),
            _ => unreachable!(),
        }
    }
}

pub type ReceiptResponse = TransactionReceipt<TypedReceipt<alloy_rpc_types::Log>>;

pub fn convert_to_anvil_receipt(receipt: AnyTransactionReceipt) -> Option<ReceiptResponse> {
    let WithOtherFields {
        inner:
            TransactionReceipt {
                transaction_hash,
                transaction_index,
                block_hash,
                block_number,
                gas_used,
                contract_address,
                effective_gas_price,
                from,
                to,
                blob_gas_price,
                blob_gas_used,
                state_root,
                inner: AnyReceiptEnvelope { inner: receipt_with_bloom, r#type },
                authorization_list,
            },
        other,
    } = receipt;

    Some(TransactionReceipt {
        transaction_hash,
        transaction_index,
        block_hash,
        block_number,
        gas_used,
        contract_address,
        effective_gas_price,
        from,
        to,
        blob_gas_price,
        blob_gas_used,
        state_root,
        authorization_list,
        inner: match r#type {
            0x00 => TypedReceipt::Legacy(receipt_with_bloom),
            0x01 => TypedReceipt::EIP2930(receipt_with_bloom),
            0x02 => TypedReceipt::EIP1559(receipt_with_bloom),
            0x03 => TypedReceipt::EIP4844(receipt_with_bloom),
            0x7E => TypedReceipt::Deposit(DepositReceipt {
                inner: receipt_with_bloom,
                deposit_nonce: other
                    .get_deserialized::<U64>("depositNonce")
                    .transpose()
                    .ok()?
                    .map(|v| v.to()),
                deposit_receipt_version: other
                    .get_deserialized::<U64>("depositReceiptVersion")
                    .transpose()
                    .ok()?
                    .map(|v| v.to()),
            }),
            _ => return None,
        },
    })
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{b256, hex, LogData};
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_decode_call() {
        let bytes_first = &mut &hex::decode("f86b02843b9aca00830186a094d3e8763675e4c425df46cc3b5c0f6cbdac39604687038d7ea4c68000802ba00eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5aea03a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca18").unwrap()[..];
        let decoded = TypedTransaction::decode(&mut &bytes_first[..]).unwrap();

        let tx = TxLegacy {
            nonce: 2u64,
            gas_price: 1000000000u128,
            gas_limit: 100000u128,
            to: TxKind::Call(Address::from_slice(
                &hex::decode("d3e8763675e4c425df46cc3b5c0f6cbdac396046").unwrap()[..],
            )),
            value: U256::from(1000000000000000u64),
            input: Bytes::default(),
            chain_id: Some(4),
        };

        let signature = Signature::from_str("0eb96ca19e8a77102767a41fc85a36afd5c61ccb09911cec5d3e86e193d9c5ae3a456401896b1b6055311536bf00a718568c744d8c1f9df59879e8350220ca182b").unwrap();

        let tx = TypedTransaction::Legacy(Signed::new_unchecked(
            tx,
            signature,
            b256!("a517b206d2223278f860ea017d3626cacad4f52ff51030dc9a96b432f17f8d34"),
        ));

        assert_eq!(tx, decoded);
    }

    #[test]
    fn test_decode_create_goerli() {
        // test that an example create tx from goerli decodes properly
        let tx_bytes =
              hex::decode("02f901ee05228459682f008459682f11830209bf8080b90195608060405234801561001057600080fd5b50610175806100206000396000f3fe608060405234801561001057600080fd5b506004361061002b5760003560e01c80630c49c36c14610030575b600080fd5b61003861004e565b604051610045919061011d565b60405180910390f35b60606020600052600f6020527f68656c6c6f2073746174656d696e64000000000000000000000000000000000060405260406000f35b600081519050919050565b600082825260208201905092915050565b60005b838110156100be5780820151818401526020810190506100a3565b838111156100cd576000848401525b50505050565b6000601f19601f8301169050919050565b60006100ef82610084565b6100f9818561008f565b93506101098185602086016100a0565b610112816100d3565b840191505092915050565b6000602082019050818103600083015261013781846100e4565b90509291505056fea264697066735822122051449585839a4ea5ac23cae4552ef8a96b64ff59d0668f76bfac3796b2bdbb3664736f6c63430008090033c080a0136ebffaa8fc8b9fda9124de9ccb0b1f64e90fbd44251b4c4ac2501e60b104f9a07eb2999eec6d185ef57e91ed099afb0a926c5b536f0155dd67e537c7476e1471")
                  .unwrap();
        let _decoded = TypedTransaction::decode(&mut &tx_bytes[..]).unwrap();
    }

    #[test]
    fn can_recover_sender() {
        // random mainnet tx: https://etherscan.io/tx/0x86718885c4b4218c6af87d3d0b0d83e3cc465df2a05c048aa4db9f1a6f9de91f
        let bytes = hex::decode("02f872018307910d808507204d2cb1827d0094388c818ca8b9251b393131c08a736a67ccb19297880320d04823e2701c80c001a0cf024f4815304df2867a1a74e9d2707b6abda0337d2d54a4438d453f4160f190a07ac0e6b3bc9395b5b9c8b9e6d77204a236577a5b18467b9175c01de4faa208d9").unwrap();

        let Ok(TypedTransaction::EIP1559(tx)) = TypedTransaction::decode(&mut &bytes[..]) else {
            panic!("decoding TypedTransaction failed");
        };

        assert_eq!(
            tx.hash(),
            &"0x86718885c4b4218c6af87d3d0b0d83e3cc465df2a05c048aa4db9f1a6f9de91f"
                .parse::<B256>()
                .unwrap()
        );
        assert_eq!(
            tx.recover_signer().unwrap(),
            "0x95222290DD7278Aa3Ddd389Cc1E1d165CC4BAfe5".parse::<Address>().unwrap()
        );
    }

    // Test vector from https://sepolia.etherscan.io/tx/0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
    // Blobscan: https://sepolia.blobscan.com/tx/0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
    #[test]
    fn test_decode_live_4844_tx() {
        use alloy_primitives::{address, b256};

        // https://sepolia.etherscan.io/getRawTx?tx=0x9a22ccb0029bc8b0ddd073be1a1d923b7ae2b2ea52100bae0db4424f9107e9c0
        let raw_tx = alloy_primitives::hex::decode("0x03f9011d83aa36a7820fa28477359400852e90edd0008252089411e9ca82a3a762b4b5bd264d4173a242e7a770648080c08504a817c800f8a5a0012ec3d6f66766bedb002a190126b3549fce0047de0d4c25cffce0dc1c57921aa00152d8e24762ff22b1cfd9f8c0683786a7ca63ba49973818b3d1e9512cd2cec4a0013b98c6c83e066d5b14af2b85199e3d4fc7d1e778dd53130d180f5077e2d1c7a001148b495d6e859114e670ca54fb6e2657f0cbae5b08063605093a4b3dc9f8f1a0011ac212f13c5dff2b2c6b600a79635103d6f580a4221079951181b25c7e654901a0c8de4cced43169f9aa3d36506363b2d2c44f6c49fc1fd91ea114c86f3757077ea01e11fdd0d1934eda0492606ee0bb80a7bf8f35cc5f86ec60fe5031ba48bfd544").unwrap();
        let res = TypedTransaction::decode(&mut raw_tx.as_slice()).unwrap();
        assert_eq!(res.r#type(), Some(3));

        let tx = match res {
            TypedTransaction::EIP4844(tx) => tx,
            _ => unreachable!(),
        };

        assert_eq!(tx.tx().tx().to, address!("11E9CA82A3a762b4B5bd264d4173a242e7a77064"));

        assert_eq!(
            tx.tx().tx().blob_versioned_hashes,
            vec![
                b256!("012ec3d6f66766bedb002a190126b3549fce0047de0d4c25cffce0dc1c57921a"),
                b256!("0152d8e24762ff22b1cfd9f8c0683786a7ca63ba49973818b3d1e9512cd2cec4"),
                b256!("013b98c6c83e066d5b14af2b85199e3d4fc7d1e778dd53130d180f5077e2d1c7"),
                b256!("01148b495d6e859114e670ca54fb6e2657f0cbae5b08063605093a4b3dc9f8f1"),
                b256!("011ac212f13c5dff2b2c6b600a79635103d6f580a4221079951181b25c7e6549")
            ]
        );

        let from = tx.recover_signer().unwrap();
        assert_eq!(from, address!("A83C816D4f9b2783761a22BA6FADB0eB0606D7B2"));
    }

    #[test]
    fn test_decode_encode_deposit_tx() {
        // https://sepolia-optimism.etherscan.io/tx/0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7
        let tx_hash: TxHash = "0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7"
            .parse::<TxHash>()
            .unwrap();

        // https://sepolia-optimism.etherscan.io/getRawTx?tx=0xbf8b5f08c43e4b860715cd64fc0849bbce0d0ea20a76b269e7bc8886d112fca7
        let raw_tx = alloy_primitives::hex::decode(
            "7ef861a0dfd7ae78bf3c414cfaa77f13c0205c82eb9365e217b2daa3448c3156b69b27ac94778f2146f48179643473b82931c4cd7b8f153efd94778f2146f48179643473b82931c4cd7b8f153efd872386f26fc10000872386f26fc10000830186a08080",
        )
        .unwrap();
        let dep_tx = TypedTransaction::decode(&mut raw_tx.as_slice()).unwrap();

        let mut encoded = Vec::new();
        dep_tx.encode_2718(&mut encoded);

        assert_eq!(raw_tx, encoded);

        assert_eq!(tx_hash, dep_tx.hash());
    }

    #[test]
    fn can_recover_sender_not_normalized() {
        let bytes = hex::decode("f85f800182520894095e7baea6a6c7c4c2dfeb977efac326af552d870a801ba048b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353a0efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c804").unwrap();

        let Ok(TypedTransaction::Legacy(tx)) = TypedTransaction::decode(&mut &bytes[..]) else {
            panic!("decoding TypedTransaction failed");
        };

        assert_eq!(tx.tx().input, Bytes::from(b""));
        assert_eq!(tx.tx().gas_price, 1);
        assert_eq!(tx.tx().gas_limit, 21000);
        assert_eq!(tx.tx().nonce, 0);
        if let TxKind::Call(to) = tx.tx().to {
            assert_eq!(
                to,
                "0x095e7baea6a6c7c4c2dfeb977efac326af552d87".parse::<Address>().unwrap()
            );
        } else {
            panic!("expected a call transaction");
        }
        assert_eq!(tx.tx().value, U256::from(0x0au64));
        assert_eq!(
            tx.recover_signer().unwrap(),
            "0f65fe9276bc9a24ae7083ae28e2660ef72df99e".parse::<Address>().unwrap()
        );
    }

    #[test]
    fn encode_legacy_receipt() {
        let expected = hex::decode("f901668001b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f85ff85d940000000000000000000000000000000000000011f842a0000000000000000000000000000000000000000000000000000000000000deada0000000000000000000000000000000000000000000000000000000000000beef830100ff").unwrap();

        let mut data = vec![];
        let receipt = TypedReceipt::Legacy(ReceiptWithBloom {
            receipt: Receipt {
                status: false.into(),
                cumulative_gas_used: 0x1u128,
                logs: vec![Log {
                    address: Address::from_str("0000000000000000000000000000000000000011").unwrap(),
                    data: LogData::new_unchecked(
                        vec![
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000dead",
                            )
                            .unwrap(),
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000beef",
                            )
                            .unwrap(),
                        ],
                        Bytes::from_str("0100ff").unwrap(),
                    ),
                }],
            },
            logs_bloom: [0; 256].into(),
        });

        receipt.encode(&mut data);

        // check that the rlp length equals the length of the expected rlp
        assert_eq!(receipt.length(), expected.len());
        assert_eq!(data, expected);
    }

    #[test]
    fn decode_legacy_receipt() {
        let data = hex::decode("f901668001b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000f85ff85d940000000000000000000000000000000000000011f842a0000000000000000000000000000000000000000000000000000000000000deada0000000000000000000000000000000000000000000000000000000000000beef830100ff").unwrap();

        let expected = TypedReceipt::Legacy(ReceiptWithBloom {
            receipt: Receipt {
                status: false.into(),
                cumulative_gas_used: 0x1u128,
                logs: vec![Log {
                    address: Address::from_str("0000000000000000000000000000000000000011").unwrap(),
                    data: LogData::new_unchecked(
                        vec![
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000dead",
                            )
                            .unwrap(),
                            B256::from_str(
                                "000000000000000000000000000000000000000000000000000000000000beef",
                            )
                            .unwrap(),
                        ],
                        Bytes::from_str("0100ff").unwrap(),
                    ),
                }],
            },
            logs_bloom: [0; 256].into(),
        });

        let receipt = TypedReceipt::decode(&mut &data[..]).unwrap();

        assert_eq!(receipt, expected);
    }
}
