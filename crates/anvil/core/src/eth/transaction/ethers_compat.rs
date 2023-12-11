//! ethers compatibility, this is mainly necessary so we can use all of `ethers` signers

use super::EthTransactionRequest;
use crate::eth::{
    proof::AccountProof,
    state::{AccountOverride, StateOverride as EthStateOverride},
    transaction::{
        DepositTransactionRequest, EIP1559TransactionRequest, EIP2930TransactionRequest,
        LegacyTransactionRequest, MaybeImpersonatedTransaction, TypedTransaction,
        TypedTransactionRequest,
    },
};
use alloy_primitives::{U128 as rU128, U256 as rU256, U64 as rU64};
use alloy_rpc_types::{
    state::{AccountOverride as AlloyAccountOverride, StateOverride},
    AccessList as AlloyAccessList, CallRequest, Signature, Transaction as AlloyTransaction,
    TransactionRequest as AlloyTransactionRequest,
};
use ethers_core::types::{
    transaction::{
        eip2718::TypedTransaction as EthersTypedTransactionRequest,
        eip2930::{AccessList, AccessListItem},
        optimism::DepositTransaction,
    },
    Address, BigEndianHash, Eip1559TransactionRequest as EthersEip1559TransactionRequest,
    Eip2930TransactionRequest as EthersEip2930TransactionRequest, NameOrAddress, StorageProof,
    Transaction as EthersTransaction, TransactionRequest as EthersLegacyTransactionRequest,
    TransactionRequest, H256, U256, U64,
};
use foundry_common::types::{ToAlloy, ToEthers};

pub fn to_alloy_proof(proof: AccountProof) -> alloy_rpc_types::EIP1186AccountProofResponse {
    alloy_rpc_types::EIP1186AccountProofResponse {
        address: proof.address.to_alloy(),
        account_proof: proof.account_proof.into_iter().map(|b| b.0.into()).collect(),
        balance: proof.balance.to_alloy(),
        code_hash: proof.code_hash.to_alloy(),
        nonce: proof.nonce.to_alloy().to::<rU64>(),
        storage_hash: proof.storage_hash.to_alloy(),
        storage_proof: proof.storage_proof.iter().map(to_alloy_storage_proof).collect(),
    }
}

pub fn to_alloy_storage_proof(proof: &StorageProof) -> alloy_rpc_types::EIP1186StorageProof {
    alloy_rpc_types::EIP1186StorageProof {
        key: rU256::from_be_bytes(proof.key.to_alloy().0).into(),
        proof: proof.proof.iter().map(|b| b.clone().0.into()).collect(),
        value: proof.value.to_alloy(),
    }
}

pub fn to_internal_tx_request(request: &AlloyTransactionRequest) -> EthTransactionRequest {
    EthTransactionRequest {
        from: request.from.map(|a| a.to_ethers()),
        to: request.to.map(|a| a.to_ethers()),
        gas_price: request.gas_price.map(|g| alloy_primitives::U256::from(g).to_ethers()),
        max_fee_per_gas: request
            .max_fee_per_gas
            .map(|g| alloy_primitives::U256::from(g).to_ethers()),
        max_priority_fee_per_gas: request
            .max_priority_fee_per_gas
            .map(|g| alloy_primitives::U256::from(g).to_ethers()),
        gas: request.gas.map(|g| g.to_ethers()),
        value: request.value.map(|v| v.to_ethers()),
        data: request.data.clone().map(|b| b.clone().0.into()),
        nonce: request.nonce.map(|n| n.to::<u64>().into()),
        chain_id: None,
        access_list: request.access_list.clone().map(|a| to_ethers_access_list(a.clone()).0),
        transaction_type: request.transaction_type.map(|t| t.to::<u64>().into()),
        // TODO: Should this be none?
        optimism_fields: None,
    }
}

