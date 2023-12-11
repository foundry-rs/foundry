use crate::abi::*;
use anvil::{spawn, Hardfork, NodeConfig};
use ethers::{
    abi::ethereum_types::BigEndianHash,
    prelude::{
        signer::SignerMiddlewareError, BlockId, Middleware, Signer, SignerMiddleware,
        TransactionRequest,
    },
    types::{
        transaction::eip2930::{AccessList, AccessListItem},
        Address, BlockNumber, Transaction, TransactionReceipt, H256, U256,
    },
};
use foundry_common::types::{to_call_request_from_tx_request, ToAlloy, ToEthers};
use futures::{future::join_all, FutureExt, StreamExt};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread")]
async fn can_transfer_eth() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from, None).await.unwrap();
    assert!(nonce.is_zero());

    let balance_before = provider.get_balance(to, None).await.unwrap();

    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();

    // craft the tx
    // specify the `from` field so that the client knows which account to use
    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    // broadcast it via the eth_sendTransaction API
    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    assert_eq!(tx.block_number, Some(1u64.into()));
    assert_eq!(tx.transaction_index, 0u64.into());

    let nonce = provider.get_transaction_count(from, None).await.unwrap();

    assert_eq!(nonce, 1u64.into());

    let to_balance = provider.get_balance(to, None).await.unwrap();

    assert_eq!(balance_before.saturating_add(amount), to_balance);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_order_transactions() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    // disable automine
    api.anvil_set_auto_mine(false).await.unwrap();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();

    let gas_price = provider.get_gas_price().await.unwrap();

    // craft the tx with lower price
    let tx = TransactionRequest::new().to(to).from(from).value(amount).gas_price(gas_price);
    let tx_lower = provider.send_transaction(tx, None).await.unwrap();

    // craft the tx with higher price
    let tx = TransactionRequest::new().to(from).from(to).value(amount).gas_price(gas_price + 1);
    let tx_higher = provider.send_transaction(tx, None).await.unwrap();

    // manually mine the block with the transactions
    api.mine_one().await;

    // get the block, await receipts
    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    let lower_price = tx_lower.await.unwrap().unwrap().transaction_hash;
    let higher_price = tx_higher.await.unwrap().unwrap().transaction_hash;
    assert_eq!(block.transactions, vec![higher_price, lower_price])
}

