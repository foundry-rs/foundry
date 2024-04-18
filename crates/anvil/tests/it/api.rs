//! general eth api tests

use crate::{
    abi::{AlloyMulticallContract, AlloySimpleStorage},
    utils::{http_provider, http_provider_with_signer},
};
use alloy_network::{EthereumSigner, TransactionBuilder};
use alloy_primitives::{Address, ChainId, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{
    request::TransactionRequest, BlockId, BlockNumberOrTag, BlockTransactions, WithOtherFields,
};
use anvil::{eth::api::CLIENT_VERSION, spawn, NodeConfig, CHAIN_ID};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn can_get_block_number() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let block_num = api.block_number().unwrap();
    assert_eq!(block_num, U256::from(0));

    let provider = http_provider(&handle.http_endpoint());

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, block_num.to::<u64>());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_dev_get_balance() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = http_provider(&handle.http_endpoint());

    let genesis_balance = handle.genesis_balance();
    for acc in handle.genesis_accounts() {
        let balance = provider.get_balance(acc, BlockId::latest()).await.unwrap();
        assert_eq!(balance, genesis_balance);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_price() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = http_provider(&handle.http_endpoint());

    let _ = provider.get_gas_price().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_accounts() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = http_provider(&handle.http_endpoint());

    let _ = provider.get_accounts().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_client_version() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = http_provider(&handle.http_endpoint());

    let version = provider.get_client_version().await.unwrap();
    assert_eq!(CLIENT_VERSION, version);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_chain_id() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = http_provider(&handle.http_endpoint());

    let chain_id = provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, CHAIN_ID);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_modify_chain_id() {
    let (_api, handle) =
        spawn(NodeConfig::test().with_chain_id(Some(ChainId::from(777_u64)))).await;
    let provider = http_provider(&handle.http_endpoint());

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
    let signer: EthereumSigner = accounts[0].clone().into();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let provider = http_provider(&handle.http_endpoint());
    let provider_with_signer = http_provider_with_signer(&handle.http_endpoint(), signer);

    let val = handle.genesis_balance().checked_div(U256::from(2)).unwrap();

    // send a dummy transaction
    let tx = TransactionRequest::default().with_from(from).with_to(Some(to).into()).with_value(val);
    let tx = WithOtherFields::new(tx);

    provider_with_signer.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let block = provider.get_block(BlockId::number(1), true).await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 1);

    let block =
        provider.get_block(BlockId::hash(block.header.hash.unwrap()), true).await.unwrap().unwrap();
    assert_eq!(block.transactions.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_get_pending_block() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let signer: EthereumSigner = accounts[0].clone().into();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let provider = http_provider(&handle.http_endpoint());
    let provider_with_signer = http_provider_with_signer(&handle.http_endpoint(), signer);

    let block = provider.get_block(BlockId::pending(), false).await.unwrap().unwrap();
    assert_eq!(block.header.number.unwrap(), 1);

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 0);

    api.anvil_set_auto_mine(false).await.unwrap();

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(Some(to).into())
        .with_value(U256::from(100));
    let tx = WithOtherFields::new(tx);

    let receipt = provider_with_signer
        .send_transaction(tx.clone())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 0);

    let block = provider.get_block(BlockId::pending(), false).await.unwrap().unwrap();
    assert_eq!(block.header.number.unwrap(), 1);
    assert_eq!(block.transactions.len(), 1);
    assert_eq!(block.transactions, BlockTransactions::Hashes(vec![receipt.transaction_hash]));

    let block = provider.get_block(BlockId::pending(), true).await.unwrap().unwrap();
    assert_eq!(block.header.gas_limit, 1);
    assert_eq!(block.transactions.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_on_pending_block() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();

    let provider = http_provider(&handle.http_endpoint());
    let provider_with_signer = http_provider_with_signer(&handle.http_endpoint(), signer);

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 0);

    api.anvil_set_auto_mine(false).await.unwrap();

    let multicall_contract_builder =
        AlloyMulticallContract::deploy_builder(&provider_with_signer).from(wallet.address());
    let multicall_contract_address = multicall_contract_builder.deploy().await.unwrap();
    let pending_multicall_contract =
        AlloyMulticallContract::new(multicall_contract_address, &provider);

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, 0);

    // Ensure that we can get the block_number from the pending contract
    let AlloyMulticallContract::aggregateReturn { blockNumber: ret_block_number, .. } =
        pending_multicall_contract
            .aggregate(vec![])
            .block(BlockId::pending())
            .call()
            .await
            .unwrap();
    assert_eq!(ret_block_number, U256::from(1));

    let accounts: Vec<Address> = handle.dev_wallets().map(|w| w.address()).collect();

    for i in 1..10 {
        api.anvil_set_coinbase(accounts[i % accounts.len()]).await.unwrap();
        api.evm_set_block_gas_limit(U256::from(30_000_000 + i)).unwrap();

        api.anvil_mine(Some(U256::from(1)), None).await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // Ensure that the right header values are set when calling a past block
    for anvil_block_number in 1..(api.block_number().unwrap().to::<usize>() + 1) {
        let block_number = BlockNumberOrTag::Number(anvil_block_number as u64);
        let block = api.block_by_number(block_number).await.unwrap().unwrap();

        let AlloyMulticallContract::getCurrentBlockTimestampReturn {
            timestamp: ret_timestamp, ..
        } = pending_multicall_contract
            .getCurrentBlockTimestamp()
            .block(BlockId::number(anvil_block_number as u64))
            .call()
            .await
            .unwrap();
        assert_eq!(block.header.timestamp, ret_timestamp.to::<u64>());

        let AlloyMulticallContract::getCurrentBlockGasLimitReturn {
            gaslimit: ret_gas_limit, ..
        } = pending_multicall_contract
            .getCurrentBlockGasLimit()
            .block(BlockId::number(anvil_block_number as u64))
            .call()
            .await
            .unwrap();
        assert_eq!(block.header.gas_limit, ret_gas_limit.to::<u128>());

        let AlloyMulticallContract::getCurrentBlockCoinbaseReturn {
            coinbase: ret_coinbase, ..
        } = pending_multicall_contract
            .getCurrentBlockCoinbase()
            .block(BlockId::number(anvil_block_number as u64))
            .call()
            .await
            .unwrap();
        assert_eq!(block.header.miner, ret_coinbase);
    }
}

