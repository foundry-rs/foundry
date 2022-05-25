use crate::next_port;
use anvil::{spawn, NodeConfig};
use ethers::{
    contract::{ContractFactory, EthEvent},
    prelude::{
        abigen, signer::SignerMiddlewareError, BlockId, Middleware, Signer, SignerMiddleware,
        TransactionRequest,
    },
    types::{Address, BlockNumber, H256, U256},
};
use ethers_solc::{project_util::TempProject, Artifact};
use futures::{future::join_all, StreamExt};
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;

abigen!(Greeter, "test-data/greeter.json");
abigen!(SimpleStorage, "test-data/SimpleStorage.json");
abigen!(MulticallContract, "test-data/multicall.json");

#[derive(Clone, Debug, EthEvent)]
pub struct ValueChanged {
    #[ethevent(indexed)]
    pub old_author: Address,
    #[ethevent(indexed)]
    pub new_author: Address,
    pub old_value: String,
    pub new_value: String,
}

#[tokio::test(flavor = "multi_thread")]
async fn can_transfer_eth() {
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

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
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

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
    api.mine_one();

    // get the block, await receipts
    let block = provider.get_block(BlockNumber::Latest).await.unwrap().unwrap();
    let lower_price = tx_lower.await.unwrap().unwrap().transaction_hash;
    let higher_price = tx_higher.await.unwrap().unwrap().transaction_hash;
    assert_eq!(block.transactions, vec![higher_price, lower_price])
}

#[tokio::test(flavor = "multi_thread")]
async fn can_respect_nonces() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

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
    let res = timeout(Duration::from_millis(1500), async move { listener.next().await }).await;
    assert!(res.is_err());

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
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;

    // disable auto mining
    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();

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
    api.mine_one();

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
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let gas_limit = api.gas_limit();
    let amount = handle.genesis_balance().checked_div(3u64.into()).unwrap();

    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    // send transaction with the exact gas limit
    let pending = provider.send_transaction(tx.clone().gas(gas_limit), None).await;

    assert!(pending.is_ok());

    // send transaction with higher gas limit
    let pending = provider.send_transaction(tx.clone().gas(gas_limit + 1u64), None).await;

    assert!(pending.is_err());
    let err = pending.unwrap_err();
    assert!(err.to_string().contains("gas too high"));

    api.anvil_set_balance(from, U256::MAX).await.unwrap();
    api.anvil_set_min_gas_price(0u64.into()).await.unwrap();

    let pending = provider.send_transaction(tx.gas(gas_limit), None).await;
    assert!(pending.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_reject_underpriced_replacement() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;

    // disable auto mining
    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();

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
    api.mine_one();
    let higher_priced_receipt = higher_priced_pending_tx.await.unwrap().unwrap();

    // ensure that only the higher priced tx was mined
    let block = provider.get_block(1u64).await.unwrap().unwrap();
    assert_eq!(1, block.transactions.len());
    assert_eq!(vec![higher_priced_receipt.transaction_hash], block.transactions);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_deploy_greeter_http() {
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

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
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;

    // can mine in auto-mine mode
    api.evm_mine(None).await.unwrap();
    // disable auto mine
    api.anvil_set_auto_mine(false).await.unwrap();
    // can mine in manual mode
    api.evm_mine(None).await.unwrap();

    let provider = handle.http_provider();

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
async fn can_call_greeter_historic() {
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

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
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.ws_provider().await;

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
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.ws_provider().await;

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

#[test]
fn test_deploy_reverting() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r#"
pragma solidity 0.8.13;
contract Contract {
    constructor() {
      require(false, "");
    }
}
"#,
    )
    .unwrap();

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    // need to run this in a runtime because svm's blocking install does panic if invoked in another
    // async runtime
    tokio::runtime::Runtime::new().unwrap().block_on(async move {
        let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
        let provider = handle.ws_provider().await;

        let wallet = handle.dev_wallets().next().unwrap();
        let client = Arc::new(SignerMiddleware::new(provider, wallet));

        let factory = ContractFactory::new(abi.unwrap(), bytecode.unwrap(), client);
        let contract = factory.deploy(()).unwrap().send().await;
        assert!(contract.is_err());

        // should catch the revert during estimation which results in an err
        let err = contract.unwrap_err();
        assert!(err.to_string().contains("execution reverted:"));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn get_past_events() {
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let address = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let contract = SimpleStorage::deploy(Arc::clone(&client), "initial value".to_string())
        .unwrap()
        .send()
        .await
        .unwrap();

    let func = contract.method::<_, H256>("setValue", "hi".to_owned()).unwrap();
    let tx = func.send().await.unwrap();
    let _receipt = tx.await.unwrap();

    // and we can fetch the events
    let logs: Vec<ValueChanged> =
        contract.event().from_block(0u64).topic1(address).query().await.unwrap();

    // 2 events, 1 in constructor, 1 in call
    assert_eq!(logs[0].new_value, "initial value");
    assert_eq!(logs[1].new_value, "hi");
    assert_eq!(logs.len(), 2);

    // and we can fetch the events at a block hash
    let hash = client.get_block(1).await.unwrap().unwrap().hash.unwrap();

    let logs: Vec<ValueChanged> =
        contract.event().at_block_hash(hash).topic1(address).query().await.unwrap();
    assert_eq!(logs[0].new_value, "initial value");
    assert_eq!(logs.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn get_blocktimestamp_works() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let contract =
        MulticallContract::deploy(Arc::clone(&client), ()).unwrap().send().await.unwrap();

    let timestamp = contract.get_current_block_timestamp().call().await.unwrap();

    assert!(timestamp > U256::one());

    // mock timestamp
    api.evm_set_next_block_timestamp(1337).unwrap();

    let timestamp = contract.get_current_block_timestamp().call().await.unwrap();
    assert_eq!(timestamp, 1337u64.into());

    // repeat call same result
    let timestamp = contract.get_current_block_timestamp().call().await.unwrap();
    assert_eq!(timestamp, 1337u64.into());
}

#[tokio::test(flavor = "multi_thread")]
async fn call_past_state() {
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.http_provider();

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
    let _tx_hash =
        *contract.method::<_, H256>("setValue", "hi".to_owned()).unwrap().send().await.unwrap();

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
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;

    let provider = handle.ws_provider().await;

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
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.ws_provider().await;

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
    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.ws_provider().await;

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
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;

    // disable auto mining so we can check if we can return pending tx from the mempool
    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();

    let from = handle.dev_wallets().next().unwrap().address();
    let tx = TransactionRequest::new().from(from).value(1337u64).to(Address::random());
    let tx = provider.send_transaction(tx, None).await.unwrap();

    let pending = provider.get_transaction(tx.tx_hash()).await.unwrap();
    assert!(pending.is_some());

    api.mine_one();
    let mined = provider.get_transaction(tx.tx_hash()).await.unwrap().unwrap();

    assert_eq!(mined.hash, pending.unwrap().hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn includes_pending_tx_for_transaction_count() {
    let (api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;

    api.anvil_set_auto_mine(false).await.unwrap();

    let provider = handle.http_provider();
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

    api.mine_one();
    let nonce = provider
        .get_transaction_count(from, Some(BlockId::Number(BlockNumber::Pending)))
        .await
        .unwrap();
    assert_eq!(nonce, tx_count.into());
}
