//! ethers compatibility, this is mainly necessary so we can use all of `ethers` signers

use super::EthTransactionRequest;
use crate::eth::transaction::{
    DepositTransactionRequest, EIP1559TransactionRequest, EIP2930TransactionRequest,
    LegacyTransactionRequest, MaybeImpersonatedTransaction, TypedTransaction,
    TypedTransactionRequest,
};
use ethers_core::types::{
    transaction::{
        eip2718::TypedTransaction as EthersTypedTransactionRequest, optimism::DepositTransaction,
    },
    Address, Eip1559TransactionRequest as EthersEip1559TransactionRequest,
    Eip2930TransactionRequest as EthersEip2930TransactionRequest, NameOrAddress,
    Transaction as EthersTransaction, TransactionRequest as EthersLegacyTransactionRequest,
    TransactionRequest, H256, U256, U64,
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
            TypedTransactionRequest::Deposit(tx) => {
                let DepositTransactionRequest {
                    source_hash,
                    from,
                    kind,
                    mint,
                    value,
                    gas_limit,
                    is_system_tx,
                    input,
                } = tx;
                EthersTypedTransactionRequest::DepositTransaction(DepositTransaction {
                    tx: TransactionRequest {
                        from: Some(from),
                        to: kind.as_call().cloned().map(Into::into),
                        gas: Some(gas_limit),
                        value: Some(value),
                        data: Some(input),
                        gas_price: Some(0.into()),
                        nonce: Some(0.into()),
                        chain_id: None,
                    },
                    source_hash,
                    mint: Some(mint),
                    is_system_tx,
                })
            }
        }
    }
}

fn to_ethers_transaction_with_hash_and_sender(
    transaction: TypedTransaction,
    hash: H256,
    from: Address,
) -> EthersTransaction {
    match transaction {
        TypedTransaction::Legacy(t) => EthersTransaction {
            hash,
            nonce: t.nonce,
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
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
            transaction_type: None,
            source_hash: H256::zero(),
            mint: None,
            is_system_tx: false,
            other: Default::default(),
        },
        TypedTransaction::EIP2930(t) => EthersTransaction {
            hash,
            nonce: t.nonce,
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
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
            source_hash: H256::zero(),
            mint: None,
            is_system_tx: false,
            other: Default::default(),
        },
        TypedTransaction::EIP1559(t) => EthersTransaction {
            hash,
            nonce: t.nonce,
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
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
            source_hash: H256::zero(),
            mint: None,
            is_system_tx: false,
            other: Default::default(),
        },
        TypedTransaction::Deposit(t) => EthersTransaction {
            hash,
            nonce: t.nonce,
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from,
            to: None,
            value: t.value,
            gas_price: Some(0.into()),
            max_fee_per_gas: Some(0.into()),
            max_priority_fee_per_gas: Some(0.into()),
            gas: t.gas_limit,
            input: t.input.clone(),
            chain_id: t.chain_id().map(Into::into),
            v: 0.into(),
            r: 0.into(),
            s: 0.into(),
            access_list: None,
            transaction_type: Some(126u64.into()),
            source_hash: t.source_hash,
            mint: Some(t.mint),
            is_system_tx: t.is_system_tx,
            other: Default::default(),
        },
    }
}

impl From<TypedTransaction> for EthersTransaction {
    fn from(transaction: TypedTransaction) -> Self {
        let hash = transaction.hash();
        let sender = transaction.recover().unwrap_or_default();
        to_ethers_transaction_with_hash_and_sender(transaction, hash, sender)
    }
}

impl From<MaybeImpersonatedTransaction> for EthersTransaction {
    fn from(transaction: MaybeImpersonatedTransaction) -> Self {
        let hash = transaction.hash();
        let sender = transaction.recover().unwrap_or_default();
        to_ethers_transaction_with_hash_and_sender(transaction.into(), hash, sender)
    }
}

impl From<TransactionRequest> for EthTransactionRequest {
    fn from(req: TransactionRequest) -> Self {
        let TransactionRequest { from, to, gas, gas_price, value, data, nonce, chain_id, .. } = req;
        EthTransactionRequest {
            from,
            to: to.and_then(|to| match to {
                NameOrAddress::Name(_) => None,
                NameOrAddress::Address(to) => Some(to),
            }),
            gas_price,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            gas,
            value,
            data,
            nonce,
            chain_id,
            access_list: None,
            transaction_type: None,
            source_hash: None,
            mint: None,
            is_system_tx: None,
        }
    }
}

impl From<EthTransactionRequest> for TransactionRequest {
    fn from(req: EthTransactionRequest) -> Self {
        let EthTransactionRequest { from, to, gas_price, gas, value, data, nonce, .. } = req;
        TransactionRequest {
            from,
            to: to.map(NameOrAddress::Address),
            gas,
            gas_price,
            value,
            data,
            nonce,
            chain_id: None,
        }
    }
}
