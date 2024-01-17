use crate::eth::utils::from_eip_to_alloy_access_list;
use alloy_primitives::{Address, B256, U128, U256, U64};
use alloy_rpc_types::{Signature, Transaction as AlloyTransaction};

use super::alloy::{MaybeImpersonatedTransaction, TypedTransaction};

impl From<MaybeImpersonatedTransaction> for AlloyTransaction {
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
) -> AlloyTransaction {
    match transaction {
        TypedTransaction::Enveloped(alloy_consensus::TxEnvelope::Legacy(t)) => AlloyTransaction {
            hash,
            nonce: U64::from(t.nonce),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: None,
            value: t.value,
            gas_price: Some(U128::from(t.gas_price)),
            max_fee_per_gas: Some(U128::from(t.gas_price)),
            max_priority_fee_per_gas: Some(U128::from(t.gas_price)),
            gas: U256::from(t.gas_limit),
            input: t.input.clone(),
            chain_id: t.chain_id.map(U64::from),
            signature: Some(Signature {
                r: t.signature().r(),
                s: t.signature().s(),
                v: U256::from(t.signature().v().y_parity_byte()),
                y_parity: None,
            }),
            access_list: None,
            transaction_type: None,
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
            other: Default::default(),
        },
        TypedTransaction::Enveloped(alloy_consensus::TxEnvelope::TaggedLegacy(t)) => AlloyTransaction {
            hash,
            nonce: U64::from(t.nonce),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: None,
            value: t.value,
            gas_price: Some(U128::from(t.gas_price)),
            max_fee_per_gas: Some(U128::from(t.gas_price)),
            max_priority_fee_per_gas: Some(U128::from(t.gas_price)),
            gas: U256::from(t.gas_limit),
            input: t.input.clone(),
            chain_id: t.chain_id.map(U64::from),
            signature: Some(Signature {
                r: t.signature().r(),
                s: t.signature().s(),
                v: U256::from(t.signature().v().y_parity_byte()),
                y_parity: None,
            }),
            access_list: None,
            transaction_type: None,
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
            other: Default::default(),
        },
        TypedTransaction::Enveloped(alloy_consensus::TxEnvelope::Eip2930(t)) => AlloyTransaction {
            hash,
            nonce: U64::from(t.nonce),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: None,
            value: t.value,
            gas_price: Some(U128::from(t.gas_price)),
            max_fee_per_gas: Some(U128::from(t.gas_price)),
            max_priority_fee_per_gas: Some(U128::from(t.gas_price)),
            gas: U256::from(t.gas_limit),
            input: t.input.clone(),
            chain_id: Some(U64::from(t.chain_id)),
            signature: Some(Signature {
                r: t.signature().r(),
                s: t.signature().s(),
                v: U256::from(t.signature().v().y_parity_byte()),
                y_parity: Some(alloy_rpc_types::Parity::from(t.signature().v().y_parity())),
            }),
            access_list: Some(from_eip_to_alloy_access_list(t.access_list.clone()).0),
            transaction_type: Some(U64::from(1)),
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
            other: Default::default(),
        },
        TypedTransaction::Enveloped(alloy_consensus::TxEnvelope::Eip1559(t)) => AlloyTransaction {
            hash,
            nonce: U64::from(t.nonce),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: None,
            value: t.value,
            gas_price: None,
            max_fee_per_gas: Some(U128::from(t.max_fee_per_gas)),
            max_priority_fee_per_gas: Some(U128::from(t.max_priority_fee_per_gas)),
            gas: U256::from(t.gas_limit),
            input: t.input.clone(),
            chain_id: Some(U64::from(t.chain_id)),
            signature: Some(Signature {
                r: t.signature().r(),
                s: t.signature().s(),
                v: U256::from(t.signature().v().y_parity_byte()),
                y_parity: Some(alloy_rpc_types::Parity::from(t.signature().v().y_parity())),
            }),
            access_list: Some(from_eip_to_alloy_access_list(t.access_list.clone()).0),
            transaction_type: Some(U64::from(2)),
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
            other: Default::default(),
        },
        TypedTransaction::Deposit(t) => AlloyTransaction {
            hash,
            nonce: U64::from(t.nonce),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: None,
            value: t.value,
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            gas: U256::from(t.gas_limit),
            input: t.input.clone().0.into(),
            chain_id: t.chain_id().map(U64::from),
            signature: None,
            access_list: None,
            transaction_type: None,
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
            other: Default::default(),
        },
    }
}

pub fn to_primitive_signature(
    signature: alloy_rpc_types::Signature,
) -> Result<alloy_primitives::Signature, alloy_primitives::SignatureError> {
    alloy_primitives::Signature::from_rs_and_parity(
        signature.r,
        signature.s,
        signature.v.to::<u64>(),
    )
}
