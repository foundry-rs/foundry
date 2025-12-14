//! Transaction related types
use alloy_consensus::{
    Receipt, Signed, Transaction, TxEnvelope, TxReceipt, Typed2718, transaction::Recovered,
};

use alloy_eips::eip2718::Encodable2718;
use alloy_network::{AnyReceiptEnvelope, AnyTransactionReceipt};
use alloy_primitives::{Address, B256, Bytes, TxHash, U64};
use alloy_rlp::{Decodable, Encodable};
use alloy_rpc_types::{Log, Transaction as RpcTransaction, TransactionReceipt};
use alloy_serde::WithOtherFields;
use bytes::BufMut;
use foundry_evm::traces::CallTraceNode;
use foundry_primitives::{FoundryReceiptEnvelope, FoundryTxEnvelope};
use op_alloy_consensus::{OpDepositReceipt, OpDepositReceiptWithBloom};
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

pub type ReceiptResponse = WithOtherFields<TransactionReceipt<FoundryReceiptEnvelope<Log>>>;

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
                0x00 => FoundryReceiptEnvelope::Legacy(receipt_with_bloom),
                0x01 => FoundryReceiptEnvelope::Eip2930(receipt_with_bloom),
                0x02 => FoundryReceiptEnvelope::Eip1559(receipt_with_bloom),
                0x03 => FoundryReceiptEnvelope::Eip4844(receipt_with_bloom),
                0x04 => FoundryReceiptEnvelope::Eip7702(receipt_with_bloom),
                0x7E => FoundryReceiptEnvelope::Deposit(OpDepositReceiptWithBloom {
                    receipt: OpDepositReceipt {
                        inner: Receipt {
                            status: alloy_consensus::Eip658Value::Eip658(
                                receipt_with_bloom.status(),
                            ),
                            cumulative_gas_used: receipt_with_bloom.cumulative_gas_used(),
                            logs: receipt_with_bloom.receipt.logs,
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
