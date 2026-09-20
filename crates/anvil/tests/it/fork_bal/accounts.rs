//! Account creation, deletion and delegation across a prefilled fork.

use super::{BalOrigin, BalProxy, BalResponse, CONTRACT};
use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U256, address, bytes};
use alloy_provider::Provider;
use alloy_rpc_types::{Authorization, BlockNumberOrTag, TransactionRequest};
use alloy_serde::WithOtherFields;
use alloy_signer::SignerSync;
use anvil::{eth::EthApi, spawn};
use foundry_primitives::FoundryNetwork;

async fn pin_latest(origin: &mut BalOrigin) {
    let block = origin
        .handle
        .http_provider()
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap();
    origin.block_number = block.header.number;
    origin.block_hash = block.header.hash;
}

async fn mine_transaction(
    api: &EthApi<FoundryNetwork>,
    request: TransactionRequest,
) -> (bool, u64) {
    let hash = api.send_transaction(WithOtherFields::new(request)).await.unwrap();
    api.mine_one().await.unwrap();
    let receipt = api.transaction_receipt(hash).await.unwrap().unwrap();
    (receipt.status(), receipt.gas_used())
}

fn account_requests(proxy: &BalProxy) -> usize {
    ["eth_getAccountInfo", "eth_getBalance", "eth_getTransactionCount", "eth_getCode"]
        .into_iter()
        .map(|method| proxy.count(method))
        .sum()
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_complete_created_account_avoids_account_rpc() {
    let mut origin = BalOrigin::new().await;
    let nonce = origin.handle.http_provider().get_transaction_count(origin.sender).await.unwrap();
    let contract = origin.sender.create(nonce);
    // Initialize slot zero to one, then install a runtime that increments it on every call.
    let runtime = bytes!("60005460010160005500");
    let deployment = bytes!("6001600055600a6011600039600a6000f360005460010160005500");
    let (success, _) = mine_transaction(
        &origin.api,
        TransactionRequest::default()
            .with_from(origin.sender)
            .with_deploy_code(deployment)
            .with_value(U256::from(42))
            .with_gas_limit(1_000_000),
    )
    .await;
    assert!(success);
    pin_latest(&mut origin).await;
    let bal = origin
        .handle
        .http_provider()
        .get_block_access_list_by_hash(origin.block_hash)
        .await
        .unwrap()
        .unwrap();
    assert!(
        bal.iter()
            .find(|account| account.address == contract)
            .unwrap()
            .account_info()
            .is_complete()
    );

    let mut outcomes = Vec::new();
    for no_bal in [false, true] {
        let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
        let (api, _handle) = spawn(origin.config(&proxy).with_no_bal(no_bal)).await;
        proxy.clear();
        assert_eq!(api.balance(contract, None).await.unwrap(), U256::from(42));
        assert_eq!(api.transaction_count(contract, None).await.unwrap(), U256::ONE);
        assert_eq!(api.get_code(contract, None).await.unwrap(), runtime);
        assert_eq!(account_requests(&proxy) == 0, !no_bal);

        outcomes.push(
            mine_transaction(
                &api,
                TransactionRequest::default()
                    .with_from(origin.sender)
                    .with_to(contract)
                    .with_gas_limit(200_000),
            )
            .await,
        );
        assert_eq!(
            api.storage_at(contract, U256::ZERO, None).await.unwrap(),
            B256::from(U256::from(2))
        );
        assert_eq!(api.get_code(contract, None).await.unwrap(), runtime);
    }
    assert!(outcomes[0].0);
    assert_eq!(outcomes[0], outcomes[1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_create_destroy_and_create2_reuse_keep_storage_empty() {
    let mut origin = BalOrigin::new().await;
    let factory = address!("000000000000000000000000000000000000fac7");
    // The child writes slot zero, then selfdestructs to its caller.
    let runtime = bytes!("602a60005533ff");
    let init = bytes!("6007600c60003960076000f3602a60005533ff");
    // CREATE2 with salt zero, and call the child only when calldata is nonempty.
    let mut factory_code =
        bytes!("6013601c5f395f60135f34f53615601a575f5f5f5f5f855af1505b00").to_vec();
    factory_code.extend_from_slice(&init);
    origin.api.anvil_set_code(factory, factory_code.into()).await.unwrap();
    let child = factory.create2_from_code(B256::ZERO, &init);
    let (success, _) = mine_transaction(
        &origin.api,
        TransactionRequest::default()
            .with_from(origin.sender)
            .with_to(factory)
            .with_input(bytes!("01"))
            .with_gas_limit(1_000_000),
    )
    .await;
    assert!(success);
    assert!(origin.api.get_code(child, None).await.unwrap().is_empty());
    assert_eq!(origin.api.storage_at(child, U256::ZERO, None).await.unwrap(), B256::ZERO);
    pin_latest(&mut origin).await;

    let bal = origin
        .handle
        .http_provider()
        .get_block_access_list_by_hash(origin.block_hash)
        .await
        .unwrap()
        .unwrap();
    let changes = bal.iter().find(|account| account.address == child).unwrap();
    assert!(changes.storage_changes.is_empty());
    assert_eq!(changes.storage_reads, vec![U256::ZERO]);
    assert!(changes.nonce_changes.is_empty());
    assert!(changes.code_changes.is_empty());

    let mut outcomes = Vec::new();
    for no_bal in [false, true] {
        let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
        let (api, _handle) = spawn(origin.config(&proxy).with_no_bal(no_bal)).await;
        proxy.clear();
        assert_eq!(api.storage_at(child, U256::ZERO, None).await.unwrap(), B256::ZERO);
        assert_eq!(proxy.count("eth_getStorageAt"), 1, "deleted writes must remain lazy");
        outcomes.push(
            mine_transaction(
                &api,
                TransactionRequest::default()
                    .with_from(origin.sender)
                    .with_to(factory)
                    .with_gas_limit(1_000_000),
            )
            .await,
        );
        assert_eq!(api.get_code(child, None).await.unwrap(), runtime);
        assert_eq!(api.transaction_count(child, None).await.unwrap(), U256::ONE);
        assert_eq!(api.storage_at(child, U256::ZERO, None).await.unwrap(), B256::ZERO);
    }
    assert!(outcomes[0].0);
    assert_eq!(outcomes[0], outcomes[1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_eip7702_set_and_clear_match_lazy_state() {
    let mut origin = BalOrigin::new().await;
    let authority = origin.handle.dev_wallets().nth(1).unwrap();
    for target in [CONTRACT, Address::ZERO] {
        let authorization = Authorization {
            chain_id: U256::ONE,
            address: target,
            nonce: origin
                .handle
                .http_provider()
                .get_transaction_count(authority.address())
                .await
                .unwrap(),
        };
        let signature = authority.sign_hash_sync(&authorization.signature_hash()).unwrap();
        let request = TransactionRequest {
            authorization_list: Some(vec![authorization.into_signed(signature)]),
            ..TransactionRequest::default()
        }
        .with_from(origin.sender)
        .with_to(authority.address())
        .with_value(U256::ONE)
        .with_gas_limit(200_000);
        let (success, _) = mine_transaction(&origin.api, request).await;
        assert!(success);
        pin_latest(&mut origin).await;
        let expected_balance = origin.api.balance(authority.address(), None).await.unwrap();
        let expected_nonce = origin.api.transaction_count(authority.address(), None).await.unwrap();
        let expected_code = if target == Address::ZERO {
            Bytes::new()
        } else {
            let mut code = bytes!("ef0100").to_vec();
            code.extend_from_slice(target.as_slice());
            code.into()
        };
        assert_eq!(origin.api.get_code(authority.address(), None).await.unwrap(), expected_code);

        let mut outcomes = Vec::new();
        for no_bal in [false, true] {
            let proxy = BalProxy::new(&origin.handle, BalResponse::Valid, false).await;
            let (api, _handle) = spawn(origin.config(&proxy).with_no_bal(no_bal)).await;
            proxy.clear();
            assert_eq!(api.balance(authority.address(), None).await.unwrap(), expected_balance);
            assert_eq!(
                api.transaction_count(authority.address(), None).await.unwrap(),
                expected_nonce
            );
            assert_eq!(api.get_code(authority.address(), None).await.unwrap(), expected_code);
            assert_eq!(account_requests(&proxy) == 0, !no_bal);
            assert_eq!(
                api.storage_at(authority.address(), U256::ZERO, None).await.unwrap(),
                B256::from(U256::ONE)
            );
            outcomes.push(
                mine_transaction(
                    &api,
                    TransactionRequest::default()
                        .with_from(origin.sender)
                        .with_to(authority.address())
                        .with_gas_limit(200_000),
                )
                .await,
            );
            let expected_storage = if target.is_zero() { 1 } else { 2 };
            assert_eq!(
                api.storage_at(authority.address(), U256::ZERO, None).await.unwrap(),
                B256::from(U256::from(expected_storage))
            );
        }
        assert!(outcomes[0].0);
        assert_eq!(outcomes[0], outcomes[1]);
    }
}