pub fn call_to_internal_tx_request(request: &CallRequest) -> EthTransactionRequest {
    EthTransactionRequest {
        from: request.from.map(|a| a.to_ethers()),
        to: request.to.map(|a| a.to_ethers()),
        gas_price: request.gas_price.map(|g| alloy_primitives::U256::from(g).to_ethers()),
        max_fee_per_gas: request
            .max_fee_per_gas
            .map(|g| alloy_primitives::U256::from(g).to_ethers()),
        max_priority_fee_per_gas: request
            .max_priority_fee_per_gas
            .map(|g| alloy_primitives::U256::from(g).to_ethers()),
        gas: request.gas.map(|g| g.to_ethers()),
        value: request.value.map(|v| v.to_ethers()),
        data: request.input.unique_input().unwrap().map(|b| b.clone().0.into()),
        nonce: request.nonce.map(|n| n.to::<u64>().into()),
        chain_id: request.chain_id.map(|c| c.to::<u64>().into()),
        access_list: request.access_list.clone().map(|a| to_ethers_access_list(a.clone()).0),
        transaction_type: request.transaction_type.map(|t| t.to::<u64>().into()),
        // TODO: Should this be none?
        optimism_fields: None,
    }
}

pub fn to_ethers_access_list(access_list: AlloyAccessList) -> AccessList {
    AccessList(
        access_list
            .0
            .into_iter()
            .map(|item| AccessListItem {
                address: item.address.to_ethers(),
                storage_keys: item
                    .storage_keys
                    .into_iter()
                    .map(|k| {
                        BigEndianHash::from_uint(&U256::from_big_endian(k.to_ethers().as_bytes()))
                    })
                    .collect(),
            })
            .collect(),
    )
}

pub fn from_ethers_access_list(access_list: AccessList) -> AlloyAccessList {
    AlloyAccessList(access_list.0.into_iter().map(ToAlloy::to_alloy).collect())
}

pub fn to_ethers_state_override(ov: StateOverride) -> EthStateOverride {
    ov.into_iter()
        .map(|(addr, o)| {
            (
                addr.to_ethers(),
                AccountOverride {
                    nonce: o.nonce.map(|n| n.to::<u64>()),
                    balance: o.balance.map(|b| b.to_ethers()),
                    code: o.code.map(|c| c.0.into()),
                    state_diff: o.state_diff.map(|s| {
                        s.into_iter()
                            .map(|(k, v)| (k.to_ethers(), H256::from_uint(&v.to_ethers())))
                            .collect()
                    }),
                    state: o.state.map(|s| {
                        s.into_iter()
                            .map(|(k, v)| (k.to_ethers(), H256::from_uint(&v.to_ethers())))
                            .collect()
                    }),
                },
            )
        })
        .collect()
}

pub fn to_alloy_state_override(ov: EthStateOverride) -> StateOverride {
    ov.into_iter()
        .map(|(addr, o)| {
            (
                addr.to_alloy(),
                AlloyAccountOverride {
                    nonce: o.nonce.map(rU64::from),
                    balance: o.balance.map(|b| b.to_alloy()),
                    code: o.code.map(|c| c.0.into()),
                    state_diff: o.state_diff.map(|s| {
                        s.into_iter()
                            .map(|(k, v)| (k.to_alloy(), rU256::from_be_bytes(v.to_alloy().0)))
                            .collect()
                    }),
                    state: o.state.map(|s| {
                        s.into_iter()
                            .map(|(k, v)| (k.to_alloy(), rU256::from_be_bytes(v.to_alloy().0)))
                            .collect()
                    }),
                },
            )
        })
        .collect()
}

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

