use crate::next_port;
use anvil::{spawn, NodeConfig};
use ethers::{
    contract::{ContractFactory, EthEvent},
    prelude::{abigen, BlockId, Middleware, Signer, SignerMiddleware, TransactionRequest},
    types::{Address, H256, U256},
};
use ethers_solc::{project_util::TempProject, Artifact};
use futures::StreamExt;
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
