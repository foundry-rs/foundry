//! OP-stack-specific helpers and type aliases used by [`super::FoundryNetwork`] and
//! [`super::FoundryTxReceipt`].

use alloy_consensus::{Receipt, ReceiptWithBloom, TxReceipt};
use alloy_primitives::U64;
use alloy_rpc_types::Log;
use alloy_serde::OtherFields;
use op_alloy_consensus::{OpDepositReceipt, OpDepositReceiptWithBloom};

use crate::FoundryReceiptEnvelope;

/// JSON-RPC transaction response type used by [`super::FoundryNetwork`].
pub type FoundryTransactionResponse = op_alloy_rpc_types::Transaction<crate::FoundryTxEnvelope>;

/// Build a [`FoundryReceiptEnvelope::Deposit`] from a `ReceiptWithBloom<Log>` plus the OP
/// deposit-specific fields decoded from the [`OtherFields`] of an `AnyTransactionReceipt`.
pub(super) fn build_deposit_receipt_envelope(
    receipt_with_bloom: ReceiptWithBloom<Receipt<Log>>,
    other: &OtherFields,
) -> FoundryReceiptEnvelope<Log> {
    // These fields may not be present in all receipts, so missing/invalid values are None.
    let deposit_nonce = other
        .get_deserialized::<U64>("depositNonce")
        .transpose()
        .ok()
        .flatten()
        .map(|v| v.to::<u64>());
    let deposit_receipt_version = other
        .get_deserialized::<U64>("depositReceiptVersion")
        .transpose()
        .ok()
        .flatten()
        .map(|v| v.to::<u64>());

    FoundryReceiptEnvelope::Deposit(OpDepositReceiptWithBloom {
        receipt: OpDepositReceipt {
            inner: Receipt {
                status: alloy_consensus::Eip658Value::Eip658(receipt_with_bloom.status()),
                cumulative_gas_used: receipt_with_bloom.cumulative_gas_used(),
                logs: receipt_with_bloom.receipt.logs,
            },
            deposit_nonce,
            deposit_receipt_version,
        },
        logs_bloom: receipt_with_bloom.logs_bloom,
    })
}
