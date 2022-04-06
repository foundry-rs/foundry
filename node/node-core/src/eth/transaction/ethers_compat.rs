//! ethers compatibility, this is mainly necessary so we can use all of `ethers` signers

use crate::eth::transaction::{
    EIP1559TransactionRequest, EIP2930TransactionRequest, LegacyTransactionRequest,
    TypedTransactionRequest,
};
use ethers_core::types::{
    transaction::eip2718::TypedTransaction as EthersTypedTransactionRequest,
    Eip1559TransactionRequest as EthersEip1559TransactionRequest,
    Eip2930TransactionRequest as EthersEip2930TransactionRequest,
    TransactionRequest as EthersLegacyTransactionRequest,
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
