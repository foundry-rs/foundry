use crate::{
    abi::{Greeter, Multicall, SimpleStorage},
    utils::{connect_pubsub, http_provider_with_signer},
};
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, Bytes, FixedBytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{
    state::{AccountOverride, StateOverride},
    AccessList, AccessListItem, BlockId, BlockNumberOrTag, BlockTransactions, TransactionRequest,
};
use alloy_serde::WithOtherFields;
use anvil::{spawn, EthereumHardfork, NodeConfig};
use eyre::Ok;
use futures::{future::join_all, FutureExt, StreamExt};
use std::{collections::HashSet, str::FromStr, time::Duration};
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread")]
async fn can_transfer_eth() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from).await.unwrap();
    assert!(nonce == 0);

    let balance_before = provider.get_balance(to).await.unwrap();

    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();

    // craft the tx
    // specify the `from` field so that the client knows which account to use
    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);
    // broadcast it via the eth_sendTransaction API
    let tx = provider.send_transaction(tx).await.unwrap();

    let tx = tx.get_receipt().await.unwrap();

    assert_eq!(tx.block_number, Some(1));
    assert_eq!(tx.transaction_index, Some(0));

    let nonce = provider.get_transaction_count(from).await.unwrap();

    assert_eq!(nonce, 1);

    let to_balance = provider.get_balance(to).await.unwrap();

    assert_eq!(balance_before.saturating_add(amount), to_balance);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_order_transactions() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    // disable automine
    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();

    let gas_price = provider.get_gas_price().await.unwrap();

    // craft the tx with lower price
    let mut tx = TransactionRequest::default().to(to).from(from).value(amount);

    tx.set_gas_price(gas_price);
    let tx = WithOtherFields::new(tx);
    let tx_lower = provider.send_transaction(tx).await.unwrap();

    // craft the tx with higher price
    let mut tx = TransactionRequest::default().to(from).from(to).value(amount);

    tx.set_gas_price(gas_price + 1);
    let tx = WithOtherFields::new(tx);
    let tx_higher = provider.send_transaction(tx).await.unwrap();

    // manually mine the block with the transactions
    api.mine_one().await;

    let higher_price = tx_higher.get_receipt().await.unwrap().transaction_hash;
    let lower_price = tx_lower.get_receipt().await.unwrap().transaction_hash;

    // get the block, await receipts
    let block = provider.get_block(BlockId::latest(), false.into()).await.unwrap().unwrap();

    assert_eq!(block.transactions, BlockTransactions::Hashes(vec![higher_price, lower_price]))
}

