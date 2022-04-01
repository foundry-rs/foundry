//! blockchain Backend

use forge_node_core::eth::transaction::{
    LegacyTransaction, PendingTransaction, TransactionKind, TypedTransaction,
};
use foundry_evm::revm::{CreateScheme, TransactTo, TxEnv};
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

pub fn to_tx_env(tx: &PendingTransaction, chain_id: Option<u64>) -> TxEnv {
    let caller = *tx.sender();

    match &tx.transaction {
        TypedTransaction::Legacy(tx) => {
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
        TypedTransaction::EIP2930(_tx) => {
            todo!()
        }
        TypedTransaction::EIP1559(_tx) => {
            todo!()
        }
    }
}

fn transact_to(kind: &TransactionKind) -> TransactTo {
    match kind {
        TransactionKind::Call(c) => TransactTo::Call(*c),
        TransactionKind::Create => TransactTo::Create(CreateScheme::Create),
    }
}
