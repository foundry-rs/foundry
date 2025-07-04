//! general eth api tests

use crate::{
    abi::{Multicall, SimpleStorage},
    utils::{connect_pubsub_with_wallet, http_provider, http_provider_with_signer},
};
use alloy_consensus::{SignableTransaction, Transaction, TxEip1559};
use alloy_network::{EthereumWallet, TransactionBuilder, TxSignerSync};
use alloy_primitives::{
    Address, B256, ChainId, U256, bytes,
    map::{AddressHashMap, B256HashMap, HashMap},
};
use alloy_provider::Provider;
use alloy_rpc_types::{
    BlockId, BlockNumberOrTag, BlockTransactions, request::TransactionRequest,
    state::AccountOverride,
};
use alloy_serde::WithOtherFields;
use anvil::{CHAIN_ID, EthereumHardfork, NodeConfig, eth::api::CLIENT_VERSION, spawn};
use futures::join;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_number() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::from(0));

    let provider = handle.http_provider();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.to::<u64>());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_dev_get_balance() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let genesis_balance = handle.genesis_balance();
    for acc in handle.genesis_accounts() {
        let balance = provider.get_balance(acc).await.unwrap();
        assert_eq!(balance, genesis_balance);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_price() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let _ = provider.get_gas_price().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_accounts() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let _ = provider.get_accounts().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_client_version() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let version = provider.get_client_version().await.unwrap();
    assert_eq!(CLIENT_VERSION, version);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_chain_id() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, CHAIN_ID);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_modify_chain_id() {
    let (_api, handle) =
        spawn(NodeConfig::test().with_chain_id(Some(ChainId::from(777_u64)))).await;
    let provider = handle.http_provider();

    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, 777);

    let chain_id = provider.get_net_version().await.unwrap();
    assert_eq!(chain_id, 777);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_network_id() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    let chain_id = api.network_id().unwrap().unwrap();
    assert_eq!(chain_id, CHAIN_ID.to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_by_number() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let signer: EthereumWallet = accounts[0].clone().into();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let val = handle.genesis_balance().checked_div(U256::from(2)).unwrap();

    // send a dummy transaction
    let tx = TransactionRequest::default().with_from(from).with_to(to).with_value(val);
    let tx = WithOtherFields::new(tx);

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let block = provider.get_block(BlockId::number(1)).full().await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 1);

    let block = provider.get_block(BlockId::hash(block.header.hash)).full().await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_pending_block() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let signer: EthereumWallet = accounts[0].clone().into();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let provider = connect_pubsub_with_wallet(&handle.http_endpoint(), signer).await;

    let block = provider.get_block(BlockId::pending()).await.unwrap().unwrap();
    assert_eq!(block.header.number, 1);

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 0);

    api.anvil_set_auto_mine(false).await.unwrap();

    let tx = TransactionRequest::default().with_from(from).with_to(to).with_value(U256::from(100));

    let pending = provider.send_transaction(tx.clone()).await.unwrap().register().await.unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 0);

    let block = provider.get_block(BlockId::pending()).await.unwrap().unwrap();
    assert_eq!(block.header.number, 1);
    assert_eq!(block.transactions.len(), 1);
    assert_eq!(block.transactions, BlockTransactions::Hashes(vec![*pending.tx_hash()]));

    let block = provider.get_block(BlockId::pending()).full().await.unwrap().unwrap();
    assert_eq!(block.header.number, 1);
    assert_eq!(block.transactions.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_estimate_gas_with_undersized_max_fee_per_gas() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    api.anvil_set_auto_mine(true).await.unwrap();

    let init_value = "toto".to_string();

    let simple_storage_contract =
        SimpleStorage::deploy(&provider, init_value.clone()).await.unwrap();

    let undersized_max_fee_per_gas = 1;

    let latest_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    let latest_block_base_fee_per_gas = latest_block.header.base_fee_per_gas.unwrap();

    assert!(undersized_max_fee_per_gas < latest_block_base_fee_per_gas);

    let estimated_gas = simple_storage_contract
        .setValue("new_value".to_string())
        .max_fee_per_gas(undersized_max_fee_per_gas.into())
        .from(wallet.address())
        .estimate_gas()
        .await
        .unwrap();

    assert!(estimated_gas > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_on_pending_block() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 0);

    api.anvil_set_auto_mine(false).await.unwrap();

    let _contract_pending = Multicall::deploy_builder(&provider)
        .from(wallet.address())
        .send()
        .await
        .unwrap()
        .register()
        .await
        .unwrap();
    let contract_address = sender.create(0);
    let contract = Multicall::new(contract_address, &provider);

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 0);

    // Ensure that we can get the block_number from the pending contract
    let Multicall::aggregateReturn { blockNumber: ret_block_number, .. } =
        contract.aggregate(vec![]).block(BlockId::pending()).call().await.unwrap();
    assert_eq!(ret_block_number, U256::from(1));

    let accounts: Vec<Address> = handle.dev_wallets().map(|w| w.address()).collect();

    for i in 1..10 {
        api.anvil_set_coinbase(accounts[i % accounts.len()]).await.unwrap();
        api.evm_set_block_gas_limit(U256::from(30_000_000 + i)).unwrap();

        api.anvil_mine(Some(U256::from(1)), None).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Ensure that the right header values are set when calling a past block
    for anvil_block_number in 1..(api.block_number().unwrap().to::<usize>() + 1) {
        let block_number = BlockNumberOrTag::Number(anvil_block_number as u64);
        let block = api.block_by_number(block_number).await.unwrap().unwrap();

        let ret_timestamp = contract
            .getCurrentBlockTimestamp()
            .block(BlockId::number(anvil_block_number as u64))
            .call()
            .await
            .unwrap();
        assert_eq!(block.header.timestamp, ret_timestamp.to::<u64>());

        let ret_gas_limit = contract
            .getCurrentBlockGasLimit()
            .block(BlockId::number(anvil_block_number as u64))
            .call()
            .await
            .unwrap();
        assert_eq!(block.header.gas_limit, ret_gas_limit.to::<u64>());

        let ret_coinbase = contract
            .getCurrentBlockCoinbase()
            .block(BlockId::number(anvil_block_number as u64))
            .call()
            .await
            .unwrap();
        assert_eq!(block.header.beneficiary, ret_coinbase);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_with_undersized_max_fee_per_gas() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    api.anvil_set_auto_mine(true).await.unwrap();

    let init_value = "toto".to_string();

    let simple_storage_contract =
        SimpleStorage::deploy(&provider, init_value.clone()).await.unwrap();

    let latest_block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();
    let latest_block_base_fee_per_gas = latest_block.header.base_fee_per_gas.unwrap();
    let undersized_max_fee_per_gas = 1;

    assert!(undersized_max_fee_per_gas < latest_block_base_fee_per_gas);

    let last_sender = simple_storage_contract
        .lastSender()
        .max_fee_per_gas(undersized_max_fee_per_gas.into())
        .from(wallet.address())
        .call()
        .await
        .unwrap();
    assert_eq!(last_sender, Address::ZERO);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_with_state_override() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumWallet = wallet.clone().into();
    let account = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    api.anvil_set_auto_mine(true).await.unwrap();

    let multicall_contract = Multicall::deploy(&provider).await.unwrap();

    let init_value = "toto".to_string();

    let simple_storage_contract =
        SimpleStorage::deploy(&provider, init_value.clone()).await.unwrap();

    // Test the `balance` account override
    let balance = U256::from(42u64);
    let mut overrides = AddressHashMap::default();
    overrides.insert(account, AccountOverride { balance: Some(balance), ..Default::default() });
    let result = multicall_contract.getEthBalance(account).state(overrides).call().await.unwrap();
    assert_eq!(result, balance);

    // Test the `state_diff` account override
    let mut state_diff = B256HashMap::default();
    state_diff.insert(B256::ZERO, account.into_word());
    let mut overrides = AddressHashMap::default();
    overrides.insert(
        *simple_storage_contract.address(),
        AccountOverride {
            // The `lastSender` is in the first storage slot
            state_diff: Some(state_diff),
            ..Default::default()
        },
    );

    let last_sender =
        simple_storage_contract.lastSender().state(HashMap::default()).call().await.unwrap();
    // No `sender` set without override
    assert_eq!(last_sender, Address::ZERO);

    let last_sender =
        simple_storage_contract.lastSender().state(overrides.clone()).call().await.unwrap();
    // `sender` *is* set with override
    assert_eq!(last_sender, account);

    let value = simple_storage_contract.getValue().state(overrides).call().await.unwrap();
    // `value` *is not* changed with state-diff
    assert_eq!(value, init_value);

    // Test the `state` account override
    let mut state = B256HashMap::default();
    state.insert(B256::ZERO, account.into_word());
    let mut overrides = AddressHashMap::default();
    overrides.insert(
        *simple_storage_contract.address(),
        AccountOverride {
            // The `lastSender` is in the first storage slot
            state: Some(state),
            ..Default::default()
        },
    );

    let last_sender =
        simple_storage_contract.lastSender().state(overrides.clone()).call().await.unwrap();
    // `sender` *is* set with override
    assert_eq!(last_sender, account);

    let value = simple_storage_contract.getValue().state(overrides).call().await.unwrap();
    // `value` *is* changed with state
    assert_eq!(value, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn can_mine_while_mining() {
    let (api, _) = spawn(NodeConfig::test()).await;

    let total_blocks = 200;

    let block_number =
        api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap().header.number;
    assert_eq!(block_number, 0);

    let block = api.block_by_number(BlockNumberOrTag::Number(block_number)).await.unwrap().unwrap();
    assert_eq!(block.header.number, 0);

    let result = join!(
        api.anvil_mine(Some(U256::from(total_blocks / 2)), None),
        api.anvil_mine(Some(U256::from(total_blocks / 2)), None)
    );
    result.0.unwrap();
    result.1.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let block_number =
        api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap().header.number;
    assert_eq!(block_number, total_blocks);

    let block = api.block_by_number(BlockNumberOrTag::Number(block_number)).await.unwrap().unwrap();
    assert_eq!(block.header.number, total_blocks);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_send_raw_tx_sync() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();

    let from = wallets[0].address();
    let mut tx = TxEip1559 {
        max_fee_per_gas: eip1559_est.max_fee_per_gas,
        max_priority_fee_per_gas: eip1559_est.max_priority_fee_per_gas,
        gas_limit: 100000,
        chain_id: 31337,
        to: alloy_primitives::TxKind::Call(from),
        input: bytes!("11112222"),
        ..Default::default()
    };
    let signature = wallets[1].sign_transaction_sync(&mut tx).unwrap();

    let tx = tx.into_signed(signature);
    let mut encoded = Vec::new();
    tx.eip2718_encode(&mut encoded);

    let receipt = api.send_raw_transaction_sync(encoded.into()).await.unwrap();
    assert_eq!(receipt.from, wallets[1].address());
    assert_eq!(receipt.to, tx.to());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_send_tx_sync() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;

    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let logger_bytecode = bytes!("66365f5f37365fa05f5260076019f3");

    let from = wallets[0].address();
    let tx = TransactionRequest::default()
        .with_from(from)
        .into_create()
        .with_nonce(0)
        .with_input(logger_bytecode);

    let receipt = api.send_transaction_sync(WithOtherFields::new(tx)).await.unwrap();
    assert_eq!(receipt.from, wallets[0].address());
}