#[tokio::test(flavor = "multi_thread")]
async fn can_respect_nonces() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from).await.unwrap();
    let amount = handle.genesis_balance().checked_div(U256::from(3u64)).unwrap();

    let tx = TransactionRequest::default().to(to).value(amount).from(from).nonce(nonce + 1);

    let tx = WithOtherFields::new(tx);

    // send the transaction with higher nonce than on chain
    let higher_pending_tx = provider.send_transaction(tx).await.unwrap();

    // ensure the listener for ready transactions times out
    let mut listener = api.new_ready_transactions();
    let res = timeout(Duration::from_millis(1500), listener.next()).await;
    res.unwrap_err();

    let tx = TransactionRequest::default().to(to).value(amount).from(from).nonce(nonce);

    let tx = WithOtherFields::new(tx);
    // send with the actual nonce which is mined immediately
    let tx = provider.send_transaction(tx).await.unwrap();

    let tx = tx.get_receipt().await.unwrap();
    // this will unblock the currently pending tx
    let higher_tx = higher_pending_tx.get_receipt().await.unwrap(); // Awaits endlessly here due to alloy/#389

    let block = provider.get_block(1.into(), false.into()).await.unwrap().unwrap();
    assert_eq!(2, block.transactions.len());
    assert_eq!(
        BlockTransactions::Hashes(vec![tx.transaction_hash, higher_tx.transaction_hash]),
        block.transactions
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_replace_transaction() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    // disable auto mining
    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();
    let amount = handle.genesis_balance().checked_div(U256::from(3u64)).unwrap();

    let tx = TransactionRequest::default().to(to).value(amount).from(from).nonce(nonce);

    let mut tx = WithOtherFields::new(tx);

    tx.set_gas_price(gas_price);
    // send transaction with lower gas price
    let _lower_priced_pending_tx = provider.send_transaction(tx.clone()).await.unwrap();

    tx.set_gas_price(gas_price + 1);
    // send the same transaction with higher gas price
    let higher_priced_pending_tx = provider.send_transaction(tx).await.unwrap();

    let higher_tx_hash = *higher_priced_pending_tx.tx_hash();
    // mine exactly one block
    api.mine_one().await;

    let block = provider.get_block(1.into(), false.into()).await.unwrap().unwrap();

    assert_eq!(block.transactions.len(), 1);
    assert_eq!(BlockTransactions::Hashes(vec![higher_tx_hash]), block.transactions);

    // FIXME: Unable to get receipt despite hotfix in https://github.com/alloy-rs/alloy/pull/614

    // lower priced transaction was replaced
    // let _lower_priced_receipt = lower_priced_pending_tx.get_receipt().await.unwrap();
    // let higher_priced_receipt = higher_priced_pending_tx.get_receipt().await.unwrap();

    // assert_eq!(1, block.transactions.len());
    // assert_eq!(
    //     BlockTransactions::Hashes(vec![higher_priced_receipt.transaction_hash]),
    //     block.transactions
    // );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_reject_too_high_gas_limits() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let gas_limit = api.gas_limit().to::<u128>();
    let amount = handle.genesis_balance().checked_div(U256::from(3u64)).unwrap();

    let tx =
        TransactionRequest::default().to(to).value(amount).from(from).with_gas_limit(gas_limit);

    let mut tx = WithOtherFields::new(tx);

    // send transaction with the exact gas limit
    let pending = provider.send_transaction(tx.clone()).await.unwrap();

    let pending_receipt = pending.get_receipt().await;
    assert!(pending_receipt.is_ok());

    tx.set_gas_limit(gas_limit + 1);

    // send transaction with higher gas limit
    let pending = provider.send_transaction(tx.clone()).await;

    assert!(pending.is_err());
    let err = pending.unwrap_err();
    assert!(err.to_string().contains("gas too high"));

    api.anvil_set_balance(from, U256::MAX).await.unwrap();

    tx.set_gas_limit(gas_limit);
    let pending = provider.send_transaction(tx).await;
    let _ = pending.unwrap();
}

