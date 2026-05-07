//! OP-stack-specific impls for [`FoundryTxEnvelope`] and [`FoundryTransactionRequest`].

use alloy_consensus::{Sealed, Transaction as _, Typed2718};
use alloy_evm::{FromRecoveredTx, FromTxWithEncoded};
use alloy_op_evm::OpTx;
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_serde::OtherFields;
use op_alloy_consensus::{
    OpDepositReceipt, OpDepositReceiptWithBloom, OpTransaction as OpTransactionTrait, OpTxEnvelope,
    TxDeposit, TxPostExec,
};
use op_revm::{OpTransaction, transaction::deposit::DepositTransactionParts};
use revm::context::TxEnv;

use super::{FoundryReceiptEnvelope, FoundryTransactionRequest, FoundryTxEnvelope};

impl OpTransactionTrait for FoundryTxEnvelope {
    fn is_deposit(&self) -> bool {
        matches!(self, Self::Deposit(_))
    }

    fn as_deposit(&self) -> Option<&Sealed<TxDeposit>> {
        match self {
            Self::Deposit(tx) => Some(tx),
            _ => None,
        }
    }

    fn as_post_exec(&self) -> Option<&Sealed<TxPostExec>> {
        if let Self::PostExec(tx) = self { Some(tx) } else { None }
    }
}

impl From<OpTxEnvelope> for FoundryTxEnvelope {
    fn from(tx: OpTxEnvelope) -> Self {
        match tx {
            OpTxEnvelope::Legacy(tx) => Self::Legacy(tx),
            OpTxEnvelope::Eip2930(tx) => Self::Eip2930(tx),
            OpTxEnvelope::Eip1559(tx) => Self::Eip1559(tx),
            OpTxEnvelope::Eip7702(tx) => Self::Eip7702(tx),
            OpTxEnvelope::Deposit(tx) => Self::Deposit(tx),
            OpTxEnvelope::PostExec(tx) => Self::PostExec(tx),
        }
    }
}

impl FromRecoveredTx<FoundryTxEnvelope> for OpTransaction<TxEnv> {
    fn from_recovered_tx(tx: &FoundryTxEnvelope, caller: Address) -> Self {
        match tx {
            FoundryTxEnvelope::Legacy(signed_tx) => {
                let base = TxEnv::from_recovered_tx(signed_tx, caller);
                Self { base, enveloped_tx: None, deposit: Default::default() }
            }
            FoundryTxEnvelope::Eip2930(signed_tx) => {
                let base = TxEnv::from_recovered_tx(signed_tx, caller);
                Self { base, enveloped_tx: None, deposit: Default::default() }
            }
            FoundryTxEnvelope::Eip1559(signed_tx) => {
                let base = TxEnv::from_recovered_tx(signed_tx, caller);
                Self { base, enveloped_tx: None, deposit: Default::default() }
            }
            FoundryTxEnvelope::Eip4844(signed_tx) => {
                let base = TxEnv::from_recovered_tx(signed_tx, caller);
                Self { base, enveloped_tx: None, deposit: Default::default() }
            }
            FoundryTxEnvelope::Eip7702(signed_tx) => {
                let base = TxEnv::from_recovered_tx(signed_tx, caller);
                Self { base, enveloped_tx: None, deposit: Default::default() }
            }
            FoundryTxEnvelope::Deposit(sealed_tx) => {
                let deposit_tx = sealed_tx.inner();
                let base = TxEnv {
                    tx_type: deposit_tx.ty(),
                    caller,
                    gas_limit: deposit_tx.gas_limit,
                    kind: deposit_tx.to,
                    value: deposit_tx.value,
                    data: deposit_tx.input.clone(),
                    ..Default::default()
                };
                let deposit = DepositTransactionParts {
                    source_hash: deposit_tx.source_hash,
                    mint: Some(deposit_tx.mint),
                    is_system_transaction: deposit_tx.is_system_transaction,
                };
                Self { base, enveloped_tx: None, deposit }
            }
            FoundryTxEnvelope::PostExec(sealed_tx) => {
                let tx = sealed_tx.inner();
                let base = TxEnv {
                    tx_type: tx.ty(),
                    caller,
                    kind: tx.kind(),
                    data: tx.input.clone(),
                    ..Default::default()
                };
                Self { base, enveloped_tx: None, deposit: Default::default() }
            }
            FoundryTxEnvelope::Tempo(_) => unreachable!("Tempo tx in Optimism context"),
        }
    }
}

impl FromRecoveredTx<FoundryTxEnvelope> for OpTx {
    fn from_recovered_tx(tx: &FoundryTxEnvelope, caller: Address) -> Self {
        Self(OpTransaction::<TxEnv>::from_recovered_tx(tx, caller))
    }
}

impl FromTxWithEncoded<FoundryTxEnvelope> for OpTx {
    fn from_encoded_tx(tx: &FoundryTxEnvelope, caller: Address, encoded: Bytes) -> Self {
        Self(OpTransaction::<TxEnv>::from_encoded_tx(tx, caller, encoded))
    }
}

