//! general eth api tests

use anvil::{
    eth::{api::CLIENT_VERSION, EthApi},
    spawn, NodeConfig, CHAIN_ID,
};
use anvil_core::eth::{state::AccountOverride, transaction::EthTransactionRequest};
use ethers::{
    abi::{Address, Tokenizable},
    prelude::{builders::ContractCall, decode_function_data, Middleware, SignerMiddleware},
    signers::Signer,
    types::{Block, BlockNumber, Chain, Transaction, TransactionRequest, H256, U256},
    utils::get_contract_address,
};
use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::abi::{MulticallContract, SimpleStorage};

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_number() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::zero());

    let provider = handle.http_provider();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.as_u64().into());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_dev_get_balance() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let genesis_balance = handle.genesis_balance();
    for acc in handle.genesis_accounts() {
        let balance = provider.get_balance(acc, None).await.unwrap();
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

    let version = provider.client_version().await.unwrap();
    assert_eq!(CLIENT_VERSION, version);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_chain_id() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let chain_id = provider.get_chainid().await.unwrap();
    assert_eq!(chain_id, CHAIN_ID.into());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_modify_chain_id() {
    let (_api, handle) = spawn(NodeConfig::test().with_chain_id(Some(Chain::Goerli))).await;
    let provider = handle.http_provider();

    let chain_id = provider.get_chainid().await.unwrap();
    assert_eq!(chain_id, Chain::Goerli.into());

    let chain_id = provider.get_net_version().await.unwrap();
    assert_eq!(chain_id, (Chain::Goerli as u64).to_string());
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
    let provider = handle.http_provider();
    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();
    // send a dummy transactions
    let tx = TransactionRequest::new().to(to).value(amount).from(from);
    let _ = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let block: Block<Transaction> = provider.get_block_with_txs(1u64).await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 1);

    let block = provider.get_block(1u64).await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 1);

    let block = provider.get_block(block.hash.unwrap()).await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_pending_block() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let accounts: Vec<_> = handle.dev_wallets().collect();

    let block = provider.get_block(BlockNumber::Pending).await.unwrap().unwrap();

    assert_eq!(block.number.unwrap().as_u64(), 1u64);

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 0u64);

    api.anvil_set_auto_mine(false).await.unwrap();

    let from = accounts[0].address();
    let to = accounts[1].address();
    let tx = TransactionRequest::new().to(to).value(100u64).from(from);

    let tx = provider.send_transaction(tx, None).await.unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 0u64);

    let block = provider.get_block(BlockNumber::Pending).await.unwrap().unwrap();
    assert_eq!(block.number.unwrap().as_u64(), 1u64);
    assert_eq!(block.transactions.len(), 1);
    assert_eq!(block.transactions, vec![tx.tx_hash()]);

    let block = provider.get_block_with_txs(BlockNumber::Pending).await.unwrap().unwrap();
    assert_eq!(block.number.unwrap().as_u64(), 1u64);
    assert_eq!(block.transactions.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_on_pending_block() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 0u64);

    api.anvil_set_auto_mine(false).await.unwrap();

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let mut deploy_tx = MulticallContract::deploy(Arc::clone(&client), ()).unwrap().deployer.tx;
    deploy_tx.set_nonce(0);
    let pending_contract_address = get_contract_address(sender, deploy_tx.nonce().unwrap());

    client.send_transaction(deploy_tx, None).await.unwrap();

    let pending_contract = MulticallContract::new(pending_contract_address, client.clone());

    let num = client.get_block_number().await.unwrap();
    assert_eq!(num.as_u64(), 0u64);

    // Ensure that we can get the block_number from the pending contract
    let (ret_block_number, _) =
        pending_contract.aggregate(vec![]).block(BlockNumber::Pending).call().await.unwrap();
    assert_eq!(ret_block_number.as_u64(), 1u64);

    let accounts: Vec<Address> = handle.dev_wallets().map(|w| w.address()).collect();
    for i in 1..10 {
        api.anvil_set_coinbase(accounts[i % accounts.len()]).await.unwrap();
        api.evm_set_block_gas_limit((30_000_000 + i).into()).unwrap();

        api.anvil_mine(Some(1.into()), None).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    // Ensure that the right header values are set when calling a past block
    for block_number in 1..(api.block_number().unwrap().as_usize() + 1) {
        let block_number = BlockNumber::Number(block_number.into());
        let block = api.block_by_number(block_number).await.unwrap().unwrap();

        let block_timestamp = pending_contract
            .get_current_block_timestamp()
            .block(block_number)
            .call()
            .await
            .unwrap();
        assert_eq!(block.timestamp, block_timestamp);

        let block_gas_limit = pending_contract
            .get_current_block_gas_limit()
            .block(block_number)
            .call()
            .await
            .unwrap();
        assert_eq!(block.gas_limit, block_gas_limit);

        let block_coinbase =
            pending_contract.get_current_block_coinbase().block(block_number).call().await.unwrap();
        assert_eq!(block.author.unwrap(), block_coinbase);
    }
}