// <https://github.com/foundry-rs/foundry/issues/8094>
#[tokio::test(flavor = "multi_thread")]
async fn can_mine_large_gas_limit() {
    let (_, handle) = spawn(NodeConfig::test().disable_block_gas_limit(true)).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let gas_limit = anvil::DEFAULT_GAS_LIMIT;
    let amount = handle.genesis_balance().checked_div(U256::from(3u64)).unwrap();

    let tx =
        TransactionRequest::default().to(to).value(amount).from(from).with_gas_limit(gas_limit * 3);

    // send transaction with higher gas limit
    let pending = provider.send_transaction(WithOtherFields::new(tx)).await.unwrap();

    let _resp = pending.get_receipt().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_reject_underpriced_replacement() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    // disable auto mining
    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();
    let amount = handle.genesis_balance().checked_div(U256::from(3u64)).unwrap();

    let tx = TransactionRequest::default().to(to).value(amount).from(from).nonce(nonce);

    let mut tx = WithOtherFields::new(tx);

    tx.set_gas_price(gas_price + 1);
    // send transaction with higher gas price
    let higher_priced_pending_tx = provider.send_transaction(tx.clone()).await.unwrap();

    tx.set_gas_price(gas_price);
    // send the same transaction with lower gas price
    let lower_priced_pending_tx = provider.send_transaction(tx).await;

    let replacement_err = lower_priced_pending_tx.unwrap_err();
    assert!(replacement_err.to_string().contains("replacement transaction underpriced"));

    // mine exactly one block
    api.mine_one().await;
    let higher_priced_receipt = higher_priced_pending_tx.get_receipt().await.unwrap();

    // ensure that only the higher priced tx was mined
    let block = provider.get_block(1.into(), false.into()).await.unwrap().unwrap();
    assert_eq!(1, block.transactions.len());
    assert_eq!(
        BlockTransactions::Hashes(vec![higher_priced_receipt.transaction_hash]),
        block.transactions
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_greeter_http() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();

    let signer: EthereumWallet = wallet.clone().into();

    let alloy_provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let alloy_greeter_addr =
        Greeter::deploy_builder(alloy_provider.clone(), "Hello World!".to_string())
            // .legacy() unimplemented! in alloy
            .deploy()
            .await
            .unwrap();

    let alloy_greeter = Greeter::new(alloy_greeter_addr, alloy_provider);

    let greeting = alloy_greeter.greet().call().await.unwrap();

    assert_eq!("Hello World!", greeting._0);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_and_mine_manually() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    // can mine in auto-mine mode
    api.evm_mine(None).await.unwrap();
    // disable auto mine
    api.anvil_set_auto_mine(false).await.unwrap();
    // can mine in manual mode
    api.evm_mine(None).await.unwrap();

    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let from = wallet.address();

    let greeter_builder =
        Greeter::deploy_builder(provider.clone(), "Hello World!".to_string()).from(from);
    let greeter_calldata = greeter_builder.calldata();

    let tx = TransactionRequest::default().from(from).with_input(greeter_calldata.to_owned());

    let tx = WithOtherFields::new(tx);

    let tx = provider.send_transaction(tx).await.unwrap();

    // mine block with tx manually
    api.evm_mine(None).await.unwrap();

    let receipt = tx.get_receipt().await.unwrap();

    let address = receipt.contract_address.unwrap();
    let greeter_contract = Greeter::new(address, provider);
    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting._0);

    let set_greeting = greeter_contract.setGreeting("Another Message".to_string());
    let tx = set_greeting.send().await.unwrap();

    // mine block manually
    api.evm_mine(None).await.unwrap();

    let _tx = tx.get_receipt().await.unwrap();
    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Another Message", greeting._0);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_mine_automatically() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    // disable auto mine
    api.anvil_set_auto_mine(false).await.unwrap();

    let wallet = handle.dev_wallets().next().unwrap();

    let greeter_builder = Greeter::deploy_builder(provider.clone(), "Hello World!".to_string())
        .from(wallet.address());

    let greeter_calldata = greeter_builder.calldata();

    let tx = TransactionRequest::default()
        .from(wallet.address())
        .with_input(greeter_calldata.to_owned());

    let tx = WithOtherFields::new(tx);

    let sent_tx = provider.send_transaction(tx).await.unwrap();

    // re-enable auto mine
    api.anvil_set_auto_mine(true).await.unwrap();

    let receipt = sent_tx.get_receipt().await.unwrap();
    assert_eq!(receipt.block_number, Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_greeter_historic() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();

    let greeter_addr = Greeter::deploy_builder(provider.clone(), "Hello World!".to_string())
        .from(wallet.address())
        .deploy()
        .await
        .unwrap();

    let greeter_contract = Greeter::new(greeter_addr, provider.clone());

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting._0);

    let block_number = provider.get_block_number().await.unwrap();

    let _receipt = greeter_contract
        .setGreeting("Another Message".to_string())
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Another Message", greeting._0);

    // min
    api.mine_one().await;

    // returns previous state
    let greeting =
        greeter_contract.greet().block(BlockId::Number(block_number.into())).call().await.unwrap();
    assert_eq!("Hello World!", greeting._0);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_greeter_ws() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider();

    let wallet = handle.dev_wallets().next().unwrap();

    let greeter_addr = Greeter::deploy_builder(provider.clone(), "Hello World!".to_string())
        .from(wallet.address())
        // .legacy() unimplemented! in alloy
        .deploy()
        .await
        .unwrap();

    let greeter_contract = Greeter::new(greeter_addr, provider.clone());

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting._0);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_get_code() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider();

    let wallet = handle.dev_wallets().next().unwrap();

    let greeter_addr = Greeter::deploy_builder(provider.clone(), "Hello World!".to_string())
        .from(wallet.address())
        .deploy()
        .await
        .unwrap();

    let code = provider.get_code_at(greeter_addr).await.unwrap();
    assert!(!code.as_ref().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_blocktimestamp_works() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let contract = Multicall::deploy(provider.clone()).await.unwrap();

    let timestamp = contract.getCurrentBlockTimestamp().call().await.unwrap().timestamp;

    assert!(timestamp > U256::from(1));

    let latest_block =
        api.block_by_number(alloy_rpc_types::BlockNumberOrTag::Latest).await.unwrap().unwrap();

    let timestamp = contract.getCurrentBlockTimestamp().call().await.unwrap().timestamp;
    assert_eq!(timestamp.to::<u64>(), latest_block.header.timestamp);

    // repeat call same result
    let timestamp = contract.getCurrentBlockTimestamp().call().await.unwrap().timestamp;
    assert_eq!(timestamp.to::<u64>(), latest_block.header.timestamp);

    // mock timestamp
    let next_timestamp = timestamp.to::<u64>() + 1337;
    api.evm_set_next_block_timestamp(next_timestamp).unwrap();

    let timestamp = contract
        .getCurrentBlockTimestamp()
        .block(BlockId::pending())
        .call()
        .await
        .unwrap()
        .timestamp;
    assert_eq!(timestamp, U256::from(next_timestamp));

    // repeat call same result
    let timestamp = contract
        .getCurrentBlockTimestamp()
        .block(BlockId::pending())
        .call()
        .await
        .unwrap()
        .timestamp;
    assert_eq!(timestamp, U256::from(next_timestamp));
}

