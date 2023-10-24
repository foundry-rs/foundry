//! tests for custom anvil endpoints
use crate::{abi::*, fork::fork_config};
use anvil::{spawn, Hardfork, NodeConfig};
use anvil_core::{
    eth::EthRequest,
    types::{NodeEnvironment, NodeForkConfig, NodeInfo},
};
use ethers::{
    abi::{ethereum_types::BigEndianHash, AbiDecode},
    prelude::{Middleware, SignerMiddleware},
    types::{
        transaction::eip2718::TypedTransaction, Address, BlockNumber, Eip1559TransactionRequest,
        TransactionRequest, H256, U256, U64,
    },
    utils::hex,
};
use foundry_evm::revm::primitives::SpecId;
use std::{
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime},
};

#[tokio::test(flavor = "multi_thread")]
async fn can_set_gas_price() {
    let (api, handle) = spawn(NodeConfig::test().with_hardfork(Some(Hardfork::Berlin))).await;
    let provider = handle.http_provider();

    let gas_price = 1337u64.into();
    api.anvil_set_min_gas_price(gas_price).await.unwrap();
    assert_eq!(gas_price, provider.get_gas_price().await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_set_block_gas_limit() {
    let (api, _) = spawn(NodeConfig::test().with_hardfork(Some(Hardfork::Berlin))).await;

    let block_gas_limit = 1337u64.into();
    assert!(api.evm_set_block_gas_limit(block_gas_limit).unwrap());
    // Mine a new block, and check the new block gas limit
    api.mine_one().await;
    let latest_block = api.block_by_number(BlockNumber::Latest).await.unwrap().unwrap();
    assert_eq!(block_gas_limit, latest_block.gas_limit);
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
    assert_eq!(val, H256::from_uint(&U256::from(12345)));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate_account() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let impersonate = Address::random();
    let to = Address::random();
    let val = 1337u64;
    let funding = U256::from(1e18 as u64);
    // fund the impersonated account
    api.anvil_set_balance(impersonate, funding).await.unwrap();

    let balance = api.balance(impersonate, None).await.unwrap();
    assert_eq!(balance, funding);

    let tx = TransactionRequest::new().from(impersonate).to(to).value(val);

    let res = provider.send_transaction(tx.clone(), None).await;
    res.unwrap_err();

    api.anvil_impersonate_account(impersonate).await.unwrap();
    assert!(api.accounts().unwrap().contains(&impersonate));

    let res = provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(res.from, impersonate);

    let nonce = provider.get_transaction_count(impersonate, None).await.unwrap();
    assert_eq!(nonce, 1u64.into());

    let balance = provider.get_balance(to, None).await.unwrap();
    assert_eq!(balance, val.into());

    api.anvil_stop_impersonating_account(impersonate).await.unwrap();
    let res = provider.send_transaction(tx, None).await;
    res.unwrap_err();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_auto_impersonate_account() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let impersonate = Address::random();
    let to = Address::random();
    let val = 1337u64;
    let funding = U256::from(1e18 as u64);
    // fund the impersonated account
    api.anvil_set_balance(impersonate, funding).await.unwrap();

    let balance = api.balance(impersonate, None).await.unwrap();
    assert_eq!(balance, funding);

    let tx = TransactionRequest::new().from(impersonate).to(to).value(val);

    let res = provider.send_transaction(tx.clone(), None).await;
    res.unwrap_err();

    api.anvil_auto_impersonate_account(true).await.unwrap();

    let res = provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(res.from, impersonate);

    let nonce = provider.get_transaction_count(impersonate, None).await.unwrap();
    assert_eq!(nonce, 1u64.into());

    let balance = provider.get_balance(to, None).await.unwrap();
    assert_eq!(balance, val.into());

    api.anvil_auto_impersonate_account(false).await.unwrap();
    let res = provider.send_transaction(tx, None).await;
    res.unwrap_err();

    // explicitly impersonated accounts get returned by `eth_accounts`
    api.anvil_impersonate_account(impersonate).await.unwrap();
    assert!(api.accounts().unwrap().contains(&impersonate));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate_contract() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let provider = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract =
        Greeter::deploy(provider, "Hello World!".to_string()).unwrap().send().await.unwrap();
    let impersonate = greeter_contract.address();

    let to = Address::random();
    let val = 1337u64;

    let provider = handle.http_provider();

    // fund the impersonated account
    api.anvil_set_balance(impersonate, U256::from(1e18 as u64)).await.unwrap();

    let tx = TransactionRequest::new().from(impersonate).to(to).value(val);

    let res = provider.send_transaction(tx.clone(), None).await;
    res.unwrap_err();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);

    api.anvil_impersonate_account(impersonate).await.unwrap();

    let res = provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(res.from, impersonate);

    let balance = provider.get_balance(to, None).await.unwrap();
    assert_eq!(balance, val.into());

    api.anvil_stop_impersonating_account(impersonate).await.unwrap();
    let res = provider.send_transaction(tx, None).await;
    res.unwrap_err();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate_gnosis_safe() {
    let (api, handle) = spawn(fork_config()).await;
    let provider = handle.http_provider();

    // <https://help.gnosis-safe.io/en/articles/4971293-i-don-t-remember-my-safe-address-where-can-i-find-it>
    let safe: Address = "0xA063Cb7CFd8E57c30c788A0572CBbf2129ae56B6".parse().unwrap();

    let code = provider.get_code(safe, None).await.unwrap();
    assert!(!code.is_empty());

    api.anvil_impersonate_account(safe).await.unwrap();

    let code = provider.get_code(safe, None).await.unwrap();
    assert!(!code.is_empty());

    let balance = U256::from(1e18 as u64);
    // fund the impersonated account
    api.anvil_set_balance(safe, balance).await.unwrap();

    let on_chain_balance = provider.get_balance(safe, None).await.unwrap();
    assert_eq!(on_chain_balance, balance);

    api.anvil_stop_impersonating_account(safe).await.unwrap();

    let code = provider.get_code(safe, None).await.unwrap();
    // code is added back after stop impersonating
    assert!(!code.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_impersonate_multiple_account() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let impersonate0 = Address::random();
    let impersonate1 = Address::random();
    let to = Address::random();

    let val = 1337u64;
    let funding = U256::from(1e18 as u64);
    // fund the impersonated accounts
    api.anvil_set_balance(impersonate0, funding).await.unwrap();
    api.anvil_set_balance(impersonate1, funding).await.unwrap();

    let tx = TransactionRequest::new().from(impersonate0).to(to).value(val);

    api.anvil_impersonate_account(impersonate0).await.unwrap();
    api.anvil_impersonate_account(impersonate1).await.unwrap();

    let res0 = provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(res0.from, impersonate0);

    let nonce = provider.get_transaction_count(impersonate0, None).await.unwrap();
    assert_eq!(nonce, 1u64.into());

    let receipt = provider.get_transaction_receipt(res0.transaction_hash).await.unwrap().unwrap();
    assert_eq!(res0, receipt);

    let res1 = provider
        .send_transaction(tx.from(impersonate1), None)
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(res1.from, impersonate1);

    let nonce = provider.get_transaction_count(impersonate1, None).await.unwrap();
    assert_eq!(nonce, 1u64.into());

    let receipt = provider.get_transaction_receipt(res1.transaction_hash).await.unwrap().unwrap();
    assert_eq!(res1, receipt);

    assert_ne!(res0, res1);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_mine_manually() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let start_num = provider.get_block_number().await.unwrap();

    for (idx, _) in std::iter::repeat(()).take(10).enumerate() {
        api.evm_mine(None).await.unwrap();
        let num = provider.get_block_number().await.unwrap();
        assert_eq!(num, start_num + idx + 1);
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

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    assert_eq!(block.number.unwrap().as_u64(), 1);
    assert_eq!(block.timestamp.as_u64(), next_timestamp.as_secs());

    api.evm_mine(None).await.unwrap();

    let next = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    assert_eq!(next.number.unwrap().as_u64(), 2);

    assert!(next.timestamp > block.timestamp);
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
    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    assert!(block.timestamp.as_u64() >= timestamp.as_secs());

    api.evm_mine(None).await.unwrap();
    let next = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    assert!(next.timestamp > block.timestamp);
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
    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    assert!(block.timestamp.as_u64() >= timestamp.as_secs());
    assert!(block.timestamp.as_u64() < now.as_secs());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_timestamp_interval() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.evm_mine(None).await.unwrap();
    let interval = 10;

    for _ in 0..5 {
        let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

        // mock timestamp
        api.evm_set_block_timestamp_interval(interval).unwrap();
        api.evm_mine(None).await.unwrap();

        let new_block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

        assert_eq!(new_block.timestamp, block.timestamp + interval);
    }

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    let next_timestamp = block.timestamp + 50;
    api.evm_set_next_block_timestamp(next_timestamp.as_u64()).unwrap();

    api.evm_mine(None).await.unwrap();
    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    assert_eq!(block.timestamp, next_timestamp);

    api.evm_mine(None).await.unwrap();

    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    // interval also works after setting the next timestamp manually
    assert_eq!(block.timestamp, next_timestamp + interval);

    assert!(api.evm_remove_block_timestamp_interval().unwrap());

    api.evm_mine(None).await.unwrap();
    let new_block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();

    // offset is applied correctly after resetting the interval
    assert!(new_block.timestamp > block.timestamp);

    api.evm_mine(None).await.unwrap();
    let another_block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    // check interval is disabled
    assert!(another_block.timestamp - new_block.timestamp < U256::from(interval));
}

// <https://github.com/foundry-rs/foundry/issues/2341>
#[tokio::test(flavor = "multi_thread")]
async fn test_can_set_storage_bsc_fork() {
    let (api, handle) =
        spawn(NodeConfig::test().with_eth_rpc_url(Some("https://bsc-dataseed.binance.org/"))).await;
    let provider = Arc::new(handle.http_provider());

    let busd_addr: Address = "0xe9e7CEA3DedcA5984780Bafc599bD69ADd087D56".parse().unwrap();
    let idx: U256 =
        "0xa6eef7e35abe7026729641147f7915573c7e97b47efa546f5f6e3230263bcb49".parse().unwrap();
    let value: H256 =
        "0x0000000000000000000000000000000000000000000000000000000000003039".parse().unwrap();

    api.anvil_set_storage_at(busd_addr, idx, value).await.unwrap();
    let storage = api.storage_at(busd_addr, idx, None).await.unwrap();
    assert_eq!(storage, value);

    let input =
        hex::decode("70a082310000000000000000000000000000000000000000000000000000000000000000")
            .unwrap();

    let busd = BUSD::new(busd_addr, provider);
    let call = busd::BalanceOfCall::decode(&input).unwrap();

    let balance = busd.balance_of(call.0).call().await.unwrap();
    assert_eq!(balance, U256::from(12345u64));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_node_info() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let node_info = api.anvil_node_info().await.unwrap();

    let provider = handle.http_provider();

    let block_number = provider.get_block_number().await.unwrap();
    let block = provider.get_block(block_number).await.unwrap().unwrap();

    let expected_node_info = NodeInfo {
        current_block_number: U64([0]),
        current_block_timestamp: 1,
        current_block_hash: block.hash.unwrap(),
        hard_fork: SpecId::SHANGHAI,
        transaction_order: "fees".to_owned(),
        environment: NodeEnvironment {
            base_fee: U256::from_str("0x3b9aca00").unwrap(),
            chain_id: U256::from_str("0x7a69").unwrap(),
            gas_limit: U256::from_str("0x1c9c380").unwrap(),
            gas_price: U256::from_str("0x77359400").unwrap(),
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
async fn test_get_transaction_receipt() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    // set the base fee
    let new_base_fee = U256::from(1_000);
    api.anvil_set_next_block_base_fee_per_gas(new_base_fee).await.unwrap();

    // send a EIP-1559 transaction
    let tx =
        TypedTransaction::Eip1559(Eip1559TransactionRequest::new().gas(U256::from(30_000_000)));
    let receipt =
        provider.send_transaction(tx.clone(), None).await.unwrap().await.unwrap().unwrap();

    // the block should have the new base fee
    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    assert_eq!(block.base_fee_per_gas.unwrap().as_u64(), new_base_fee.as_u64());

    // mine block
    api.evm_mine(None).await.unwrap();

    // the transaction receipt should have the original effective gas price
    let new_receipt = provider.get_transaction_receipt(receipt.transaction_hash).await.unwrap();
    assert_eq!(
        receipt.effective_gas_price.unwrap().as_u64(),
        new_receipt.unwrap().effective_gas_price.unwrap().as_u64()
    );
}

// test can set chain id
#[tokio::test(flavor = "multi_thread")]
async fn test_set_chain_id() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let chain_id = provider.get_chainid().await.unwrap();
    assert_eq!(chain_id, U256::from(31337));

    let chain_id = 1234;
    api.anvil_set_chain_id(chain_id).await.unwrap();

    let chain_id = provider.get_chainid().await.unwrap();
    assert_eq!(chain_id, U256::from(1234));
}

// <https://github.com/foundry-rs/foundry/issues/6096>
#[tokio::test(flavor = "multi_thread")]
async fn test_fork_revert_next_block_timestamp() {
    let (api, _handle) = spawn(fork_config()).await;

    // Mine a new block, and check the new block gas limit
    api.mine_one().await;
    let latest_block = api.block_by_number(BlockNumber::Latest).await.unwrap().unwrap();

    let snapshot_id = api.evm_snapshot().await.unwrap();
    api.mine_one().await;
    api.evm_revert(snapshot_id).await.unwrap();
    let block = api.block_by_number(BlockNumber::Latest).await.unwrap().unwrap();
    assert_eq!(block, latest_block);

    api.mine_one().await;
    let block = api.block_by_number(BlockNumber::Latest).await.unwrap().unwrap();
    assert!(block.timestamp > latest_block.timestamp);
}
