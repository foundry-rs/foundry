use crate::helpers::*;
use alloy_primitives::{Address, U256};
use alloy_rpc_types::{
    state::{StateOverride, AccountOverride},
    TransactionRequest,
};
use alloy_serde::WithOtherFields;
use cast::Cast;
use std::collections::HashMap;

#[tokio::test]
async fn test_state_overrides() {
    let (api, handle) = spawn_http_server();
    let cast = Cast::new(api);

    let addr = Address::random();
    let mut state_overrides = HashMap::new();
    let mut account_override = AccountOverride::default();
    account_override.balance = Some(U256::from(1000));
    state_overrides.insert(addr, account_override);
    let state_override = Some(StateOverride(state_overrides));

    let tx = TransactionRequest::default().to(addr);
    let tx = WithOtherFields::new(tx);

    let result = cast.call(&tx, None, None, state_override).await;
    assert!(result.is_ok());

    handle.shutdown().await;
}

#[tokio::test]
async fn test_multiple_state_overrides() {
    let (api, handle) = spawn_http_server();
    let cast = Cast::new(api);

    let addr1 = Address::random();
    let addr2 = Address::random();
    let mut state_overrides = HashMap::new();

    let mut account_override1 = AccountOverride::default();
    account_override1.balance = Some(U256::from(1000));
    account_override1.nonce = Some(U256::from(1));
    state_overrides.insert(addr1, account_override1);

    let mut account_override2 = AccountOverride::default();
    account_override2.code = Some(vec![0x60, 0x00, 0x60, 0x00, 0x60, 0x00].into());
    account_override2.storage = Some(HashMap::from([(U256::from(1), U256::from(2))]));
    state_overrides.insert(addr2, account_override2);

    let state_override = Some(StateOverride(state_overrides));

    let tx = TransactionRequest::default().to(addr1);
    let tx = WithOtherFields::new(tx);

    let result = cast.call(&tx, None, None, state_override).await;
    assert!(result.is_ok());

    handle.shutdown().await;
} 