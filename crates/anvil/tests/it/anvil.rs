//! tests for anvil specific logic

use alloy_consensus::EMPTY_ROOT_HASH;
use alloy_eips::BlockNumberOrTag;
use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, U256, bytes, hex};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_sol_types::SolCall;
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::EthereumHardfork;

#[tokio::test(flavor = "multi_thread")]
async fn test_can_change_mining_mode() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    assert!(api.anvil_get_auto_mine().unwrap());
    assert!(api.anvil_get_interval_mining().unwrap().is_none());

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 0);

    api.anvil_set_interval_mining(1).unwrap();
    assert!(!api.anvil_get_auto_mine().unwrap());
    assert!(matches!(api.anvil_get_interval_mining().unwrap(), Some(1)));
    // changing the mining mode will instantly mine a new block
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 0);

    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 1);

    // assert that no block is mined when the interval is set to 0
    api.anvil_set_interval_mining(0).unwrap();
    assert!(!api.anvil_get_auto_mine().unwrap());
    assert!(api.anvil_get_interval_mining().unwrap().is_none());
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 1);
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

#[tokio::test(flavor = "multi_thread")]
async fn test_can_set_genesis_timestamp() {
    let genesis_timestamp = 1000u64;
    let (_api, handle) =
        spawn(NodeConfig::test().with_genesis_timestamp(genesis_timestamp.into())).await;
    let provider = handle.http_provider();

    assert_eq!(
        genesis_timestamp,
        provider.get_block(0.into()).await.unwrap().unwrap().header.timestamp
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_can_use_default_genesis_timestamp() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    assert_ne!(0u64, provider.get_block(0.into()).await.unwrap().unwrap().header.timestamp);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_can_handle_large_timestamp() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    let num = 317071597274;
    api.evm_set_next_block_timestamp(num).unwrap();
    api.mine_one().await;

    let block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(block.header.timestamp, num);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_shanghai_fields() {
    let (api, _handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Shanghai.into()))).await;
    api.mine_one().await;

    let block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(block.header.withdrawals_root, Some(EMPTY_ROOT_HASH));
    assert_eq!(block.withdrawals, Some(Default::default()));
    assert!(block.header.blob_gas_used.is_none());
    assert!(block.header.excess_blob_gas.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cancun_fields() {
    let (api, _handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()))).await;
    api.mine_one().await;

    let block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(block.header.withdrawals_root, Some(EMPTY_ROOT_HASH));
    assert_eq!(block.withdrawals, Some(Default::default()));
    assert!(block.header.blob_gas_used.is_some());
    assert!(block.header.excess_blob_gas.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_can_set_genesis_block_number() {
    let (_api, handle) = spawn(NodeConfig::test().with_genesis_block_number(Some(1337u64))).await;
    let provider = handle.http_provider();

    let block_number = provider.get_block_number().await.unwrap();
    assert_eq!(block_number, 1337u64);

    assert_eq!(1337, provider.get_block(1337.into()).await.unwrap().unwrap().header.number);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_can_use_default_genesis_block_number() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    assert_eq!(0, provider.get_block(0.into()).await.unwrap().unwrap().header.number);
}

/// Verify that genesis block number affects both RPC and EVM execution layer.
#[tokio::test(flavor = "multi_thread")]
async fn test_number_opcode_reflects_genesis_block_number() {
    let genesis_number: u64 = 4242;
    let (api, handle) =
        spawn(NodeConfig::test().with_genesis_block_number(Some(genesis_number))).await;
    let provider = handle.http_provider();

    // RPC layer should return configured genesis number
    let bn = provider.get_block_number().await.unwrap();
    assert_eq!(bn, genesis_number);

    // Deploy bytecode that returns block.number
    // 0x43 (NUMBER) 0x5f (PUSH0) 0x52 (MSTORE) 0x60 0x20 (PUSH1 0x20) 0x5f (PUSH0) 0xf3 (RETURN)
    let target = Address::random();
    api.anvil_set_code(target, bytes!("435f5260205ff3")).await.unwrap();

    // EVM execution should reflect genesis number (+ 1 for pending block)
    let tx = alloy_rpc_types::TransactionRequest::default().with_to(target);
    let out = provider.call(tx.into()).await.unwrap();
    let returned = U256::from_be_slice(out.as_ref());
    assert_eq!(returned, U256::from(genesis_number + 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_anvil_recover_signature() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    alloy_sol_types::sol! {
    #[sol(rpc)]
        contract TestRecover {
            function testRecover(bytes32 hash, uint8 v, bytes32 r, bytes32 s, address expected) external pure {
                address recovered = ecrecover(hash, v, r, s);
                require(recovered == expected, "ecrecover failed: address mismatch");
            }
        }
    }
    let bytecode = hex::decode(
        "0x60808060405234601557610125908161001a8239f35b5f80fdfe60808060405260043610156011575f80fd5b5f3560e01c63bff0b743146023575f80fd5b3460eb5760a036600319011260eb5760243560ff811680910360eb576084356001600160a01b038116929083900360eb5760805f916020936004358252848201526044356040820152606435606082015282805260015afa1560e0575f516001600160a01b031603609057005b60405162461bcd60e51b815260206004820152602260248201527f65637265636f766572206661696c65643a2061646472657373206d69736d61746044820152610c6d60f31b6064820152608490fd5b6040513d5f823e3d90fd5b5f80fdfea264697066735822122006368b42bca31c97f2c409a1cc5186dc899d4255ecc28db7bbb0ad285dc82ae464736f6c634300081c0033",
    ).unwrap();

    let tx = TransactionRequest::default().with_deploy_code(bytecode);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();
    let contract_address = receipt.contract_address().unwrap();
    let contract = TestRecover::new(contract_address, &provider);

    let sig = alloy_primitives::hex::decode("11".repeat(65)).unwrap();
    let r = B256::from_slice(&sig[0..32]);
    let s = B256::from_slice(&sig[32..64]);
    let v = sig[64];
    let fake_hash = B256::random();
    let expected = alloy_primitives::address!("0x1234567890123456789012345678901234567890");
    api.anvil_impersonate_signature(sig.clone().into(), expected).await.unwrap();
    let result = contract.testRecover(fake_hash, v, r, s, expected).call().await;
    assert!(result.is_ok(), "ecrecover failed: {:?}", result.err());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fake_signature_transaction() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    alloy_sol_types::sol! {
    #[sol(rpc)]
        contract TestRecover {
            function testRecover(bytes32 hash, uint8 v, bytes32 r, bytes32 s, address expected) external pure {
                address recovered = ecrecover(hash, v, r, s);
                require(recovered == expected, "ecrecover failed: address mismatch");
            }
        }
    }
    let bytecode = hex::decode(
        "0x60808060405234601557610125908161001a8239f35b5f80fdfe60808060405260043610156011575f80fd5b5f3560e01c63bff0b743146023575f80fd5b3460eb5760a036600319011260eb5760243560ff811680910360eb576084356001600160a01b038116929083900360eb5760805f916020936004358252848201526044356040820152606435606082015282805260015afa1560e0575f516001600160a01b031603609057005b60405162461bcd60e51b815260206004820152602260248201527f65637265636f766572206661696c65643a2061646472657373206d69736d61746044820152610c6d60f31b6064820152608490fd5b6040513d5f823e3d90fd5b5f80fdfea264697066735822122006368b42bca31c97f2c409a1cc5186dc899d4255ecc28db7bbb0ad285dc82ae464736f6c634300081c0033",
    ).unwrap();

    let tx = TransactionRequest::default().with_deploy_code(bytecode);
    let _receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    let sig = alloy_primitives::hex::decode("11".repeat(65)).unwrap();
    let r = B256::from_slice(&sig[0..32]);
    let s = B256::from_slice(&sig[32..64]);
    let v = sig[64];
    let fake_hash = B256::random();
    let expected = alloy_primitives::address!("0x1234567890123456789012345678901234567890");
    api.anvil_impersonate_signature(sig.clone().into(), expected).await.unwrap();
    let calldata = TestRecover::testRecoverCall { hash: fake_hash, v, r, s, expected }.abi_encode();
    let tx = TransactionRequest::default().with_input(calldata);
    let pending = provider.send_transaction(tx.into()).await.unwrap();
    let result = pending.get_receipt().await;

    assert!(result.is_ok(), "ecrecover failed: {:?}", result.err());
}