async fn call_with_override<M, D>(
    api: &EthApi,
    call: ContractCall<M, D>,
    to: Address,
    overrides: HashMap<Address, AccountOverride>,
) -> D
where
    D: Tokenizable,
{
    let result = api
        .call(
            EthTransactionRequest {
                data: call.tx.data().cloned(),
                to: Some(to),
                ..Default::default()
            },
            None,
            Some(overrides),
        )
        .await
        .unwrap();
    decode_function_data(&call.function, result.as_ref(), false).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_with_state_override() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    api.anvil_set_auto_mine(true).await.unwrap();

    let wallet = handle.dev_wallets().next().unwrap();
    let account = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let init_value = "toto".to_string();
    let multicall =
        MulticallContract::deploy(Arc::clone(&client), ()).unwrap().send().await.unwrap();
    let simple_storage = SimpleStorage::deploy(Arc::clone(&client), init_value.clone())
        .unwrap()
        .send()
        .await
        .unwrap();

    // Test the `balance` account override
    let balance = 42u64.into();
    let result = call_with_override(
        &api,
        multicall.get_eth_balance(account),
        multicall.address(),
        HashMap::from([(
            account,
            AccountOverride { balance: Some(balance), ..Default::default() },
        )]),
    )
    .await;
    assert_eq!(result, balance);

    // Test the `state_diff` account override
    let overrides = HashMap::from([(
        simple_storage.address(),
        AccountOverride {
            // The `lastSender` is in the first storage slot
            state_diff: Some(HashMap::from([(H256::from_low_u64_be(0), account.into())])),
            ..Default::default()
        },
    )]);

    let last_sender = call_with_override(
        &api,
        simple_storage.last_sender(),
        simple_storage.address(),
        Default::default(),
    )
    .await;
    // No `sender` set without override
    assert_eq!(last_sender, Address::zero());
    let last_sender = call_with_override(
        &api,
        simple_storage.last_sender(),
        simple_storage.address(),
        overrides.clone(),
    )
    .await;
    // `sender` *is* set with override
    assert_eq!(last_sender, account);
    let value =
        call_with_override(&api, simple_storage.get_value(), simple_storage.address(), overrides)
            .await;
    // `value` *is not* changed with state-diff
    assert_eq!(value, init_value);

    // Test the `state` account override
    let overrides = HashMap::from([(
        simple_storage.address(),
        AccountOverride {
            // The `lastSender` is in the first storage slot
            state: Some(HashMap::from([(H256::from_low_u64_be(0), account.into())])),
            ..Default::default()
        },
    )]);

    let last_sender = call_with_override(
        &api,
        simple_storage.last_sender(),
        simple_storage.address(),
        overrides.clone(),
    )
    .await;
    // `sender` *is* set with override
    assert_eq!(last_sender, account);
    let value =
        call_with_override(&api, simple_storage.get_value(), simple_storage.address(), overrides)
            .await;
    // `value` *is* changed with state
    assert_eq!(value, "");
}
