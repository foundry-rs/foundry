use crate::utils::http_provider;
use alloy_eips::{BlockNumberOrTag, eip2935::HISTORY_STORAGE_ADDRESS};
use alloy_network::TransactionBuilder;
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::EthereumHardfork;

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_contract_deployed_at_genesis() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    let code = provider.get_code_at(HISTORY_STORAGE_ADDRESS).await.unwrap();
    assert!(!code.is_empty(), "EIP-2935 history storage contract should be deployed at genesis");
}

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_stores_parent_block_hash() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    // Mine a few blocks so there are parent hashes to store
    api.mine_one().await;
    api.mine_one().await;
    api.mine_one().await;

    // Block 1's hash should be stored when block 2 was mined
    let block1 = provider
        .get_block_by_number(BlockNumberOrTag::from(1))
        .await
        .unwrap()
        .expect("block 1 should exist");
    let block1_hash = block1.header.hash;

    // Query the history storage contract for block 1's hash.
    // The EIP-2935 contract uses raw calldata (not ABI-encoded): pass the block number
    // as a 32-byte big-endian word directly.
    let call_data: [u8; 32] = U256::from(1).to_be_bytes();
    let tx = TransactionRequest::default().with_to(HISTORY_STORAGE_ADDRESS).with_input(call_data);
    let result = provider.call(tx.into()).await.unwrap();

    let stored_hash = alloy_primitives::B256::from_slice(&result);
    assert_eq!(stored_hash, block1_hash, "EIP-2935 contract should store parent block hash");
}

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_no_system_call_on_genesis() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    // At genesis (block 0), the contract should exist but no system call should have
    // written any parent hash into its storage. Check raw storage slot 0 directly.
    let slot = provider.get_storage_at(HISTORY_STORAGE_ADDRESS, U256::from(0)).await.unwrap();
    assert_eq!(slot, U256::ZERO, "No hash should be stored in the contract at genesis");
}

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_not_deployed_before_prague() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    let code = provider.get_code_at(HISTORY_STORAGE_ADDRESS).await.unwrap();
    assert!(code.is_empty(), "EIP-2935 contract should NOT be deployed before Prague");
}