#[tokio::test(flavor = "multi_thread")]
async fn can_respect_nonces() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from, None).await.unwrap();
    let amount = handle.genesis_balance().checked_div(3u64.into()).unwrap();

    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    // send the transaction with higher nonce than on chain
    let higher_pending_tx =
        provider.send_transaction(tx.clone().nonce(nonce + 1u64), None).await.unwrap();

    // ensure the listener for ready transactions times out
    let mut listener = api.new_ready_transactions();
    let res = timeout(Duration::from_millis(1500), listener.next()).await;
    res.unwrap_err();

    // send with the actual nonce which is mined immediately
    let tx =
        provider.send_transaction(tx.nonce(nonce), None).await.unwrap().await.unwrap().unwrap();

    // this will unblock the currently pending tx
    let higher_tx = higher_pending_tx.await.unwrap().unwrap();

    let block = provider.get_block(1u64).await.unwrap().unwrap();
    assert_eq!(2, block.transactions.len());
    assert_eq!(vec![tx.transaction_hash, higher_tx.transaction_hash], block.transactions);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_replace_transaction() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    // disable auto mining
    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.ethers_http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from, None).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();
    let amount = handle.genesis_balance().checked_div(3u64.into()).unwrap();

    let tx = TransactionRequest::new().to(to).value(amount).from(from).nonce(nonce);

    // send transaction with lower gas price
    let lower_priced_pending_tx =
        provider.send_transaction(tx.clone().gas_price(gas_price), None).await.unwrap();

    // send the same transaction with higher gas price
    let higher_priced_pending_tx =
        provider.send_transaction(tx.gas_price(gas_price + 1u64), None).await.unwrap();

    // mine exactly one block
    api.mine_one().await;

    // lower priced transaction was replaced
    let lower_priced_receipt = lower_priced_pending_tx.await.unwrap();
    assert!(lower_priced_receipt.is_none());

    let higher_priced_receipt = higher_priced_pending_tx.await.unwrap().unwrap();

    // ensure that only the replacement tx was mined
    let block = provider.get_block(1u64).await.unwrap().unwrap();
    assert_eq!(1, block.transactions.len());
    assert_eq!(vec![higher_priced_receipt.transaction_hash], block.transactions);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_reject_too_high_gas_limits() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let gas_limit = api.gas_limit();
    let amount = handle.genesis_balance().checked_div(3u64.into()).unwrap();

    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    // send transaction with the exact gas limit
    let pending = provider.send_transaction(tx.clone().gas(gas_limit.to_ethers()), None).await;

    pending.unwrap();

    // send transaction with higher gas limit
    let pending =
        provider.send_transaction(tx.clone().gas(gas_limit.to_ethers() + 1u64), None).await;

    assert!(pending.is_err());
    let err = pending.unwrap_err();
    assert!(err.to_string().contains("gas too high"));

    api.anvil_set_balance(from.to_alloy(), U256::MAX.to_alloy()).await.unwrap();

    let pending = provider.send_transaction(tx.gas(gas_limit.to_ethers()), None).await;
    pending.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_reject_underpriced_replacement() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    // disable auto mining
    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.ethers_http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from, None).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();
    let amount = handle.genesis_balance().checked_div(3u64.into()).unwrap();

    let tx = TransactionRequest::new().to(to).value(amount).from(from).nonce(nonce);

    // send transaction with higher gas price
    let higher_priced_pending_tx =
        provider.send_transaction(tx.clone().gas_price(gas_price + 1u64), None).await.unwrap();

    // send the same transaction with lower gas price
    let lower_priced_pending_tx = provider.send_transaction(tx.gas_price(gas_price), None).await;

    let replacement_err = lower_priced_pending_tx.unwrap_err();
    assert!(replacement_err.to_string().contains("replacement transaction underpriced"));

    // mine exactly one block
    api.mine_one().await;
    let higher_priced_receipt = higher_priced_pending_tx.await.unwrap().unwrap();

    // ensure that only the higher priced tx was mined
    let block = provider.get_block(1u64).await.unwrap().unwrap();
    assert_eq!(1, block.transactions.len());
    assert_eq!(vec![higher_priced_receipt.transaction_hash], block.transactions);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_greeter_http() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string())
        .unwrap()
        .legacy()
        .send()
        .await
        .unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);

    let greeter_contract =
        Greeter::deploy(client, "Hello World!".to_string()).unwrap().send().await.unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);
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

    let provider = handle.ethers_http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let tx = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string()).unwrap().deployer.tx;

    let tx = client.send_transaction(tx, None).await.unwrap();

    // mine block with tx manually
    api.evm_mine(None).await.unwrap();

    let receipt = tx.await.unwrap().unwrap();

    let address = receipt.contract_address.unwrap();
    let greeter_contract = Greeter::new(address, Arc::clone(&client));
    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);

    let set_greeting = greeter_contract.set_greeting("Another Message".to_string());
    let tx = set_greeting.send().await.unwrap();

    // mine block manually
    api.evm_mine(None).await.unwrap();

    let _tx = tx.await.unwrap();
    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Another Message", greeting);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_mine_automatically() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    // disable auto mine
    api.anvil_set_auto_mine(false).await.unwrap();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let tx = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string()).unwrap().deployer.tx;
    let sent_tx = client.send_transaction(tx, None).await.unwrap();

    // re-enable auto mine
    api.anvil_set_auto_mine(true).await.unwrap();

    let receipt = sent_tx.await.unwrap().unwrap();
    assert_eq!(receipt.status.unwrap().as_u64(), 1u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_greeter_historic() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string())
        .unwrap()
        .send()
        .await
        .unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);

    let block = client.get_block_number().await.unwrap();

    greeter_contract
        .set_greeting("Another Message".to_string())
        .send()
        .await
        .unwrap()
        .await
        .unwrap();
    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Another Message", greeting);

    // returns previous state
    let greeting =
        greeter_contract.greet().block(BlockId::Number(block.into())).call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_greeter_ws() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_ws_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string())
        .unwrap()
        .legacy()
        .send()
        .await
        .unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);

    let greeter_contract =
        Greeter::deploy(client, "Hello World!".to_string()).unwrap().send().await.unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_get_code() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_ws_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string())
        .unwrap()
        .legacy()
        .send()
        .await
        .unwrap();

    let code = client.get_code(greeter_contract.address(), None).await.unwrap();
    assert!(!code.as_ref().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn get_blocktimestamp_works() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let contract =
        MulticallContract::deploy(Arc::clone(&client), ()).unwrap().send().await.unwrap();

    let timestamp = contract.get_current_block_timestamp().call().await.unwrap();

    assert!(timestamp > U256::one());

    let latest_block =
        api.block_by_number(alloy_rpc_types::BlockNumberOrTag::Latest).await.unwrap().unwrap();

    let timestamp = contract.get_current_block_timestamp().call().await.unwrap();
    assert_eq!(timestamp, latest_block.header.timestamp.to_ethers());

    // repeat call same result
    let timestamp = contract.get_current_block_timestamp().call().await.unwrap();
    assert_eq!(timestamp, latest_block.header.timestamp.to_ethers());

    // mock timestamp
    let next_timestamp = timestamp.as_u64() + 1337;
    api.evm_set_next_block_timestamp(next_timestamp).unwrap();

    let timestamp =
        contract.get_current_block_timestamp().block(BlockNumber::Pending).call().await.unwrap();
    assert_eq!(timestamp, next_timestamp.into());

    // repeat call same result
    let timestamp =
        contract.get_current_block_timestamp().block(BlockNumber::Pending).call().await.unwrap();
    assert_eq!(timestamp, next_timestamp.into());
}