impl FromTxWithEncoded<FoundryTxEnvelope> for OpTransaction<TxEnv> {
    fn from_encoded_tx(tx: &FoundryTxEnvelope, caller: Address, encoded: Bytes) -> Self {
        match tx {
            FoundryTxEnvelope::Legacy(signed_tx) => {
                let base = TxEnv::from_recovered_tx(signed_tx, caller);
                Self { base, enveloped_tx: Some(encoded), deposit: Default::default() }
            }
            FoundryTxEnvelope::Eip2930(signed_tx) => {
                let base = TxEnv::from_recovered_tx(signed_tx, caller);
                Self { base, enveloped_tx: Some(encoded), deposit: Default::default() }
            }
            FoundryTxEnvelope::Eip1559(signed_tx) => {
                let base = TxEnv::from_recovered_tx(signed_tx, caller);
                Self { base, enveloped_tx: Some(encoded), deposit: Default::default() }
            }
            FoundryTxEnvelope::Eip4844(signed_tx) => {
                let base = TxEnv::from_recovered_tx(signed_tx, caller);
                Self { base, enveloped_tx: Some(encoded), deposit: Default::default() }
            }
            FoundryTxEnvelope::Eip7702(signed_tx) => {
                let base = TxEnv::from_recovered_tx(signed_tx, caller);
                Self { base, enveloped_tx: Some(encoded), deposit: Default::default() }
            }
            FoundryTxEnvelope::Deposit(sealed_tx) => {
                let deposit_tx = sealed_tx.inner();
                let base = TxEnv {
                    tx_type: deposit_tx.ty(),
                    caller,
                    gas_limit: deposit_tx.gas_limit,
                    kind: deposit_tx.to,
                    value: deposit_tx.value,
                    data: deposit_tx.input.clone(),
                    ..Default::default()
                };
                let deposit = DepositTransactionParts {
                    source_hash: deposit_tx.source_hash,
                    mint: Some(deposit_tx.mint),
                    is_system_transaction: deposit_tx.is_system_transaction,
                };
                Self { base, enveloped_tx: Some(encoded), deposit }
            }
            FoundryTxEnvelope::PostExec(sealed_tx) => {
                let tx = sealed_tx.inner();
                let base = TxEnv {
                    tx_type: tx.ty(),
                    caller,
                    kind: tx.kind(),
                    data: tx.input.clone(),
                    ..Default::default()
                };
                Self { base, enveloped_tx: Some(encoded), deposit: Default::default() }
            }
            FoundryTxEnvelope::Tempo(_) => unreachable!("Tempo tx in Optimism context"),
        }
    }
}

impl From<op_alloy_rpc_types::Transaction<FoundryTxEnvelope>> for FoundryTransactionRequest {
    fn from(tx: op_alloy_rpc_types::Transaction<FoundryTxEnvelope>) -> Self {
        tx.inner.into_inner().into()
    }
}

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

/// OP-stack-specific accessors on [`FoundryReceiptEnvelope`].
impl<T> FoundryReceiptEnvelope<T> {
    /// Return the receipt's deposit_nonce if it is a deposit receipt.
    pub fn deposit_nonce(&self) -> Option<u64> {
        self.as_deposit_receipt().and_then(|r| r.deposit_nonce)
    }

    /// Return the receipt's deposit version if it is a deposit receipt.
    pub fn deposit_receipt_version(&self) -> Option<u64> {
        self.as_deposit_receipt().and_then(|r| r.deposit_receipt_version)
    }

    /// Returns the deposit receipt if it is a deposit receipt.
    pub const fn as_deposit_receipt_with_bloom(&self) -> Option<&OpDepositReceiptWithBloom<T>> {
        match self {
            Self::Deposit(t) => Some(t),
            _ => None,
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

#[cfg(test)]
mod tests {
    use alloy_network::eip2718::Encodable2718;
    use alloy_primitives::TxHash;
    use alloy_rlp::Decodable;

    use super::*;

    #[test]
    fn test_from_recovered_tx_legacy_op() {
        use alloy_consensus::transaction::SignerRecoverable;

        let tx = r#"
        {
            "type": "0x0",
            "chainId": "0x1",
            "nonce": "0x0",
            "gas": "0x5208",
            "gasPrice": "0x1",
            "to": "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
            "value": "0x1",
            "input": "0x",
            "r": "0x85c2794a580da137e24ccc823b45ae5cea99371ae23ee13860fcc6935f8305b0",
            "s": "0x41de7fa4121dab284af4453d30928241208bafa90cdb701fe9bc7054759fe3cd",
            "v": "0x1b",
            "hash": "0x8c9b68e8947ace33028dba167354fde369ed7bbe34911b772d09b3c64b861515"
        }"#;

        let typed_tx: FoundryTxEnvelope = serde_json::from_str(tx).unwrap();
        let sender = typed_tx.recover_signer().unwrap();

        // Test OpTransaction<TxEnv> conversion via FromRecoveredTx trait
        let op_tx = OpTransaction::<TxEnv>::from_recovered_tx(&typed_tx, sender);
        assert_eq!(op_tx.base.caller, sender);
        assert_eq!(op_tx.base.gas_limit, 0x5208);
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
        let dep_tx = FoundryTxEnvelope::decode(&mut raw_tx.as_slice()).unwrap();

        let mut encoded = Vec::new();
        dep_tx.encode_2718(&mut encoded);

        assert_eq!(raw_tx, encoded);

        assert_eq!(tx_hash, dep_tx.hash());
    }
}
