//! tests for custom anvil endpoints

use crate::next_port;
use anvil::{spawn, NodeConfig};
use ethers::{
    prelude::Middleware,
    types::{Address, TransactionRequest},
    utils::WEI_IN_ETHER,
};

#[tokio::test(flavor = "multi_thread")]
async fn can_set_gas_price() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    let gas_price = 1337u64.into();
    api.anvil_set_min_gas_price(gas_price).await.unwrap();
    assert_eq!(gas_price, provider.get_gas_price().await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    let impersonated = Address::random();
    let to = Address::random();

    let balance = WEI_IN_ETHER.saturating_mul(10u64.into());
    api.anvil_set_balance(impersonated, balance).await.unwrap();
    assert_eq!(balance, provider.get_balance(impersonated, None).await.unwrap());

    let tx = TransactionRequest::new().to(to).value(balance / 2);

    let res = provider.send_transaction(tx.clone().from(impersonated), None).await;
    assert!(res.is_err());

    api.anvil_impersonate_account(impersonated).await.unwrap();
    let tx = provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();

    assert_eq!(tx.from, impersonated);
}
