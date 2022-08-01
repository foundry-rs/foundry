//! tests for anvil specific logic

use anvil::{spawn, NodeConfig};
use ethers::{prelude::Middleware, types::Address};

#[tokio::test(flavor = "multi_thread")]
async fn test_can_change_mining_mode() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    assert!(api.anvil_get_auto_mine().unwrap());

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 0);

    api.anvil_set_interval_mining(1).unwrap();
    assert!(!api.anvil_get_auto_mine().unwrap());
    // changing the mining mode will instantly mine a new block
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 0);

    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_default_dev_keys() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let dev_accounts = handle.dev_accounts().collect::<Vec<_>>();
    let accounts = provider.get_accounts().await.unwrap();
    assert_eq!(dev_accounts, accounts);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_set_empty_code() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    let addr = Address::random();
    api.anvil_set_code(addr, Vec::new().into()).await.unwrap();
    let code = api.get_code(addr, None).await.unwrap();
    assert!(code.as_ref().is_empty());
}