// async fn call_with_override<M, D>(
//     api: &EthApi,
//     call: ContractCall<M, D>,
//     to: Address,
//     overrides: StateOverride,
// ) -> D
// where
//     D: Tokenizable,
// {
//     let result = api
//         .call(
//             WithOtherFields::new(TransactionRequest {
//                 input: TransactionInput::maybe_input(call.tx.data().cloned().map(|b|
// b.0.into())),                 to: Some(to),
//                 ..Default::default()
//             }),
//             None,
//             Some(overrides),
//         )
//         .await
//         .unwrap();
//     decode_function_data(&call.function, result.as_ref(), false).unwrap()
// }

#[tokio::test(flavor = "multi_thread")]
async fn can_call_with_state_override() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();

    let provider = http_provider(&handle.http_endpoint());
    let provider_with_signer = http_provider_with_signer(&handle.http_endpoint(), signer);

    api.anvil_set_auto_mine(true).await.unwrap();

    let multicall_contract_builder =
        AlloyMulticallContract::deploy_builder(&provider_with_signer).from(wallet.address());
    let multicall_contract_address = multicall_contract_builder.deploy().await.unwrap();
    let multicall_contract = AlloyMulticallContract::new(multicall_contract_address, &provider);

    let init_value = "toto".to_string();
    let simple_storage_contract_builder =
        AlloySimpleStorage::deploy_builder(&provider_with_signer, init_value.clone())
            .from(wallet.address());
    let simple_storage_contract_address = simple_storage_contract_builder.deploy().await.unwrap();
    let simple_storage_contract =
        AlloySimpleStorage::new(simple_storage_contract_address, &provider);

    // // Test the `balance` account override
    // let balance = rU256::from(42u64);
    // let result = call_with_override(
    //     &api,
    //     multicall.get_eth_balance(account),
    //     multicall.address(),
    //     HashMap::from([(
    //         account.to_alloy(),
    //         AccountOverride { balance: Some(balance), ..Default::default() },
    //     )]),
    // )
    // .await;
    // assert_eq!(result, balance.to_ethers());

    // // Test the `state_diff` account override
    // let overrides = HashMap::from([(
    //     simple_storage.address().to_alloy(),
    //     AccountOverride {
    //         // The `lastSender` is in the first storage slot
    //         state_diff: Some(HashMap::from([(
    //             B256::ZERO,
    //             rU256::from_be_slice(B256::from(account.to_alloy().into_word()).as_slice()),
    //         )])),
    //         ..Default::default()
    //     },
    // )]);

    // let last_sender = call_with_override(
    //     &api,
    //     simple_storage.last_sender(),
    //     simple_storage.address(),
    //     Default::default(),
    // )
    // .await;
    // // No `sender` set without override
    // assert_eq!(last_sender, Address::zero());
    // let last_sender = call_with_override(
    //     &api,
    //     simple_storage.last_sender(),
    //     simple_storage.address(),
    //     overrides.clone(),
    // )
    // .await;
    // // `sender` *is* set with override
    // assert_eq!(last_sender, account);
    // let value =
    //     call_with_override(&api, simple_storage.get_value(), simple_storage.address(), overrides)
    //         .await;
    // // `value` *is not* changed with state-diff
    // assert_eq!(value, init_value);

    // // Test the `state` account override
    // let overrides = HashMap::from([(
    //     simple_storage.address().to_alloy(),
    //     AccountOverride {
    //         // The `lastSender` is in the first storage slot
    //         state: Some(HashMap::from([(
    //             B256::ZERO,
    //             rU256::from_be_slice(B256::from(account.to_alloy().into_word()).as_slice()),
    //         )])),
    //         ..Default::default()
    //     },
    // )]);

    // let last_sender = call_with_override(
    //     &api,
    //     simple_storage.last_sender(),
    //     simple_storage.address(),
    //     overrides.clone(),
    // )
    // .await;
    // // `sender` *is* set with override
    // assert_eq!(last_sender, account);
    // let value =
    //     call_with_override(&api, simple_storage.get_value(), simple_storage.address(), overrides)
    //         .await;
    // // `value` *is* changed with state
    // assert_eq!(value, "");
}
