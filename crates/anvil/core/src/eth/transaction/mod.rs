//! Transaction related types
use alloy_consensus::{
    Receipt, ReceiptEnvelope, ReceiptWithBloom, Signed, Transaction, TxEip1559, TxEip2930,
    TxEnvelope, TxLegacy, TxReceipt, Typed2718,
    transaction::{
        Recovered, TxEip7702,
        eip4844::{TxEip4844, TxEip4844Variant, TxEip4844WithSidecar},
    },
};

use alloy_eips::eip2718::Encodable2718;
use alloy_network::{AnyReceiptEnvelope, AnyTransactionReceipt};
use alloy_primitives::{Address, B256, Bloom, Bytes, TxHash, TxKind, U64, U256};
use alloy_rlp::{Decodable, Encodable};
use alloy_rpc_types::{
    Transaction as RpcTransaction, TransactionReceipt, request::TransactionRequest,
    trace::otterscan::OtsReceipt,
};
use alloy_serde::{OtherFields, WithOtherFields};
use bytes::BufMut;
use foundry_evm::traces::CallTraceNode;
use foundry_primitives::{FoundryTxEnvelope, FoundryTypedTx};
use op_alloy_consensus::{OpDepositReceipt, OpDepositReceiptWithBloom, TxDeposit};
use revm::interpreter::InstructionResult;
use serde::{Deserialize, Serialize};
use std::ops::Deref;