fn to_alloy_transaction_with_hash_and_sender(
    transaction: TypedTransaction,
    hash: H256,
    from: Address,
) -> AlloyTransaction {
    match transaction {
        TypedTransaction::Legacy(t) => AlloyTransaction {
            hash: hash.to_alloy(),
            nonce: t.nonce.to_alloy().to::<rU64>(),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from: from.to_alloy(),
            to: None,
            value: t.value.to_alloy(),
            gas_price: Some(t.gas_price.to_alloy().to::<rU128>()),
            max_fee_per_gas: Some(t.gas_price.to_alloy().to::<rU128>()),
            max_priority_fee_per_gas: Some(t.gas_price.to_alloy().to::<rU128>()),
            gas: t.gas_limit.to_alloy(),
            input: t.input.clone().0.into(),
            chain_id: t.chain_id().map(rU64::from),
            signature: Some(Signature {
                r: t.signature.r.to_alloy(),
                s: t.signature.s.to_alloy(),
                v: rU256::from(t.signature.v),
                y_parity: None,
            }),
            access_list: None,
            transaction_type: None,
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
            other: Default::default(),
        },
        TypedTransaction::EIP2930(t) => AlloyTransaction {
            hash: hash.to_alloy(),
            nonce: t.nonce.to_alloy().to::<rU64>(),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from: from.to_alloy(),
            to: None,
            value: t.value.to_alloy(),
            gas_price: Some(t.gas_price.to_alloy().to::<rU128>()),
            max_fee_per_gas: Some(t.gas_price.to_alloy().to::<rU128>()),
            max_priority_fee_per_gas: Some(t.gas_price.to_alloy().to::<rU128>()),
            gas: t.gas_limit.to_alloy(),
            input: t.input.clone().0.into(),
            chain_id: Some(rU64::from(t.chain_id)),
            signature: Some(Signature {
                r: rU256::from_be_bytes(t.r.to_alloy().0),
                s: rU256::from_be_bytes(t.s.to_alloy().0),
                v: rU256::from(t.odd_y_parity as u8),
                y_parity: Some(t.odd_y_parity.into()),
            }),
            access_list: Some(from_ethers_access_list(t.access_list).0),
            transaction_type: Some(rU64::from(1)),
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
            other: Default::default(),
        },
        TypedTransaction::EIP1559(t) => AlloyTransaction {
            hash: hash.to_alloy(),
            nonce: t.nonce.to_alloy().to::<rU64>(),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from: from.to_alloy(),
            to: None,
            value: t.value.to_alloy(),
            gas_price: None,
            max_fee_per_gas: Some(t.max_fee_per_gas.to_alloy().to::<rU128>()),
            max_priority_fee_per_gas: Some(t.max_priority_fee_per_gas.to_alloy().to::<rU128>()),
            gas: t.gas_limit.to_alloy(),
            input: t.input.clone().0.into(),
            chain_id: Some(rU64::from(t.chain_id)),
            signature: Some(Signature {
                r: rU256::from_be_bytes(t.r.to_alloy().0),
                s: rU256::from_be_bytes(t.s.to_alloy().0),
                v: rU256::from(t.odd_y_parity as u8),
                y_parity: Some(t.odd_y_parity.into()),
            }),
            access_list: Some(from_ethers_access_list(t.access_list).0),
            transaction_type: Some(rU64::from(2)),
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
            other: Default::default(),
        },
        TypedTransaction::Deposit(t) => AlloyTransaction {
            hash: hash.to_alloy(),
            nonce: t.nonce.to_alloy().to::<rU64>(),
            block_hash: None,
            block_number: None,
            transaction_index: None,
            from: from.to_alloy(),
            to: None,
            value: t.value.to_alloy(),
            gas_price: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            gas: t.gas_limit.to_alloy(),
            input: t.input.clone().0.into(),
            chain_id: t.chain_id().map(rU64::from),
            signature: None,
            access_list: None,
            transaction_type: None,
            max_fee_per_blob_gas: None,
            blob_versioned_hashes: vec![],
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

impl From<MaybeImpersonatedTransaction> for AlloyTransaction {
    fn from(transaction: MaybeImpersonatedTransaction) -> Self {
        let hash = transaction.hash();
        let sender = transaction.recover().unwrap_or_default();

        to_alloy_transaction_with_hash_and_sender(transaction.into(), hash, sender)
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
            optimism_fields: None,
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