#[tokio::test(flavor = "multi_thread")]
async fn call_past_state() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let contract_addr =
        SimpleStorage::deploy_builder(provider.clone(), "initial value".to_string())
            .from(wallet.address())
            .deploy()
            .await
            .unwrap();

    let contract = SimpleStorage::new(contract_addr, provider.clone());

    let deployed_block = provider.get_block_number().await.unwrap();

    let value = contract.getValue().call().await.unwrap();
    assert_eq!(value._0, "initial value");

    let gas_price = api.gas_price();
    let set_tx = contract.setValue("hi".to_string()).gas_price(gas_price + 1);

    let _receipt = set_tx.send().await.unwrap().get_receipt().await.unwrap();

    // assert new value
    let value = contract.getValue().call().await.unwrap();
    assert_eq!(value._0, "hi");

    // assert previous value
    let value =
        contract.getValue().block(BlockId::Number(deployed_block.into())).call().await.unwrap();
    assert_eq!(value._0, "initial value");

    let hash = provider
        .get_block(BlockId::Number(1.into()), false.into())
        .await
        .unwrap()
        .unwrap()
        .header
        .hash;
    let value = contract.getValue().block(BlockId::Hash(hash.into())).call().await.unwrap();
    assert_eq!(value._0, "initial value");
}

