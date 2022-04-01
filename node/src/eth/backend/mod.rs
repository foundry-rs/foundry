//! blockchain Backend

use ethers::types::{transaction::eip2930::AccessListItem, Address, U256};
use forge_node_core::eth::transaction::{
    EIP1559Transaction, EIP2930Transaction, LegacyTransaction, PendingTransaction, TransactionKind,
    TypedTransaction,
};
use foundry_evm::{
    revm::{CreateScheme, TransactTo, TxEnv},
    utils::h256_to_u256_be,
};
use std::time::Duration;

/// [revm](foundry_evm::revm) related types
pub mod db;
/// In-memory Backend
pub mod mem;

/// Returns the current duration since unix epoch.
pub fn duration_since_unix_epoch() -> Duration {
    use std::time::SystemTime;
    let now = SystemTime::now();
    now.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|err| panic!("Current time {:?} is invalid: {:?}", now, err))
}

pub fn to_tx_env(tx: &PendingTransaction) -> TxEnv {
    let caller = *tx.sender();

    match &tx.transaction {
        TypedTransaction::Legacy(tx) => {
            let chain_id = tx.chain_id();
            let LegacyTransaction { nonce, gas_price, gas_limit, value, kind, input, .. } = tx;
            TxEnv {
                caller,
                transact_to: transact_to(kind),
                data: input.0.clone(),
                chain_id,
                nonce: Some(nonce.as_u64()),
                value: *value,
                gas_price: *gas_price,
                gas_priority_fee: None,
                gas_limit: gas_limit.as_u64(),
                access_list: vec![],
            }
        }
        TypedTransaction::EIP2930(tx) => {
            let EIP2930Transaction {
                chain_id,
                nonce,
                gas_price,
                gas_limit,
                kind,
                value,
                input,
                access_list,
                ..
            } = tx;
            TxEnv {
                caller,
                transact_to: transact_to(kind),
                data: input.0.clone(),
                chain_id: Some(*chain_id),
                nonce: Some(nonce.as_u64()),
                value: *value,
                gas_price: *gas_price,
                gas_priority_fee: None,
                gas_limit: gas_limit.as_u64(),
                access_list: to_access_list(access_list.0.clone()),
            }
        }
        TypedTransaction::EIP1559(tx) => {
            let EIP1559Transaction {
                chain_id,
                nonce,
                max_priority_fee_per_gas,
                max_fee_per_gas,
                gas_limit,
                kind,
                value,
                input,
                access_list,
                ..
            } = tx;
            TxEnv {
                caller,
                transact_to: transact_to(kind),
                data: input.0.clone(),
                chain_id: Some(*chain_id),
                nonce: Some(nonce.as_u64()),
                value: *value,
                gas_price: *max_fee_per_gas,
                gas_priority_fee: Some(*max_priority_fee_per_gas),
                gas_limit: gas_limit.as_u64(),
                access_list: to_access_list(access_list.0.clone()),
            }
        }
    }
}

fn to_access_list(list: Vec<AccessListItem>) -> Vec<(Address, Vec<U256>)> {
    list.into_iter()
        .map(|item| (item.address, item.storage_keys.into_iter().map(h256_to_u256_be).collect()))
        .collect()
}

fn transact_to(kind: &TransactionKind) -> TransactTo {
    match kind {
        TransactionKind::Call(c) => TransactTo::Call(*c),
        TransactionKind::Create => TransactTo::Create(CreateScheme::Create),
    }
}
