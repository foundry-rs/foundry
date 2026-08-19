//! Base-specific transaction conversions.

use alloy_consensus::Typed2718;
use alloy_evm::{FromRecoveredTx, FromTxWithEncoded};
use alloy_network::eip2718::Encodable2718;
use alloy_primitives::{Address, Bytes};
#[cfg(not(feature = "optimism"))]
use alloy_primitives::{B256, U256};
#[cfg(not(feature = "optimism"))]
use alloy_serde::OtherFields;
use base_common_consensus::BaseTxEnvelope;
use base_common_evm::{BaseTransaction, DepositTransactionParts};
#[cfg(not(feature = "optimism"))]
use op_revm::transaction::deposit::DepositTransactionParts as OpDepositTransactionParts;
use revm::context::TxEnv;

#[cfg(not(feature = "optimism"))]
use super::FoundryReceiptEnvelope;
use super::FoundryTxEnvelope;

/// Converts RPC extension fields into OP-compatible deposit parts.
#[cfg(not(feature = "optimism"))]
pub fn get_deposit_tx_parts(
    other: &OtherFields,
) -> Result<OpDepositTransactionParts, Vec<&'static str>> {
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
        Ok(OpDepositTransactionParts { source_hash, mint, is_system_transaction })
    } else {
        Err(missing)
    }
}

#[cfg(not(feature = "optimism"))]
impl<T> FoundryReceiptEnvelope<T> {
    /// Returns the deposit nonce when this is a deposit receipt.
    pub const fn deposit_nonce(&self) -> Option<u64> {
        match self {
            Self::Deposit(receipt) => receipt.receipt.deposit_nonce,
            _ => None,
        }
    }

    /// Returns the deposit receipt version when this is a deposit receipt.
    pub const fn deposit_receipt_version(&self) -> Option<u64> {
        match self {
            Self::Deposit(receipt) => receipt.receipt.deposit_receipt_version,
            _ => None,
        }
    }
}

impl FromRecoveredTx<FoundryTxEnvelope> for BaseTransaction<TxEnv> {
    fn from_recovered_tx(tx: &FoundryTxEnvelope, caller: Address) -> Self {
        let encoded = tx.encoded_2718().into();
        Self::from_encoded_tx(tx, caller, encoded)
    }
}

impl FromTxWithEncoded<FoundryTxEnvelope> for BaseTransaction<TxEnv> {
    fn from_encoded_tx(tx: &FoundryTxEnvelope, caller: Address, encoded: Bytes) -> Self {
        match tx {
            FoundryTxEnvelope::Legacy(signed) => Self {
                base: TxEnv::from_recovered_tx(signed, caller),
                enveloped_tx: Some(encoded),
                deposit: Default::default(),
                eip8130: None,
            },
            FoundryTxEnvelope::Eip2930(signed) => Self {
                base: TxEnv::from_recovered_tx(signed, caller),
                enveloped_tx: Some(encoded),
                deposit: Default::default(),
                eip8130: None,
            },
            FoundryTxEnvelope::Eip1559(signed) => Self {
                base: TxEnv::from_recovered_tx(signed, caller),
                enveloped_tx: Some(encoded),
                deposit: Default::default(),
                eip8130: None,
            },
            FoundryTxEnvelope::Eip4844(signed) => Self {
                base: TxEnv::from_recovered_tx(signed, caller),
                enveloped_tx: Some(encoded),
                deposit: Default::default(),
                eip8130: None,
            },
            FoundryTxEnvelope::Eip7702(signed) => Self {
                base: TxEnv::from_recovered_tx(signed, caller),
                enveloped_tx: Some(encoded),
                deposit: Default::default(),
                eip8130: None,
            },
            #[cfg(any(feature = "base", feature = "optimism"))]
            FoundryTxEnvelope::Deposit(sealed) => {
                let deposit = sealed.inner();
                let base = TxEnv {
                    tx_type: deposit.ty(),
                    caller,
                    gas_limit: deposit.gas_limit,
                    kind: deposit.to,
                    value: deposit.value,
                    data: deposit.input.clone(),
                    ..Default::default()
                };
                Self {
                    base,
                    enveloped_tx: None,
                    deposit: DepositTransactionParts {
                        source_hash: deposit.source_hash,
                        mint: Some(deposit.mint),
                        is_system_transaction: deposit.is_system_transaction,
                    },
                    eip8130: None,
                }
            }
            #[cfg(feature = "optimism")]
            FoundryTxEnvelope::PostExec(_) => {
                unreachable!("post-execution transaction in Base context")
            }
            FoundryTxEnvelope::Eip8130(signed) => {
                let envelope = BaseTxEnvelope::Eip8130(signed.clone());
                Self::from_recovered_tx(&envelope, caller)
            }
            FoundryTxEnvelope::Tempo(_) => unreachable!("Tempo transaction in Base context"),
        }
    }
}