#[tokio::test(flavor = "multi_thread")]
async fn can_handle_multiple_concurrent_transfers_with_same_nonce() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.ws_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from).await.unwrap();

    // explicitly set the nonce
    let tx = TransactionRequest::default()
        .to(to)
        .value(U256::from(100))
        .from(from)
        .nonce(nonce)
        .with_gas_limit(21000u128);

    let tx = WithOtherFields::new(tx);

    let mut tasks = Vec::new();
    for _ in 0..10 {
        let tx = tx.clone();
        let provider = provider.clone();
        let task = tokio::task::spawn(async move {
            provider.send_transaction(tx).await.unwrap().get_receipt().await
        });
        tasks.push(task);
    }

    // only one succeeded
    let successful_tx =
        join_all(tasks).await.into_iter().filter(|res| res.as_ref().is_ok()).count();
    assert_eq!(successful_tx, 1);

    assert_eq!(provider.get_transaction_count(from).await.unwrap(), 1u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_handle_multiple_concurrent_deploys_with_same_nonce() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let from = wallet.address();
    let nonce = provider.get_transaction_count(from).await.unwrap();

    let mut tasks = Vec::new();

    let greeter = Greeter::deploy_builder(provider.clone(), "Hello World!".to_string());

    let greeter_calldata = greeter.calldata();

    let tx = TransactionRequest::default()
        .from(from)
        .with_input(greeter_calldata.to_owned())
        .nonce(nonce)
        .with_gas_limit(300_000u128);

    let tx = WithOtherFields::new(tx);

    for _ in 0..10 {
        let provider = provider.clone();
        let tx = tx.clone();
        let task = tokio::task::spawn(async move {
            Ok(provider.send_transaction(tx).await?.get_receipt().await.unwrap())
        });
        tasks.push(task);
    }

    // only one succeeded
    let successful_tx =
        join_all(tasks).await.into_iter().filter(|res| res.as_ref().unwrap().is_ok()).count();
    assert_eq!(successful_tx, 1);
    assert_eq!(provider.get_transaction_count(from).await.unwrap(), 1u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_handle_multiple_concurrent_transactions_with_same_nonce() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let from = wallet.address();

    let greeter_contract =
        Greeter::deploy(provider.clone(), "Hello World!".to_string()).await.unwrap();

    let nonce = provider.get_transaction_count(from).await.unwrap();

    let mut tasks = Vec::new();

    let deploy = Greeter::deploy_builder(provider.clone(), "Hello World!".to_string());
    let deploy_calldata = deploy.calldata();
    let deploy_tx = TransactionRequest::default()
        .from(from)
        .with_input(deploy_calldata.to_owned())
        .nonce(nonce)
        .with_gas_limit(300_000u128);
    let deploy_tx = WithOtherFields::new(deploy_tx);

    let set_greeting = greeter_contract.setGreeting("Hello".to_string());
    let set_greeting_calldata = set_greeting.calldata();

    let set_greeting_tx = TransactionRequest::default()
        .from(from)
        .with_input(set_greeting_calldata.to_owned())
        .nonce(nonce)
        .with_gas_limit(300_000u128);
    let set_greeting_tx = WithOtherFields::new(set_greeting_tx);

    for idx in 0..10 {
        let provider = provider.clone();
        let task = if idx % 2 == 0 {
            let tx = deploy_tx.clone();
            tokio::task::spawn(async move {
                Ok(provider.send_transaction(tx).await?.get_receipt().await.unwrap())
            })
        } else {
            let tx = set_greeting_tx.clone();
            tokio::task::spawn(async move {
                Ok(provider.send_transaction(tx).await?.get_receipt().await.unwrap())
            })
        };

        tasks.push(task);
    }

    // only one succeeded
    let successful_tx =
        join_all(tasks).await.into_iter().filter(|res| res.as_ref().unwrap().is_ok()).count();
    assert_eq!(successful_tx, 1);
    assert_eq!(provider.get_transaction_count(from).await.unwrap(), nonce + 1);
}
#[tokio::test(flavor = "multi_thread")]
async fn can_get_pending_transaction() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    // disable auto mining so we can check if we can return pending tx from the mempool
    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();

    let from = handle.dev_wallets().next().unwrap().address();
    let tx = TransactionRequest::default().from(from).value(U256::from(1337)).to(Address::random());
    let tx = WithOtherFields::new(tx);
    let tx = provider.send_transaction(tx).await.unwrap();

    let pending = provider.get_transaction_by_hash(*tx.tx_hash()).await;
    assert!(pending.is_ok());

    api.mine_one().await;
    let mined = provider.get_transaction_by_hash(*tx.tx_hash()).await.unwrap().unwrap();

    assert_eq!(mined.hash, pending.unwrap().unwrap().hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_raw_transaction() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    // first test the pending tx, disable auto mine
    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();

    let from = handle.dev_wallets().next().unwrap().address();
    let tx = TransactionRequest::default().from(from).value(U256::from(1488)).to(Address::random());
    let tx = WithOtherFields::new(tx);
    let tx = provider.send_transaction(tx).await.unwrap();

    let res1 = api.raw_transaction(*tx.tx_hash()).await;
    assert!(res1.is_ok());

    api.mine_one().await;
    let res2 = api.raw_transaction(*tx.tx_hash()).await;

    assert_eq!(res1.unwrap(), res2.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_first_nonce_is_zero() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();

    let nonce = provider.get_transaction_count(from).block_id(BlockId::pending()).await.unwrap();

    assert_eq!(nonce, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_handle_different_sender_nonce_calculation() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();
    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from_first = accounts[0].address();
    let from_second = accounts[1].address();

    let tx_count = 10u64;

    // send a bunch of tx to the mempool and check nonce is returned correctly
    for idx in 1..=tx_count {
        let tx_from_first = TransactionRequest::default()
            .from(from_first)
            .value(U256::from(1337u64))
            .to(Address::random());
        let tx_from_first = WithOtherFields::new(tx_from_first);
        let _tx = provider.send_transaction(tx_from_first).await.unwrap();
        let nonce_from_first =
            provider.get_transaction_count(from_first).block_id(BlockId::pending()).await.unwrap();
        assert_eq!(nonce_from_first, idx);

        let tx_from_second = TransactionRequest::default()
            .from(from_second)
            .value(U256::from(1337u64))
            .to(Address::random());
        let tx_from_second = WithOtherFields::new(tx_from_second);
        let _tx = provider.send_transaction(tx_from_second).await.unwrap();
        let nonce_from_second =
            provider.get_transaction_count(from_second).block_id(BlockId::pending()).await.unwrap();
        assert_eq!(nonce_from_second, idx);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn includes_pending_tx_for_transaction_count() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();
    let from = handle.dev_wallets().next().unwrap().address();

    let tx_count = 10u64;

    // send a bunch of tx to the mempool and check nonce is returned correctly
    for idx in 1..=tx_count {
        let tx =
            TransactionRequest::default().from(from).value(U256::from(1337)).to(Address::random());
        let tx = WithOtherFields::new(tx);
        let _tx = provider.send_transaction(tx).await.unwrap();
        let nonce =
            provider.get_transaction_count(from).block_id(BlockId::pending()).await.unwrap();
        assert_eq!(nonce, idx);
    }

    api.mine_one().await;
    let nonce = provider.get_transaction_count(from).block_id(BlockId::pending()).await.unwrap();
    assert_eq!(nonce, tx_count);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_historic_info() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();
    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);
    let tx = provider.send_transaction(tx).await.unwrap();
    let _ = tx.get_receipt().await.unwrap();

    let nonce_pre =
        provider.get_transaction_count(from).block_id(BlockId::number(0)).await.unwrap();

    let nonce_post = provider.get_transaction_count(from).await.unwrap();

    assert!(nonce_pre < nonce_post);

    let balance_pre = provider.get_balance(from).block_id(BlockId::number(0)).await.unwrap();

    let balance_post = provider.get_balance(from).await.unwrap();

    assert!(balance_post < balance_pre);

    let to_balance = provider.get_balance(to).await.unwrap();
    assert_eq!(balance_pre.saturating_add(amount), to_balance);
}

// <https://github.com/eth-brownie/brownie/issues/1549>
#[tokio::test(flavor = "multi_thread")]
async fn test_tx_receipt() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let provider = handle.http_provider();

    let tx = TransactionRequest::default().to(Address::random()).value(U256::from(1337));

    let tx = WithOtherFields::new(tx);
    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    assert!(tx.to.is_some());

    let greeter_deploy = Greeter::deploy_builder(provider.clone(), "Hello World!".to_string());
    let greeter_calldata = greeter_deploy.calldata();

    let tx = TransactionRequest::default()
        .from(wallet.address())
        .with_input(greeter_calldata.to_owned());

    let tx = WithOtherFields::new(tx);
    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    // `to` field is none if it's a contract creation transaction: https://ethereum.org/developers/docs/apis/json-rpc/#eth_gettransactionreceipt
    assert!(tx.to.is_none());
    assert!(tx.contract_address.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_stream_pending_transactions() {
    let (_api, handle) =
        spawn(NodeConfig::test().with_blocktime(Some(Duration::from_secs(2)))).await;
    let num_txs = 5;

    let provider = handle.http_provider();
    let ws_provider = connect_pubsub(&handle.ws_endpoint()).await;

    let accounts = provider.get_accounts().await.unwrap();
    let tx =
        TransactionRequest::default().from(accounts[0]).to(accounts[0]).value(U256::from(1e18));

    let mut sending = futures::future::join_all(
        std::iter::repeat(tx.clone())
            .take(num_txs)
            .enumerate()
            .map(|(nonce, tx)| tx.nonce(nonce as u64))
            .map(|tx| async {
                let tx = WithOtherFields::new(tx);
                provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap()
            }),
    )
    .fuse();

    let mut watch_tx_stream = provider
        .watch_pending_transactions()
        .await
        .unwrap()
        .into_stream()
        .flat_map(futures::stream::iter)
        .take(num_txs)
        .fuse();

    let mut sub_tx_stream = ws_provider
        .subscribe_pending_transactions()
        .await
        .unwrap()
        .into_stream()
        .take(num_txs)
        .fuse();

    let mut sent = None;
    let mut watch_received = Vec::with_capacity(num_txs);
    let mut sub_received = Vec::with_capacity(num_txs);

    loop {
        futures::select! {
            txs = sending => {
                sent = Some(txs)
            },
            tx = watch_tx_stream.next() => {
                if let Some(tx) = tx {
                    watch_received.push(tx);
                }
            },
            tx = sub_tx_stream.next() => {
                if let Some(tx) = tx {
                    sub_received.push(tx);
                }
            },
            complete => unreachable!(),
        };

        if watch_received.len() == num_txs && sub_received.len() == num_txs {
            if let Some(sent) = &sent {
                assert_eq!(sent.len(), watch_received.len());
                let sent_txs = sent.iter().map(|tx| tx.transaction_hash).collect::<HashSet<_>>();
                assert_eq!(sent_txs, watch_received.iter().copied().collect());
                assert_eq!(sent_txs, sub_received.iter().copied().collect());
                break
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tx_access_list() {
    /// returns a String representation of the AccessList, with sorted
    /// keys (address) and storage slots
    fn access_list_to_sorted_string(a: AccessList) -> String {
        let mut a = a.0;
        a.sort_by_key(|v| v.address);

        let a = a
            .iter_mut()
            .map(|v| {
                v.storage_keys.sort();
                (v.address, std::mem::take(&mut v.storage_keys))
            })
            .collect::<Vec<_>>();

        format!("{a:?}")
    }

    /// asserts that the two access lists are equal, by comparing their sorted
    /// string representation
    fn assert_access_list_eq(a: AccessList, b: AccessList) {
        assert_eq!(access_list_to_sorted_string(a), access_list_to_sorted_string(b))
    }

    // We want to test a couple of things:
    //     - When calling a contract with no storage read/write, it shouldn't be in the AL
    //     - When a contract calls a contract, the latter one should be in the AL
    //     - No precompiles should be in the AL
    //     - The sender shouldn't be in the AL
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.http_provider();

    let sender = Address::random();
    let other_acc = Address::random();
    let multicall = Multicall::deploy(provider.clone()).await.unwrap();
    let simple_storage = SimpleStorage::deploy(provider.clone(), "foo".to_string()).await.unwrap();

    // when calling `setValue` on SimpleStorage, both the `lastSender` and `_value` storages are
    // modified The `_value` is a `string`, so the storage slots here (small string) are `0x1`
    // and `keccak(0x1)`
    let set_value = simple_storage.setValue("bar".to_string());
    let set_value_calldata = set_value.calldata();
    let set_value_tx = TransactionRequest::default()
        .from(sender)
        .to(*simple_storage.address())
        .with_input(set_value_calldata.to_owned());
    let set_value_tx = WithOtherFields::new(set_value_tx);
    let access_list = provider.create_access_list(&set_value_tx).await.unwrap();
    // let set_value_tx = simple_storage.set_value("bar".to_string()).from(sender).tx;
    // let access_list = client.create_access_list(&set_value_tx, None).await.unwrap();
    assert_access_list_eq(
        access_list.access_list,
        AccessList::from(vec![AccessListItem {
            address: *simple_storage.address(),
            storage_keys: vec![
                FixedBytes::ZERO,
                FixedBytes::with_last_byte(1),
                FixedBytes::from_str(
                    "0xb10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6",
                )
                .unwrap(),
            ],
        }]),
    );

    // With a subcall that fetches the balances of an account (`other_acc`), only the address
    // of this account should be in the Access List
    let call_tx = multicall.getEthBalance(other_acc);
    let call_tx_data = call_tx.calldata();
    let call_tx = TransactionRequest::default()
        .from(sender)
        .to(*multicall.address())
        .with_input(call_tx_data.to_owned());
    let call_tx = WithOtherFields::new(call_tx);
    let access_list = provider.create_access_list(&call_tx).await.unwrap();
    assert_access_list_eq(
        access_list.access_list,
        AccessList::from(vec![AccessListItem { address: other_acc, storage_keys: vec![] }]),
    );

    // With a subcall to another contract, the AccessList should be the same as when calling the
    // subcontract directly (given that the proxy contract doesn't read/write any state)
    let subcall_tx = multicall.aggregate(vec![Multicall::Call {
        target: *simple_storage.address(),
        callData: set_value_calldata.to_owned(),
    }]);

    let subcall_tx_calldata = subcall_tx.calldata();

    let subcall_tx = TransactionRequest::default()
        .from(sender)
        .to(*multicall.address())
        .with_input(subcall_tx_calldata.to_owned());
    let subcall_tx = WithOtherFields::new(subcall_tx);
    let access_list = provider.create_access_list(&subcall_tx).await.unwrap();
    assert_access_list_eq(
        access_list.access_list,
        // H256::from_uint(&(1u64.into())),
        AccessList::from(vec![AccessListItem {
            address: *simple_storage.address(),
            storage_keys: vec![
                FixedBytes::ZERO,
                FixedBytes::with_last_byte(1),
                FixedBytes::from_str(
                    "0xb10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6",
                )
                .unwrap(),
            ],
        }]),
    );
}

// ensures that the gas estimate is running on pending block by default
#[tokio::test(flavor = "multi_thread")]
async fn estimates_gas_on_pending_by_default() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    // disable auto mine
    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let recipient = Address::random();

    let tx = TransactionRequest::default().from(sender).to(recipient).value(U256::from(1e18));
    let tx = WithOtherFields::new(tx);

    let _pending = provider.send_transaction(tx).await.unwrap();

    let tx = TransactionRequest::default()
        .from(recipient)
        .to(sender)
        .value(U256::from(1e10))
        .input(Bytes::from(vec![0x42]).into());
    api.estimate_gas(WithOtherFields::new(tx), None, None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_gas() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let recipient = Address::random();

    let tx = TransactionRequest::default()
        .from(recipient)
        .to(sender)
        .value(U256::from(1e10))
        .input(Bytes::from(vec![0x42]).into());
    // Expect the gas estimation to fail due to insufficient funds.
    let error_result = api.estimate_gas(WithOtherFields::new(tx.clone()), None, None).await;

    assert!(error_result.is_err(), "Expected an error due to insufficient funds");
    let error_message = error_result.unwrap_err().to_string();
    assert!(
        error_message.contains("Insufficient funds for gas * price + value"),
        "Error message did not match expected: {error_message}"
    );

    // Setup state override to simulate sufficient funds for the recipient.
    let addr = recipient;
    let account_override =
        AccountOverride { balance: Some(alloy_primitives::U256::from(1e18)), ..Default::default() };
    let mut state_override = StateOverride::new();
    state_override.insert(addr, account_override);

    // Estimate gas with state override implying sufficient funds.
    let gas_estimate = api
        .estimate_gas(WithOtherFields::new(tx), None, Some(state_override))
        .await
        .expect("Failed to estimate gas with state override");

    // Assert the gas estimate meets the expected minimum.
    assert!(gas_estimate >= U256::from(21000), "Gas estimate is lower than expected minimum");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reject_gas_too_low() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let account = handle.dev_accounts().next().unwrap();

    let gas = 21_000u64 - 1;
    let tx = TransactionRequest::default()
        .to(Address::random())
        .value(U256::from(1337u64))
        .from(account)
        .with_gas_limit(gas as u128);
    let tx = WithOtherFields::new(tx);

    let resp = provider.send_transaction(tx).await;

    let err = resp.unwrap_err().to_string();
    assert!(err.contains("intrinsic gas too low"));
}

// <https://github.com/foundry-rs/foundry/issues/3783>
#[tokio::test(flavor = "multi_thread")]
async fn can_call_with_high_gas_limit() {
    let (_api, handle) = spawn(NodeConfig::test().with_gas_limit(Some(100_000_000))).await;
    let provider = handle.http_provider();

    let greeter_contract = Greeter::deploy(provider, "Hello World!".to_string()).await.unwrap();

    let greeting = greeter_contract.greet().gas(60_000_000u128).call().await.unwrap();
    assert_eq!("Hello World!", greeting._0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reject_eip1559_pre_london() {
    let (api, handle) =
        spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Berlin.into()))).await;
    let provider = handle.http_provider();

    let gas_limit = api.gas_limit().to::<u128>();
    let gas_price = api.gas_price();

    let unsupported_call_builder =
        Greeter::deploy_builder(provider.clone(), "Hello World!".to_string());
    let unsupported_calldata = unsupported_call_builder.calldata();

    let unsup_tx = TransactionRequest::default()
        .from(handle.dev_accounts().next().unwrap())
        .with_input(unsupported_calldata.to_owned())
        .with_gas_limit(gas_limit)
        .with_max_fee_per_gas(gas_price)
        .with_max_priority_fee_per_gas(gas_price);

    let unsup_tx = WithOtherFields::new(unsup_tx);

    let unsupported = provider.send_transaction(unsup_tx).await.unwrap_err().to_string();
    assert!(unsupported.contains("not supported by the current hardfork"), "{unsupported}");

    let greeter_contract_addr =
        Greeter::deploy_builder(provider.clone(), "Hello World!".to_string())
            .gas(gas_limit)
            .gas_price(gas_price)
            .deploy()
            .await
            .unwrap();

    let greeter_contract = Greeter::new(greeter_contract_addr, provider);

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting._0);
}

// https://github.com/foundry-rs/foundry/issues/6931
#[tokio::test(flavor = "multi_thread")]
async fn can_mine_multiple_in_block() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    // disable auto mine
    api.anvil_set_auto_mine(false).await.unwrap();

    let tx = TransactionRequest {
        from: Some("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".parse().unwrap()),
        ..Default::default()
    };

    // broadcast it via the eth_sendTransaction API
    let first = api.send_transaction(WithOtherFields::new(tx.clone())).await.unwrap();
    let second = api.send_transaction(WithOtherFields::new(tx.clone())).await.unwrap();

    api.anvil_mine(Some(U256::from(1)), Some(U256::ZERO)).await.unwrap();

    let block = api.block_by_number(BlockNumberOrTag::Latest).await.unwrap().unwrap();

    let txs = block.transactions.hashes().collect::<Vec<_>>();
    assert_eq!(txs, vec![first, second]);
}