/// Converts a [TransactionRequest] into a [FoundryTypedTx].
/// Should be removed once the call builder abstraction for providers is in place.
pub fn transaction_request_to_typed(
    tx: WithOtherFields<TransactionRequest>,
) -> Option<FoundryTypedTx> {
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
                authorization_list,
                chain_id: _,
            },
        other,
    } = tx;

    // Special case: OP-stack deposit tx
    if transaction_type == Some(0x7E) || has_optimism_fields(&other) {
        let mint = other.get_deserialized::<U256>("mint")?.map(|m| m.to::<u128>()).ok()?;

        return Some(FoundryTypedTx::Deposit(TxDeposit {
            from: from.unwrap_or_default(),
            source_hash: other.get_deserialized::<B256>("sourceHash")?.ok()?,
            to: to.unwrap_or_default(),
            mint,
            value: value.unwrap_or_default(),
            gas_limit: gas.unwrap_or_default(),
            is_system_transaction: other.get_deserialized::<bool>("isSystemTx")?.ok()?,
            input: input.into_input().unwrap_or_default(),
        }));
    }

    // EIP7702
    if transaction_type == Some(4) || authorization_list.is_some() {
        return Some(FoundryTypedTx::Eip7702(TxEip7702 {
            nonce: nonce.unwrap_or_default(),
            max_fee_per_gas: max_fee_per_gas.unwrap_or_default(),
            max_priority_fee_per_gas: max_priority_fee_per_gas.unwrap_or_default(),
            gas_limit: gas.unwrap_or_default(),
            value: value.unwrap_or(U256::ZERO),
            input: input.into_input().unwrap_or_default(),
            // requires to
            to: to?.into_to()?,
            chain_id: 0,
            access_list: access_list.unwrap_or_default(),
            authorization_list: authorization_list.unwrap_or_default(),
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
        sidecar.as_ref(),
        to,
    ) {
        // legacy transaction
        (Some(0), _, None, None, None, None, None, None, _)
        | (None, Some(_), None, None, None, None, None, None, _) => {
            Some(FoundryTypedTx::Legacy(TxLegacy {
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
        (Some(1), _, None, None, _, None, None, None, _)
        | (None, _, None, None, Some(_), None, None, None, _) => {
            Some(FoundryTypedTx::Eip2930(TxEip2930 {
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
        (Some(2), None, _, _, _, _, None, None, _)
        | (None, None, Some(_), _, _, _, None, None, _)
        | (None, None, _, Some(_), _, _, None, None, _)
        | (None, None, None, None, None, _, None, None, _) => {
            // Empty fields fall back to the canonical transaction schema.
            Some(FoundryTypedTx::Eip1559(TxEip1559 {
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
        (Some(3), None, _, _, _, _, Some(_), _, to) => {
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

            if let Some(sidecar) = sidecar {
                Some(FoundryTypedTx::Eip4844(TxEip4844Variant::TxEip4844WithSidecar(
                    TxEip4844WithSidecar::from_tx_and_sidecar(tx, sidecar),
                )))
            } else {
                Some(FoundryTypedTx::Eip4844(TxEip4844Variant::TxEip4844(tx)))
            }
        }
        _ => None,
    }
}

pub fn has_optimism_fields(other: &OtherFields) -> bool {
    other.contains_key("sourceHash")
        && other.contains_key("mint")
        && other.contains_key("isSystemTx")
}

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
    pub fn recover(&self) -> Result<Address, alloy_primitives::SignatureError> {
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
    pub fn new(transaction: FoundryTxEnvelope) -> Result<Self, alloy_primitives::SignatureError> {
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
    ) -> Result<Self, alloy_primitives::SignatureError> {
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

/// RPC-specific variant of TypedReceipt for boundary conversion
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TypedReceiptRpc {
    #[serde(rename = "0x0", alias = "0x00")]
    Legacy(ReceiptWithBloom<Receipt<alloy_rpc_types::Log>>),
    #[serde(rename = "0x1", alias = "0x01")]
    Eip2930(ReceiptWithBloom<Receipt<alloy_rpc_types::Log>>),
    #[serde(rename = "0x2", alias = "0x02")]
    Eip1559(ReceiptWithBloom<Receipt<alloy_rpc_types::Log>>),
    #[serde(rename = "0x3", alias = "0x03")]
    Eip4844(ReceiptWithBloom<Receipt<alloy_rpc_types::Log>>),
    #[serde(rename = "0x4", alias = "0x04")]
    Eip7702(ReceiptWithBloom<Receipt<alloy_rpc_types::Log>>),
    #[serde(rename = "0x7E", alias = "0x7e")]
    Deposit(OpDepositReceiptWithBloom),
}

impl TypedReceiptRpc {
    pub fn as_receipt_with_bloom(&self) -> &ReceiptWithBloom<Receipt<alloy_rpc_types::Log>> {
        match self {
            Self::Legacy(r)
            | Self::Eip1559(r)
            | Self::Eip2930(r)
            | Self::Eip4844(r)
            | Self::Eip7702(r) => r,
            Self::Deposit(_) => unreachable!("use variant-specific helpers for deposit"),
        }
    }

    pub fn logs_bloom(&self) -> &Bloom {
        match self {
            Self::Legacy(r)
            | Self::Eip1559(r)
            | Self::Eip2930(r)
            | Self::Eip4844(r)
            | Self::Eip7702(r) => &r.logs_bloom,
            Self::Deposit(r) => &r.logs_bloom,
        }
    }

    pub fn logs(&self) -> &[alloy_rpc_types::Log] {
        match self {
            Self::Legacy(r)
            | Self::Eip1559(r)
            | Self::Eip2930(r)
            | Self::Eip4844(r)
            | Self::Eip7702(r) => &r.receipt.logs,
            Self::Deposit(_) => unreachable!("use variant-specific helpers for deposit"),
        }
    }

    pub fn cumulative_gas_used(&self) -> u64 {
        self.as_receipt_with_bloom().cumulative_gas_used()
    }
}

// Intentionally only provide a concrete conversion used by RPC response/Otterscan path.
impl From<TypedReceiptRpc> for ReceiptWithBloom<Receipt<alloy_rpc_types::Log>> {
    fn from(value: TypedReceiptRpc) -> Self {
        match value {
            TypedReceiptRpc::Legacy(r)
            | TypedReceiptRpc::Eip1559(r)
            | TypedReceiptRpc::Eip2930(r)
            | TypedReceiptRpc::Eip4844(r)
            | TypedReceiptRpc::Eip7702(r) => r,
            TypedReceiptRpc::Deposit(r) => {
                // Convert OP deposit receipt (primitives::Log) to RPC receipt (rpc_types::Log)
                let receipt = Receipt::<alloy_rpc_types::Log> {
                    status: r.receipt.inner.status,
                    cumulative_gas_used: r.receipt.inner.cumulative_gas_used,
                    logs: r
                        .receipt
                        .inner
                        .logs
                        .into_iter()
                        .map(|l| alloy_rpc_types::Log {
                            inner: l,
                            block_hash: None,
                            block_number: None,
                            block_timestamp: None,
                            transaction_hash: None,
                            transaction_index: None,
                            log_index: None,
                            removed: false,
                        })
                        .collect(),
                };
                Self { receipt, logs_bloom: r.logs_bloom }
            }
        }
    }
}

impl From<TypedReceiptRpc> for OtsReceipt {
    fn from(value: TypedReceiptRpc) -> Self {
        let r#type = match value {
            TypedReceiptRpc::Legacy(_) => 0x00,
            TypedReceiptRpc::Eip2930(_) => 0x01,
            TypedReceiptRpc::Eip1559(_) => 0x02,
            TypedReceiptRpc::Eip4844(_) => 0x03,
            TypedReceiptRpc::Eip7702(_) => 0x04,
            TypedReceiptRpc::Deposit(_) => 0x7E,
        } as u8;
        let receipt = ReceiptWithBloom::<Receipt<alloy_rpc_types::Log>>::from(value);
        let status = receipt.status();
        let cumulative_gas_used = receipt.cumulative_gas_used();
        let logs = receipt.logs().to_vec();
        let logs_bloom = receipt.logs_bloom;

        Self { status, cumulative_gas_used, logs: Some(logs), logs_bloom: Some(logs_bloom), r#type }
    }
}

impl From<ReceiptEnvelope<alloy_rpc_types::Log>> for TypedReceiptRpc {
    fn from(value: ReceiptEnvelope<alloy_rpc_types::Log>) -> Self {
        match value {
            ReceiptEnvelope::Legacy(r) => Self::Legacy(r),
            ReceiptEnvelope::Eip2930(r) => Self::Eip2930(r),
            ReceiptEnvelope::Eip1559(r) => Self::Eip1559(r),
            ReceiptEnvelope::Eip4844(r) => Self::Eip4844(r),
            ReceiptEnvelope::Eip7702(r) => Self::Eip7702(r),
        }
    }
}

pub type ReceiptResponse = WithOtherFields<TransactionReceipt<TypedReceiptRpc>>;

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
                inner: AnyReceiptEnvelope { inner: receipt_with_bloom, r#type },
            },
        other,
    } = receipt;

    Some(WithOtherFields {
        inner: TransactionReceipt {
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
            inner: match r#type {
                0x00 => TypedReceiptRpc::Legacy(receipt_with_bloom),
                0x01 => TypedReceiptRpc::Eip2930(receipt_with_bloom),
                0x02 => TypedReceiptRpc::Eip1559(receipt_with_bloom),
                0x03 => TypedReceiptRpc::Eip4844(receipt_with_bloom),
                0x04 => TypedReceiptRpc::Eip7702(receipt_with_bloom),
                0x7E => TypedReceiptRpc::Deposit(OpDepositReceiptWithBloom {
                    receipt: OpDepositReceipt {
                        inner: Receipt {
                            status: alloy_consensus::Eip658Value::Eip658(
                                receipt_with_bloom.status(),
                            ),
                            cumulative_gas_used: receipt_with_bloom.cumulative_gas_used(),
                            logs: receipt_with_bloom
                                .receipt
                                .logs
                                .into_iter()
                                .map(|l| l.inner)
                                .collect(),
                        },
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
                    },
                    logs_bloom: receipt_with_bloom.logs_bloom,
                }),
                _ => return None,
            },
        },
        other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // <https://github.com/foundry-rs/foundry/issues/10852>
    #[test]
    fn test_receipt_convert() {
        let s = r#"{"type":"0x4","status":"0x1","cumulativeGasUsed":"0x903fd1","logs":[{"address":"0x0000d9fcd47bf761e7287d8ee09917d7e2100000","topics":["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef","0x0000000000000000000000000000000000000000000000000000000000000000","0x000000000000000000000000234ce51365b9c417171b6dad280f49143e1b0547"],"data":"0x00000000000000000000000000000000000000000000032139b42c3431700000","blockHash":"0xd26b59c1d8b5bfa9362d19eb0da3819dfe0b367987a71f6d30908dd45e0d7a60","blockNumber":"0x159663e","blockTimestamp":"0x68411f7b","transactionHash":"0x17a6af73d1317e69cfc3cac9221bd98261d40f24815850a44dbfbf96652ae52a","transactionIndex":"0x22","logIndex":"0x158","removed":false}],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000000000000000008100000000000000000000000000000000000000000000000020000200000000000000800000000800000000000000010000000000000000002000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000000","transactionHash":"0x17a6af73d1317e69cfc3cac9221bd98261d40f24815850a44dbfbf96652ae52a","transactionIndex":"0x22","blockHash":"0xd26b59c1d8b5bfa9362d19eb0da3819dfe0b367987a71f6d30908dd45e0d7a60","blockNumber":"0x159663e","gasUsed":"0x28ee7","effectiveGasPrice":"0x4bf02090","from":"0x234ce51365b9c417171b6dad280f49143e1b0547","to":"0x234ce51365b9c417171b6dad280f49143e1b0547","contractAddress":null}"#;
        let receipt: AnyTransactionReceipt = serde_json::from_str(s).unwrap();
        let _converted = convert_to_anvil_receipt(receipt).unwrap();
    }
}
