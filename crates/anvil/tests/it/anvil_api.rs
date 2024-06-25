//! tests for custom anvil endpoints

use crate::{
    abi::{Greeter, MulticallContract, BUSD},
    fork::fork_config,
    utils::http_provider_with_signer,
};
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{address, fixed_bytes, Address, U256};
use alloy_provider::{ext::TxPoolApi, Provider};
use alloy_rpc_types::{
    anvil::{ForkedNetwork, Forking, Metadata, NodeEnvironment, NodeForkConfig, NodeInfo},
    BlockId, BlockNumberOrTag, TransactionRequest,
};
use alloy_serde::WithOtherFields;
use anvil::{eth::api::CLIENT_VERSION, spawn, Hardfork, NodeConfig};
use anvil_core::eth::EthRequest;
use foundry_evm::revm::primitives::SpecId;
use std::{
    str::FromStr,
    time::{Duration, SystemTime},
};

#[tokio::test(flavor = "multi_thread")]
async fn can_set_gas_price() {
    let (api, handle) = spawn(NodeConfig::test().with_hardfork(Some(Hardfork::Berlin))).await;
    let provider = handle.http_provider();

    let gas_price = U256::from(1337);
    api.anvil_set_min_gas_price(gas_price).await.unwrap();
    assert_eq!(gas_price.to::<u128>(), provider.get_gas_price().await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_set_block_gas_limit() {
    let (api, _) = spawn(NodeConfig::test().with_hardfork(Some(Hardfork::Berlin))).await;

    let block_gas_limit = U256::from(1337);
    assert!(api.evm_set_block_gas_limit(block_gas_limit).unwrap());
    // Mine a new block, and check the new block gas limit
    api.mine_one().await;
    let latest_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(block_gas_limit.to::<u128>(), latest_block.header.gas_limit);
}

// Ref <https://github.com/foundry-rs/foundry/issues/2341>
#[tokio::test(flavor = "multi_thread")]
async fn can_set_storage() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    let s = r#"{"jsonrpc": "2.0", "method": "hardhat_setStorageAt", "id": 1, "params": ["0xe9e7CEA3DedcA5984780Bafc599bD69ADd087D56", "0xa6eef7e35abe7026729641147f7915573c7e97b47efa546f5f6e3230263bcb49", "0x0000000000000000000000000000000000000000000000000000000000003039"]}"#;
    let req = serde_json::from_str::<EthRequest>(s).unwrap();
    let (addr, slot, val) = match req.clone() {
        EthRequest::SetStorageAt(addr, slot, val) => (addr, slot, val),
        _ => unreachable!(),
    };

    api.execute(req).await;

    let storage_value = api.storage_at(addr, slot, None).await.unwrap();
    assert_eq!(val, storage_value);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate_account() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.http_provider();

    let impersonate = Address::random();
    let to = Address::random();
    let val = U256::from(1337);
    let funding = U256::from(1e18 as u64);
    // fund the impersonated account
    api.anvil_set_balance(impersonate, funding).await.unwrap();

    let balance = api.balance(impersonate, None).await.unwrap();
    assert_eq!(balance, funding);

    let tx = TransactionRequest::default().with_from(impersonate).with_to(to).with_value(val);
    let tx = WithOtherFields::new(tx);

    let res = provider.send_transaction(tx.clone()).await;
    res.unwrap_err();

    api.anvil_impersonate_account(impersonate).await.unwrap();
    assert!(api.accounts().unwrap().contains(&impersonate));

    let res = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
    assert_eq!(res.from, impersonate);

    let nonce = provider.get_transaction_count(impersonate).await.unwrap();
    assert_eq!(nonce, 1);

    let balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance, val);

    api.anvil_stop_impersonating_account(impersonate).await.unwrap();
    let res = provider.send_transaction(tx).await;
    res.unwrap_err();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_auto_impersonate_account() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.http_provider();

    let impersonate = Address::random();
    let to = Address::random();
    let val = U256::from(1337);
    let funding = U256::from(1e18 as u64);
    // fund the impersonated account
    api.anvil_set_balance(impersonate, funding).await.unwrap();

    let balance = api.balance(impersonate, None).await.unwrap();
    assert_eq!(balance, funding);

    let tx = TransactionRequest::default().with_from(impersonate).with_to(to).with_value(val);
    let tx = WithOtherFields::new(tx);

    let res = provider.send_transaction(tx.clone()).await;
    res.unwrap_err();

    api.anvil_auto_impersonate_account(true).await.unwrap();

    let res = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
    assert_eq!(res.from, impersonate);

    let nonce = provider.get_transaction_count(impersonate).await.unwrap();
    assert_eq!(nonce, 1);

    let balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance, val);

    api.anvil_auto_impersonate_account(false).await.unwrap();
    let res = provider.send_transaction(tx).await;
    res.unwrap_err();

    // explicitly impersonated accounts get returned by `eth_accounts`
    api.anvil_impersonate_account(impersonate).await.unwrap();
    assert!(api.accounts().unwrap().contains(&impersonate));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate_contract() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.http_provider();

    let greeter_contract = Greeter::deploy(&provider, "Hello World!".to_string()).await.unwrap();
    let impersonate = greeter_contract.address().to_owned();

    let to = Address::random();
    let val = U256::from(1337);

    // // fund the impersonated account
    api.anvil_set_balance(impersonate, U256::from(1e18 as u64)).await.unwrap();

    let tx = TransactionRequest::default().with_from(impersonate).to(to).with_value(val);
    let tx = WithOtherFields::new(tx);

    let res = provider.send_transaction(tx.clone()).await;
    res.unwrap_err();

    let greeting = greeter_contract.greet().call().await.unwrap()._0;
    assert_eq!("Hello World!", greeting);

    api.anvil_impersonate_account(impersonate).await.unwrap();

    let res = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
    assert_eq!(res.from, impersonate);

    let balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance, val);

    api.anvil_stop_impersonating_account(impersonate).await.unwrap();
    let res = provider.send_transaction(tx).await;
    res.unwrap_err();

    let greeting = greeter_contract.greet().call().await.unwrap()._0;
    assert_eq!("Hello World!", greeting);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate_gnosis_safe() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    // <https://help.safe.global/en/articles/40824-i-don-t-remember-my-safe-address-where-can-i-find-it>
    let safe = address!("A063Cb7CFd8E57c30c788A0572CBbf2129ae56B6");

    let code = provider.get_code_at(safe).await.unwrap();
    assert!(!code.is_empty());

    api.anvil_impersonate_account(safe).await.unwrap();

    let code = provider.get_code_at(safe).await.unwrap();
    assert!(!code.is_empty());

    let balance = U256::from(1e18 as u64);
    // fund the impersonated account
    api.anvil_set_balance(safe, balance).await.unwrap();

    let on_chain_balance = provider.get_balance(safe).await.unwrap();
    assert_eq!(on_chain_balance, balance);

    api.anvil_stop_impersonating_account(safe).await.unwrap();

    let code = provider.get_code_at(safe).await.unwrap();
    // code is added back after stop impersonating
    assert!(!code.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate_multiple_accounts() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let impersonate0 = Address::random();
    let impersonate1 = Address::random();
    let to = Address::random();

    let val = U256::from(1337);
    let funding = U256::from(1e18 as u64);
    // fund the impersonated accounts
    api.anvil_set_balance(impersonate0, funding).await.unwrap();
    api.anvil_set_balance(impersonate1, funding).await.unwrap();

    let tx = TransactionRequest::default().with_from(impersonate0).to(to).with_value(val);
    let tx = WithOtherFields::new(tx);

    api.anvil_impersonate_account(impersonate0).await.unwrap();
    api.anvil_impersonate_account(impersonate1).await.unwrap();

    let res0 = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();
    assert_eq!(res0.from, impersonate0);

    let nonce = provider.get_transaction_count(impersonate0).await.unwrap();
    assert_eq!(nonce, 1);

    let receipt = provider.get_transaction_receipt(res0.transaction_hash).await.unwrap().unwrap();
    assert_eq!(res0.inner, receipt.inner);

    let res1 = provider
        .send_transaction(tx.with_from(impersonate1))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert_eq!(res1.from, impersonate1);

    let nonce = provider.get_transaction_count(impersonate1).await.unwrap();
    assert_eq!(nonce, 1);

    let receipt = provider.get_transaction_receipt(res1.transaction_hash).await.unwrap().unwrap();
    assert_eq!(res1.inner, receipt.inner);

    assert_ne!(res0.inner, res1.inner);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_mine_manually() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let start_num = provider.get_block_number().await.unwrap();

    for (idx, _) in std::iter::repeat(()).take(10).enumerate() {
        api.evm_mine(None).await.unwrap();
        let num = provider.get_block_number().await.unwrap();
        assert_eq!(num, start_num + idx as u64 + 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_set_next_timestamp() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();

    let next_timestamp = now + Duration::from_secs(60);

    // mock timestamp
    api.evm_set_next_block_timestamp(next_timestamp.as_secs()).unwrap();

    api.evm_mine(None).await.unwrap();

    let block = provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();

    assert_eq!(block.header.number.unwrap(), 1);
    assert_eq!(block.header.timestamp, next_timestamp.as_secs());

    api.evm_mine(None).await.unwrap();

    let next = provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();
    assert_eq!(next.header.number.unwrap(), 2);

    assert!(next.header.timestamp > block.header.timestamp);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_evm_set_time() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();

    let timestamp = now + Duration::from_secs(120);

    // mock timestamp
    api.evm_set_time(timestamp.as_secs()).unwrap();

    // mine a block
    api.evm_mine(None).await.unwrap();
    let block = provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();

    assert!(block.header.timestamp >= timestamp.as_secs());

    api.evm_mine(None).await.unwrap();
    let next = provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();

    assert!(next.header.timestamp > block.header.timestamp);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_evm_set_time_in_past() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();

    let timestamp = now - Duration::from_secs(120);

    // mock timestamp
    api.evm_set_time(timestamp.as_secs()).unwrap();

    // mine a block
    api.evm_mine(None).await.unwrap();
    let block = provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();

    assert!(block.header.timestamp >= timestamp.as_secs());
    assert!(block.header.timestamp < now.as_secs());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_timestamp_interval() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.evm_mine(None).await.unwrap();
    let interval = 10;

    for _ in 0..5 {
        let block = provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();

        // mock timestamp
        api.evm_set_block_timestamp_interval(interval).unwrap();
        api.evm_mine(None).await.unwrap();

        let new_block =
            provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();

        assert_eq!(new_block.header.timestamp, block.header.timestamp + interval);
    }

    let block = provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();

    let next_timestamp = block.header.timestamp + 50;
    api.evm_set_next_block_timestamp(next_timestamp).unwrap();

    api.evm_mine(None).await.unwrap();
    let block = provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();
    assert_eq!(block.header.timestamp, next_timestamp);

    api.evm_mine(None).await.unwrap();

    let block = provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();
    // interval also works after setting the next timestamp manually
    assert_eq!(block.header.timestamp, next_timestamp + interval);

    assert!(api.evm_remove_block_timestamp_interval().unwrap());

    api.evm_mine(None).await.unwrap();
    let new_block = provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();

    // offset is applied correctly after resetting the interval
    assert!(new_block.header.timestamp > block.header.timestamp);

    api.evm_mine(None).await.unwrap();
    let another_block =
        provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();
    // check interval is disabled
    assert!(another_block.header.timestamp - new_block.header.timestamp < interval);
}

// <https://github.com/foundry-rs/foundry/issues/2341>
#[tokio::test(flavor = "multi_thread")]
async fn test_can_set_storage_bsc_fork() {
    let (api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some("https://bsc-dataseed.binance.org/"))).await;
    let provider = handle.http_provider();

    let busd_addr = address!("e9e7CEA3DedcA5984780Bafc599bD69ADd087D56");
    let idx = U256::from_str("0xa6eef7e35abe7026729641147f7915573c7e97b47efa546f5f6e3230263bcb49")
        .unwrap();
    let value = fixed_bytes!("0000000000000000000000000000000000000000000000000000000000003039");

    api.anvil_set_storage_at(busd_addr, idx, value).await.unwrap();
    let storage = api.storage_at(busd_addr, idx, None).await.unwrap();
    assert_eq!(storage, value);

    let busd_contract = BUSD::new(busd_addr, &provider);

    let BUSD::balanceOfReturn { _0 } = busd_contract
        .balanceOf(address!("0000000000000000000000000000000000000000"))
        .call()
        .await
        .unwrap();
    let balance = _0;
    assert_eq!(balance, U256::from(12345u64));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_node_info() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let node_info = api.anvil_node_info().await.unwrap();

    let provider = handle.http_provider();

    let block_number = provider.get_block_number().await.unwrap();
    let block =
        provider.get_block(BlockId::from(block_number), false.into()).await.unwrap().unwrap();
    let hard_fork: &str = SpecId::CANCUN.into();

    let expected_node_info = NodeInfo {
        current_block_number: 0_u64,
        current_block_timestamp: 1,
        current_block_hash: block.header.hash.unwrap(),
        hard_fork: hard_fork.to_string(),
        transaction_order: "fees".to_owned(),
        environment: NodeEnvironment {
            base_fee: U256::from_str("0x3b9aca00").unwrap().to(),
            chain_id: 0x7a69,
            gas_limit: U256::from_str("0x1c9c380").unwrap().to(),
            gas_price: U256::from_str("0x77359400").unwrap().to(),
        },
        fork_config: NodeForkConfig {
            fork_url: None,
            fork_block_number: None,
            fork_retry_backoff: None,
        },
    };

    assert_eq!(node_info, expected_node_info);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_metadata() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let metadata = api.anvil_metadata().await.unwrap();

    let provider = handle.http_provider();

    let block_number = provider.get_block_number().await.unwrap();
    let chain_id = provider.get_chain_id().await.unwrap();
    let block =
        provider.get_block(BlockId::from(block_number), false.into()).await.unwrap().unwrap();

    let expected_metadata = Metadata {
        latest_block_hash: block.header.hash.unwrap(),
        latest_block_number: block_number,
        chain_id,
        client_version: CLIENT_VERSION.to_string(),
        instance_id: api.instance_id(),
        forked_network: None,
        snapshots: Default::default(),
    };

    assert_eq!(metadata, expected_metadata);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_metadata_on_fork() {
    let (api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some("https://bsc-dataseed.binance.org/"))).await;
    let provider = handle.http_provider();

    let metadata = api.anvil_metadata().await.unwrap();

    let block_number = provider.get_block_number().await.unwrap();
    let chain_id = provider.get_chain_id().await.unwrap();
    let block =
        provider.get_block(BlockId::from(block_number), false.into()).await.unwrap().unwrap();

    let expected_metadata = Metadata {
        latest_block_hash: block.header.hash.unwrap(),
        latest_block_number: block_number,
        chain_id,
        client_version: CLIENT_VERSION.to_string(),
        instance_id: api.instance_id(),
        forked_network: Some(ForkedNetwork {
            chain_id,
            fork_block_number: block_number,
            fork_block_hash: block.header.hash.unwrap(),
        }),
        snapshots: Default::default(),
    };

    assert_eq!(metadata, expected_metadata);
}

#[tokio::test(flavor = "multi_thread")]
async fn metadata_changes_on_reset() {
    let (api, _) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some("https://bsc-dataseed.binance.org/"))).await;

    let metadata = api.anvil_metadata().await.unwrap();
    let instance_id = metadata.instance_id;

    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: None })).await.unwrap();

    let new_metadata = api.anvil_metadata().await.unwrap();
    let new_instance_id = new_metadata.instance_id;

    assert_ne!(instance_id, new_instance_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_transaction_receipt() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    // set the base fee
    let new_base_fee = U256::from(1000);
    api.anvil_set_next_block_base_fee_per_gas(new_base_fee).await.unwrap();

    // send a EIP-1559 transaction
    let to = Address::random();
    let val = U256::from(1337);
    let tx = TransactionRequest::default().with_to(to).with_value(val);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    // the block should have the new base fee
    let block = provider.get_block(BlockId::default(), false.into()).await.unwrap().unwrap();
    assert_eq!(block.header.base_fee_per_gas.unwrap(), new_base_fee.to::<u128>());

    // mine blocks
    api.evm_mine(None).await.unwrap();

    // the transaction receipt should have the original effective gas price
    let new_receipt = provider.get_transaction_receipt(receipt.transaction_hash).await.unwrap();
    assert_eq!(receipt.effective_gas_price, new_receipt.unwrap().effective_gas_price);
}

