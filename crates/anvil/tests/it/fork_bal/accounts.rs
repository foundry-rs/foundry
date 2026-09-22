//! Account creation, deletion and delegation across a prefilled fork.

use super::{BalOrigin, CONTRACT, cached_storage, has_cached_account};
use crate::abi::{COUNTER_INIT_CODE, COUNTER_RUNTIME_CODE};
use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes, U256, address, bytes};
use alloy_provider::Provider;
use alloy_rpc_types::{Authorization, BlockId, BlockNumberOrTag, TransactionRequest};
use alloy_serde::WithOtherFields;
use alloy_signer::SignerSync;
use anvil::{EthereumHardfork, eth::EthApi, spawn};
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

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_prefills_complete_created_accounts() {
    let mut origin = BalOrigin::new().await;
    let nonce = origin.handle.http_provider().get_transaction_count(origin.sender).await.unwrap();
    let contract = origin.sender.create(nonce);
    let (success, _) = mine_transaction(
        &origin.api,
        TransactionRequest::default()
            .with_from(origin.sender)
            .with_deploy_code(COUNTER_INIT_CODE)
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
        let (api, _handle) = spawn(origin.config().with_no_bal(no_bal)).await;
        assert_eq!(has_cached_account(&api, contract).await, !no_bal);
        assert_eq!(api.balance(contract, None).await.unwrap(), U256::from(42));
        assert_eq!(api.transaction_count(contract, None).await.unwrap(), U256::ONE);
        assert_eq!(api.get_code(contract, None).await.unwrap(), COUNTER_RUNTIME_CODE);

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
        assert_eq!(api.get_code(contract, None).await.unwrap(), COUNTER_RUNTIME_CODE);
    }
    assert!(outcomes[0].0);
    assert_eq!(outcomes[0], outcomes[1]);

    // The BAL remains unchanged when Anvil account overrides modify its block's state.
    origin.api.anvil_set_balance(contract, U256::from(99)).await.unwrap();
    origin.api.anvil_set_nonce(contract, U256::from(7)).await.unwrap();
    origin.api.anvil_set_code(contract, bytes!("00")).await.unwrap();
    let (api, _handle) = spawn(origin.config()).await;
    assert!(has_cached_account(&api, contract).await);
    assert_eq!(api.balance(contract, None).await.unwrap(), U256::from(99));
    assert_eq!(api.transaction_count(contract, None).await.unwrap(), U256::from(7));
    assert_eq!(api.get_code(contract, None).await.unwrap(), bytes!("00"));
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
        let (api, _handle) = spawn(origin.config().with_no_bal(no_bal)).await;
        assert_eq!(cached_storage(&api, child, U256::ZERO).await, None);
        assert_eq!(api.storage_at(child, U256::ZERO, None).await.unwrap(), B256::ZERO);
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
async fn fork_bal_historical_storage_keeps_local_deletion() {
    let mut origin = BalOrigin::new().await;
    // Empty calldata increments slot zero; nonempty calldata selfdestructs to the caller.
    origin
        .api
        .anvil_set_code(CONTRACT, bytes!("361560075733ff5b60005460010160005500"))
        .await
        .unwrap();
    BalOrigin::increment(&origin.api, origin.sender).await;
    pin_latest(&mut origin).await;

    for no_bal in [false, true] {
        // Source eligibility is Cancun+, but local Shanghai execution can delete existing storage.
        let (api, _handle) = spawn(
            origin
                .config()
                .with_no_bal(no_bal)
                .with_hardfork(Some(EthereumHardfork::Shanghai.into())),
        )
        .await;
        assert_eq!(
            cached_storage(&api, CONTRACT, U256::ZERO).await,
            (!no_bal).then_some(U256::from(2))
        );

        // Keep the old slot unread: only BAL prefill should add it to the remote snapshot.
        let (success, _) = mine_transaction(
            &api,
            TransactionRequest::default()
                .with_from(origin.sender)
                .with_to(CONTRACT)
                .with_input(bytes!("01"))
                .with_gas_limit(200_000),
        )
        .await;
        assert!(success);
        let deleted_block = BlockId::number(api.block_number().unwrap().to());
        assert!(api.get_code(CONTRACT, None).await.unwrap().is_empty());
        assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::ZERO);

        api.mine_one().await.unwrap();
        assert_eq!(
            api.storage_at(CONTRACT, U256::ZERO, Some(deleted_block)).await.unwrap(),
            B256::ZERO,
            "historical storage must stay deleted with no_bal={no_bal}"
        );
    }
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
            let (api, _handle) = spawn(origin.config().with_no_bal(no_bal)).await;
            assert_eq!(has_cached_account(&api, authority.address()).await, !no_bal);
            assert_eq!(api.balance(authority.address(), None).await.unwrap(), expected_balance);
            assert_eq!(
                api.transaction_count(authority.address(), None).await.unwrap(),
                expected_nonce
            );
            assert_eq!(api.get_code(authority.address(), None).await.unwrap(), expected_code);
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
