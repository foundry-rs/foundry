use crate::utils::http_provider;
use alloy_eips::{BlockId, BlockNumberOrTag, eip2935::HISTORY_STORAGE_ADDRESS};
use alloy_network::TransactionBuilder;
use alloy_primitives::{B256, U256, address, bytes};
use alloy_provider::Provider;
use alloy_rpc_types::{
    TransactionRequest,
    trace::{opcode::BlockOpcodeGas, parity::TraceType},
};
use alloy_serde::WithOtherFields;
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

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_local_block_replay_applies_pre_execution_changes() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = handle.http_provider();
    let probe = address!("0000000000000000000000000000000000001000");

    // Forward the calldata to the history contract and store one at slot zero only when the
    // returned parent hash is non-zero.
    api.anvil_set_code(
        probe,
        bytes!(
            "60205f5f3760205f60205f730000f90827f1c53a10cb7a02335b1753200029355afa505f5115602e5760015f55005b00"
        ),
    )
    .await
    .unwrap();
    api.mine_one().await;

    let parent = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .expect("parent block should exist");
    api.anvil_set_auto_mine(false).await.unwrap();

    let from = handle.dev_wallets().next().unwrap().address();
    let call_data: [u8; 32] = U256::from(parent.header.number).to_be_bytes();
    let pending = provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(from)
                .to(probe)
                .with_input(call_data)
                .with_gas_limit(100_000),
        ))
        .await
        .unwrap();
    api.mine_one().await;
    let receipt = pending.get_receipt().await.unwrap();
    let block_number = receipt.block_number.unwrap();

    let replay = api
        .trace_replay_block_transactions(
            block_number.into(),
            [TraceType::StateDiff].into_iter().collect(),
        )
        .await
        .unwrap();
    let storage = &replay[0]
        .full_trace
        .state_diff
        .as_ref()
        .expect("state diff should be present")
        .get(&probe)
        .expect("probe should change")
        .storage;
    assert!(storage.contains_key(&B256::ZERO));

    let opcode_gas: Option<BlockOpcodeGas> = provider
        .raw_request("trace_blockOpcodeGas".into(), (BlockId::number(block_number),))
        .await
        .unwrap();
    assert!(opcode_gas.expect("block should exist").contains("SSTORE"));
}