// test can set chain id
#[tokio::test(flavor = "multi_thread")]
async fn test_set_chain_id() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, 31337);

    let chain_id = 1234;
    api.anvil_set_chain_id(chain_id).await.unwrap();

    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, 1234);
}

// <https://github.com/foundry-rs/foundry/issues/6096>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_revert_next_block_timestamp() {
    let (api, _handle) = spawn(fork_config()).await;

    // Mine a new block, and check the new block gas limit
    api.mine_one().await;
    let latest_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    let snapshot_id = api.evm_snapshot().await.unwrap();
    api.mine_one().await;
    api.evm_revert(snapshot_id).await.unwrap();
    let block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert_eq!(block, latest_block);

    api.mine_one().await;
    let block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    assert!(block.header.timestamp > latest_block.header.timestamp);
}

// test that after a snapshot revert, the env block is reset
// to its correct value (block number, etc.)
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_revert_call_latest_block_timestamp() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    // Mine a new block, and check the new block gas limit
    api.mine_one().await;
    let latest_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    let snapshot_id = api.evm_snapshot().await.unwrap();
    api.mine_one().await;
    api.evm_revert(snapshot_id).await.unwrap();

    let multicall_contract =
        MulticallContract::new(address!("eefba1e63905ef1d7acba5a8513c70307c1ce441"), &provider);

    let MulticallContract::getCurrentBlockTimestampReturn { timestamp } =
        multicall_contract.getCurrentBlockTimestamp().call().await.unwrap();
    assert_eq!(timestamp, U256::from(latest_block.header.timestamp));

    let MulticallContract::getCurrentBlockDifficultyReturn { difficulty } =
        multicall_contract.getCurrentBlockDifficulty().call().await.unwrap();
    assert_eq!(difficulty, U256::from(latest_block.header.difficulty));

    let MulticallContract::getCurrentBlockGasLimitReturn { gaslimit } =
        multicall_contract.getCurrentBlockGasLimit().call().await.unwrap();
    assert_eq!(gaslimit, U256::from(latest_block.header.gas_limit));

    let MulticallContract::getCurrentBlockCoinbaseReturn { coinbase } =
        multicall_contract.getCurrentBlockCoinbase().call().await.unwrap();
    assert_eq!(coinbase, latest_block.header.miner);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_remove_pool_transactions() {
    let (api, handle) =
        spawn(NodeConfig::test().with_blocktime(Some(Duration::from_secs(5)))).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();
    let from = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let sender = Address::random();
    let to = Address::random();
    let val = U256::from(1337);
    let tx = TransactionRequest::default().with_from(sender).with_to(to).with_value(val);
    let tx = WithOtherFields::new(tx);

    provider.send_transaction(tx.with_from(from)).await.unwrap().register().await.unwrap();

    let initial_txs = provider.txpool_inspect().await.unwrap();
    assert_eq!(initial_txs.pending.len(), 1);

    api.anvil_remove_pool_transactions(wallet.address()).await.unwrap();

    let final_txs = provider.txpool_inspect().await.unwrap();
    assert_eq!(final_txs.pending.len(), 0);
}
