//! ethers compatibility, this is mainly necessary so we can use all of `ethers` signers

use crate::eth::transaction::{
    EIP1559TransactionRequest, EIP2930TransactionRequest, LegacyTransactionRequest,
    TypedTransaction, TypedTransactionRequest,
};
use ethers_core::types::{
    transaction::eip2718::TypedTransaction as EthersTypedTransactionRequest, Address,
    Eip1559TransactionRequest as EthersEip1559TransactionRequest,
    Eip2930TransactionRequest as EthersEip2930TransactionRequest, Transaction as EthersTransaction,
    TransactionRequest as EthersLegacyTransactionRequest, U256, U64,
};

impl From<TypedTransactionRequest> for EthersTypedTransactionRequest {
    fn from(tx: TypedTransactionRequest) -> Self {
        match tx {
            TypedTransactionRequest::Legacy(tx) => {
                let LegacyTransactionRequest {
                    nonce,
                    gas_price,
                    gas_limit,
                    kind,
                    value,
                    input,
                    chain_id,
                } = tx;
                EthersTypedTransactionRequest::Legacy(EthersLegacyTransactionRequest {
                    from: None,
                    to: kind.as_call().cloned().map(Into::into),
                    gas: Some(gas_limit),
                    gas_price: Some(gas_price),
                    value: Some(value),
                    data: Some(input),
                    nonce: Some(nonce),
                    chain_id: chain_id.map(Into::into),
                })
            }
            TypedTransactionRequest::EIP2930(tx) => {
                let EIP2930TransactionRequest {
                    chain_id,
                    nonce,
                    gas_price,
                    gas_limit,
                    kind,
                    value,
                    input,
                    access_list,
                } = tx;
                EthersTypedTransactionRequest::Eip2930(EthersEip2930TransactionRequest {
                    tx: EthersLegacyTransactionRequest {
                        from: None,
                        to: kind.as_call().cloned().map(Into::into),
                        gas: Some(gas_limit),
                        gas_price: Some(gas_price),
                        value: Some(value),
                        data: Some(input),
                        nonce: Some(nonce),
                        chain_id: Some(chain_id.into()),
                    },
                    access_list: access_list.into(),
                })
            }
            TypedTransactionRequest::EIP1559(tx) => {
                let EIP1559TransactionRequest {
                    chain_id,
                    nonce,
                    max_priority_fee_per_gas,
                    max_fee_per_gas,
                    gas_limit,
                    kind,
                    value,
                    input,
                    access_list,
                } = tx;
                EthersTypedTransactionRequest::Eip1559(EthersEip1559TransactionRequest {
                    from: None,
                    to: kind.as_call().cloned().map(Into::into),
                    gas: Some(gas_limit),
                    value: Some(value),
                    data: Some(input),
                    nonce: Some(nonce),
                    access_list: access_list.into(),
                    max_priority_fee_per_gas: Some(max_priority_fee_per_gas),
                    max_fee_per_gas: Some(max_fee_per_gas),
                    chain_id: Some(chain_id.into()),
                })
            }
        }
    }
}

impl From<TypedTransaction> for EthersTransaction {
    fn from(transaction: TypedTransaction) -> Self {
        let hash = transaction.hash();
        match transaction {
            TypedTransaction::Legacy(t) => EthersTransaction {
                hash,
                nonce: t.nonce,
                block_hash: None,
                block_number: None,
                transaction_index: None,
                from: Address::default(),
                to: None,
                value: t.value,
                gas_price: Some(t.gas_price),
                max_fee_per_gas: Some(t.gas_price),
                max_priority_fee_per_gas: Some(t.gas_price),
                gas: t.gas_limit,
                input: t.input.clone(),
                chain_id: t.chain_id().map(Into::into),
                v: t.signature.v.into(),
                r: t.signature.r,
                s: t.signature.s,
                access_list: None,
                transaction_type: Some(0u64.into()),
                other: Default::default(),
            },
            TypedTransaction::EIP2930(t) => EthersTransaction {
                hash,
                nonce: t.nonce,
                block_hash: None,
                block_number: None,
                transaction_index: None,
                from: Address::default(),
                to: None,
                value: t.value,
                gas_price: Some(t.gas_price),
                max_fee_per_gas: Some(t.gas_price),
                max_priority_fee_per_gas: Some(t.gas_price),
                gas: t.gas_limit,
                input: t.input.clone(),
                chain_id: Some(t.chain_id.into()),
                v: U64::from(t.odd_y_parity as u8),
                r: U256::from(t.r.as_bytes()),
                s: U256::from(t.s.as_bytes()),
                access_list: Some(t.access_list),
                transaction_type: Some(1u64.into()),
                other: Default::default(),
            },
            TypedTransaction::EIP1559(t) => EthersTransaction {
                hash,
                nonce: t.nonce,
                block_hash: None,
                block_number: None,
                transaction_index: None,
                from: Address::default(),
                to: None,
                value: t.value,
                gas_price: None,
                max_fee_per_gas: Some(t.max_fee_per_gas),
                max_priority_fee_per_gas: Some(t.max_priority_fee_per_gas),
                gas: t.gas_limit,
                input: t.input.clone(),
                chain_id: Some(t.chain_id.into()),
                v: U64::from(t.odd_y_parity as u8),
                r: U256::from(t.r.as_bytes()),
                s: U256::from(t.s.as_bytes()),
                access_list: Some(t.access_list),
                transaction_type: Some(2u64.into()),
                other: Default::default(),
            },
        }
    }
}
