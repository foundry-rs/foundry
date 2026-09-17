//! Deposit helpers shared by Base and Optimism.

use super::FoundryReceiptEnvelope;
use alloy_primitives::{B256, U256};
use alloy_serde::OtherFields;
use op_alloy_consensus::OpDepositReceipt;
use op_revm::transaction::deposit::DepositTransactionParts;

/// Converts `OtherFields` to `DepositTransactionParts`, produces error with missing fields.
pub fn get_deposit_tx_parts(
    other: &OtherFields,
) -> Result<DepositTransactionParts, Vec<&'static str>> {
    let mut missing = Vec::new();
    let source_hash =
        other.get_deserialized::<B256>("sourceHash").transpose().ok().flatten().unwrap_or_else(
            || {
                missing.push("sourceHash");
                Default::default()
            },
        );
    let mint = other
        .get_deserialized::<U256>("mint")
        .transpose()
        .unwrap_or_else(|_| {
            missing.push("mint");
            Default::default()
        })
        .map(|value| value.saturating_to::<u128>());
    let is_system_transaction =
        other.get_deserialized::<bool>("isSystemTx").transpose().ok().flatten().unwrap_or_else(
            || {
                missing.push("isSystemTx");
                Default::default()
            },
        );
    if missing.is_empty() {
        Ok(DepositTransactionParts { source_hash, mint, is_system_transaction })
    } else {
        Err(missing)
    }
}

/// Deposit accessors shared by Base and Optimism.
impl<T> FoundryReceiptEnvelope<T> {
    /// Return the receipt's deposit_nonce if it is a deposit receipt.
    pub const fn deposit_nonce(&self) -> Option<u64> {
        match self.as_deposit_receipt() {
            Some(receipt) => receipt.deposit_nonce,
            None => None,
        }
    }

    /// Return the receipt's deposit version if it is a deposit receipt.
    pub const fn deposit_receipt_version(&self) -> Option<u64> {
        match self.as_deposit_receipt() {
            Some(receipt) => receipt.deposit_receipt_version,
            None => None,
        }
    }

    /// Returns the deposit receipt if it is a deposit receipt.
    pub const fn as_deposit_receipt(&self) -> Option<&OpDepositReceipt<T>> {
        match self {
            Self::Deposit(t) => Some(&t.receipt),
            _ => None,
        }
    }
}