#[tokio::test(flavor = "multi_thread")]
async fn call_past_state() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let contract = SimpleStorage::deploy(Arc::clone(&client), "initial value".to_string())
        .unwrap()
        .send()
        .await
        .unwrap();

    let deployed_block = client.get_block_number().await.unwrap();

    // assert initial state
    let value = contract.method::<_, String>("getValue", ()).unwrap().call().await.unwrap();
    assert_eq!(value, "initial value");

    // make a call with `client`
    let _tx_hash = contract
        .method::<_, H256>("setValue", "hi".to_owned())
        .unwrap()
        .send()
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    // assert new value
    let value = contract.method::<_, String>("getValue", ()).unwrap().call().await.unwrap();
    assert_eq!(value, "hi");

    // assert previous value
    let value = contract
        .method::<_, String>("getValue", ())
        .unwrap()
        .block(BlockId::Number(deployed_block.into()))
        .call()
        .await
        .unwrap();
    assert_eq!(value, "initial value");

    let hash = client.get_block(1).await.unwrap().unwrap().hash.unwrap();
    let value = contract
        .method::<_, String>("getValue", ())
        .unwrap()
        .block(BlockId::Hash(hash))
        .call()
        .await
        .unwrap();
    assert_eq!(value, "initial value");
}

#[tokio::test(flavor = "multi_thread")]
async fn can_handle_multiple_concurrent_transfers_with_same_nonce() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.ethers_ws_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let nonce = provider.get_transaction_count(from, None).await.unwrap();

    // explicitly set the nonce
    let tx = TransactionRequest::new().to(to).value(100u64).from(from).nonce(nonce).gas(21_000u64);
    let mut tasks = Vec::new();
    for _ in 0..10 {
        let provider = provider.clone();
        let tx = tx.clone();
        let task =
            tokio::task::spawn(async move { provider.send_transaction(tx, None).await?.await });
        tasks.push(task);
    }

    // only one succeeded
    let successful_tx =
        join_all(tasks).await.into_iter().filter(|res| res.as_ref().unwrap().is_ok()).count();
    assert_eq!(successful_tx, 1);

    assert_eq!(provider.get_transaction_count(from, None).await.unwrap(), 1u64.into());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_handle_multiple_concurrent_deploys_with_same_nonce() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_ws_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let from = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));
    let nonce = client.get_transaction_count(from, None).await.unwrap();
    // explicitly set the nonce
    let mut tasks = Vec::new();
    let mut tx =
        Greeter::deploy(Arc::clone(&client), "Hello World!".to_string()).unwrap().deployer.tx;
    tx.set_nonce(nonce);
    tx.set_gas(300_000u64);

    for _ in 0..10 {
        let client = Arc::clone(&client);
        let tx = tx.clone();
        let task = tokio::task::spawn(async move {
            Ok::<_, SignerMiddlewareError<_, _>>(
                client.send_transaction(tx, None).await?.await.unwrap(),
            )
        });
        tasks.push(task);
    }

    // only one succeeded
    let successful_tx =
        join_all(tasks).await.into_iter().filter(|res| res.as_ref().unwrap().is_ok()).count();
    assert_eq!(successful_tx, 1);
    assert_eq!(client.get_transaction_count(from, None).await.unwrap(), 1u64.into());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_handle_multiple_concurrent_transactions_with_same_nonce() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_ws_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let from = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string())
        .unwrap()
        .send()
        .await
        .unwrap();

    let nonce = client.get_transaction_count(from, None).await.unwrap();
    // explicitly set the nonce
    let mut tasks = Vec::new();
    let mut deploy_tx =
        Greeter::deploy(Arc::clone(&client), "Hello World!".to_string()).unwrap().deployer.tx;
    deploy_tx.set_nonce(nonce);
    deploy_tx.set_gas(300_000u64);

    let mut set_greeting_tx = greeter_contract.set_greeting("Hello".to_string()).tx;
    set_greeting_tx.set_nonce(nonce);
    set_greeting_tx.set_gas(300_000u64);

    for idx in 0..10 {
        let client = Arc::clone(&client);
        let task = if idx % 2 == 0 {
            let tx = deploy_tx.clone();
            tokio::task::spawn(async move {
                Ok::<_, SignerMiddlewareError<_, _>>(
                    client.send_transaction(tx, None).await?.await.unwrap(),
                )
            })
        } else {
            let tx = set_greeting_tx.clone();
            tokio::task::spawn(async move {
                Ok::<_, SignerMiddlewareError<_, _>>(
                    client.send_transaction(tx, None).await?.await.unwrap(),
                )
            })
        };

        tasks.push(task);
    }

    // only one succeeded
    let successful_tx =
        join_all(tasks).await.into_iter().filter(|res| res.as_ref().unwrap().is_ok()).count();
    assert_eq!(successful_tx, 1);
    assert_eq!(client.get_transaction_count(from, None).await.unwrap(), nonce + 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_pending_transaction() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    // disable auto mining so we can check if we can return pending tx from the mempool
    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.ethers_http_provider();

    let from = handle.dev_wallets().next().unwrap().address();
    let tx = TransactionRequest::new().from(from).value(1337u64).to(Address::random());
    let tx = provider.send_transaction(tx, None).await.unwrap();

    let pending = provider.get_transaction(tx.tx_hash()).await.unwrap();
    assert!(pending.is_some());

    api.mine_one().await;
    let mined = provider.get_transaction(tx.tx_hash()).await.unwrap().unwrap();

    assert_eq!(mined.hash, pending.unwrap().hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_first_noce_is_zero() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.ethers_http_provider();
    let from = handle.dev_wallets().next().unwrap().address();

    let nonce = provider
        .get_transaction_count(from, Some(BlockId::Number(BlockNumber::Pending)))
        .await
        .unwrap();

    assert_eq!(nonce, U256::zero());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_handle_different_sender_nonce_calculation() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.ethers_http_provider();
    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from_first = accounts[0].address();
    let from_second = accounts[1].address();

    let tx_count = 10u64;

    // send a bunch of tx to the mempool and check nonce is returned correctly
    for idx in 1..=tx_count {
        let tx_from_first =
            TransactionRequest::new().from(from_first).value(1337u64).to(Address::random());
        let _tx = provider.send_transaction(tx_from_first, None).await.unwrap();
        let nonce_from_first = provider
            .get_transaction_count(from_first, Some(BlockId::Number(BlockNumber::Pending)))
            .await
            .unwrap();
        assert_eq!(nonce_from_first, idx.into());

        let tx_from_second =
            TransactionRequest::new().from(from_second).value(1337u64).to(Address::random());
        let _tx = provider.send_transaction(tx_from_second, None).await.unwrap();
        let nonce_from_second = provider
            .get_transaction_count(from_second, Some(BlockId::Number(BlockNumber::Pending)))
            .await
            .unwrap();
        assert_eq!(nonce_from_second, idx.into());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn includes_pending_tx_for_transaction_count() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.ethers_http_provider();
    let from = handle.dev_wallets().next().unwrap().address();

    let tx_count = 10u64;

    // send a bunch of tx to the mempool and check nonce is returned correctly
    for idx in 1..=tx_count {
        let tx = TransactionRequest::new().from(from).value(1337u64).to(Address::random());
        let _tx = provider.send_transaction(tx, None).await.unwrap();
        let nonce = provider
            .get_transaction_count(from, Some(BlockId::Number(BlockNumber::Pending)))
            .await
            .unwrap();
        assert_eq!(nonce, idx.into());
    }

    api.mine_one().await;
    let nonce = provider
        .get_transaction_count(from, Some(BlockId::Number(BlockNumber::Pending)))
        .await
        .unwrap();
    assert_eq!(nonce, tx_count.into());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_historic_info() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();
    let tx = TransactionRequest::new().to(to).value(amount).from(from);
    let _tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let nonce_pre = provider
        .get_transaction_count(from, Some(BlockNumber::Number(0.into()).into()))
        .await
        .unwrap();

    let nonce_post =
        provider.get_transaction_count(from, Some(BlockNumber::Latest.into())).await.unwrap();

    assert!(nonce_pre < nonce_post);

    let balance_pre =
        provider.get_balance(from, Some(BlockNumber::Number(0.into()).into())).await.unwrap();

    let balance_post = provider.get_balance(from, Some(BlockNumber::Latest.into())).await.unwrap();

    assert!(balance_post < balance_pre);

    let to_balance = provider.get_balance(to, None).await.unwrap();
    assert_eq!(balance_pre.saturating_add(amount), to_balance);
}

// <https://github.com/eth-brownie/brownie/issues/1549>
#[tokio::test(flavor = "multi_thread")]
async fn test_tx_receipt() {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(handle.ethers_http_provider(), wallet));

    let tx = TransactionRequest::new().to(Address::random()).value(1337u64);

    let tx = client.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
    assert!(tx.to.is_some());

    let tx = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string()).unwrap().deployer.tx;

    let tx = client.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    // `to` field is none if it's a contract creation transaction: https://eth.wiki/json-rpc/API#eth_getTransactionReceipt
    assert!(tx.to.is_none());
    assert!(tx.contract_address.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_stream_pending_transactions() {
    let (_api, handle) =
        spawn(NodeConfig::test().with_blocktime(Some(Duration::from_secs(2)))).await;
    let num_txs = 5;
    let provider = handle.ethers_http_provider();
    let ws_provider = handle.ethers_ws_provider();

    let accounts = provider.get_accounts().await.unwrap();
    let tx = TransactionRequest::new().from(accounts[0]).to(accounts[0]).value(1e18 as u64);

    let mut sending = futures::future::join_all(
        std::iter::repeat(tx.clone())
            .take(num_txs)
            .enumerate()
            .map(|(nonce, tx)| tx.nonce(nonce))
            .map(|tx| async {
                provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap()
            }),
    )
    .fuse();

    let mut watch_tx_stream =
        provider.watch_pending_transactions().await.unwrap().transactions_unordered(num_txs).fuse();

    let mut sub_tx_stream =
        ws_provider.subscribe_pending_txs().await.unwrap().transactions_unordered(2).fuse();

    let mut sent: Option<Vec<TransactionReceipt>> = None;
    let mut watch_received: Vec<Transaction> = Vec::with_capacity(num_txs);
    let mut sub_received: Vec<Transaction> = Vec::with_capacity(num_txs);

    loop {
        futures::select! {
            txs = sending => {
                sent = Some(txs)
            },
            tx = watch_tx_stream.next() => {
                watch_received.push(tx.unwrap().unwrap());
            },
            tx = sub_tx_stream.next() => {
                sub_received.push(tx.unwrap().unwrap());
            },
        };
        if watch_received.len() == num_txs && sub_received.len() == num_txs {
            if let Some(ref sent) = sent {
                assert_eq!(sent.len(), watch_received.len());
                let sent_txs = sent.iter().map(|tx| tx.transaction_hash).collect::<HashSet<_>>();
                assert_eq!(sent_txs, watch_received.iter().map(|tx| tx.hash).collect());
                assert_eq!(sent_txs, sub_received.iter().map(|tx| tx.hash).collect());
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

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(handle.ethers_http_provider(), wallet));

    let sender = Address::random();
    let other_acc = Address::random();
    let multicall = MulticallContract::deploy(client.clone(), ()).unwrap().send().await.unwrap();
    let simple_storage =
        SimpleStorage::deploy(client.clone(), "foo".to_string()).unwrap().send().await.unwrap();

    // when calling `setValue` on SimpleStorage, both the `lastSender` and `_value` storages are
    // modified The `_value` is a `string`, so the storage slots here (small string) are `0x1`
    // and `keccak(0x1)`
    let set_value_tx = simple_storage.set_value("bar".to_string()).from(sender).tx;
    let access_list = client.create_access_list(&set_value_tx, None).await.unwrap();
    assert_access_list_eq(
        access_list.access_list,
        AccessList::from(vec![AccessListItem {
            address: simple_storage.address(),
            storage_keys: vec![
                H256::zero(),
                H256::from_uint(&(1u64.into())),
                "0xb10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6"
                    .parse()
                    .unwrap(),
            ],
        }]),
    );

    // With a subcall that fetches the balances of an account (`other_acc`), only the address
    // of this account should be in the Access List
    let call_tx = multicall.get_eth_balance(other_acc).from(sender).tx;
    let access_list = client.create_access_list(&call_tx, None).await.unwrap();
    assert_access_list_eq(
        access_list.access_list,
        AccessList::from(vec![AccessListItem { address: other_acc, storage_keys: vec![] }]),
    );

    // With a subcall to another contract, the AccessList should be the same as when calling the
    // subcontract directly (given that the proxy contract doesn't read/write any state)
    let subcall_tx = multicall
        .aggregate(vec![Call {
            target: simple_storage.address(),
            call_data: set_value_tx.data().unwrap().clone(),
        }])
        .from(sender)
        .tx;
    let access_list = client.create_access_list(&subcall_tx, None).await.unwrap();
    assert_access_list_eq(
        access_list.access_list,
        AccessList::from(vec![AccessListItem {
            address: simple_storage.address(),
            storage_keys: vec![
                H256::zero(),
                H256::from_uint(&(1u64.into())),
                "0xb10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6"
                    .parse()
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

    let provider = handle.ethers_http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let recipient = Address::random();

    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let tx = TransactionRequest::new().from(sender).to(recipient).value(1e18 as u64);
    client.send_transaction(tx, None).await.unwrap();

    let tx =
        TransactionRequest::new().from(recipient).to(sender).value(1e10 as u64).data(vec![0x42]);
    api.estimate_gas(to_call_request_from_tx_request(tx), None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reject_gas_too_low() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let account = handle.dev_accounts().next().unwrap();

    let gas = 21_000u64 - 1;
    let tx = TransactionRequest::new()
        .to(Address::random())
        .value(U256::from(1337u64))
        .from(account)
        .gas(gas);

    let resp = provider.send_transaction(tx, None).await;

    let err = resp.unwrap_err().to_string();
    assert!(err.contains("intrinsic gas too low"));
}

// <https://github.com/foundry-rs/foundry/issues/3783>
#[tokio::test(flavor = "multi_thread")]
async fn can_call_with_high_gas_limit() {
    let (_api, handle) =
        spawn(NodeConfig::test().with_gas_limit(Some(U256::from(100_000_000).to_alloy()))).await;
    let provider = handle.ethers_http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string())
        .unwrap()
        .send()
        .await
        .unwrap();

    let greeting = greeter_contract.greet().gas(60_000_000u64).call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reject_eip1559_pre_london() {
    let (api, handle) = spawn(NodeConfig::test().with_hardfork(Some(Hardfork::Berlin))).await;
    let provider = handle.ethers_http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let gas_limit = api.gas_limit();
    let gas_price = api.gas_price().unwrap();
    let unsupported = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string())
        .unwrap()
        .gas(gas_limit.to_ethers())
        .gas_price(gas_price.to_ethers())
        .send()
        .await
        .unwrap_err()
        .to_string();
    assert!(unsupported.contains("not supported by the current hardfork"), "{unsupported}");

    let greeter_contract = Greeter::deploy(Arc::clone(&client), "Hello World!".to_string())
        .unwrap()
        .legacy()
        .send()
        .await
        .unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}
