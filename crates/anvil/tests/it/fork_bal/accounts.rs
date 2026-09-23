//! Account creation and historical deletion across a prefilled fork.

use super::{BalOrigin, CONTRACT, cached_storage, has_cached_account};
use crate::abi::{COUNTER_INIT_CODE, COUNTER_RUNTIME_CODE};
use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{B256, U256, bytes};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, BlockNumberOrTag, TransactionRequest};
use alloy_serde::WithOtherFields;
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

async fn mine_transaction(api: &EthApi<FoundryNetwork>, request: TransactionRequest) {
    let hash = api.send_transaction(WithOtherFields::new(request)).await.unwrap();
    api.mine_one().await.unwrap();
    assert!(api.transaction_receipt(hash).await.unwrap().unwrap().status());
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_bal_prefills_complete_created_accounts() {
    let mut origin = BalOrigin::new().await;
    let nonce = origin.handle.http_provider().get_transaction_count(origin.sender).await.unwrap();
    let contract = origin.sender.create(nonce);
    mine_transaction(
        &origin.api,
        TransactionRequest::default()
            .with_from(origin.sender)
            .with_deploy_code(COUNTER_INIT_CODE)
            .with_value(U256::from(42))
            .with_gas_limit(1_000_000),
    )
    .await;
    pin_latest(&mut origin).await;

    let (api, _handle) = spawn(origin.config()).await;
    assert!(has_cached_account(&api, contract).await);
    assert_eq!(api.balance(contract, None).await.unwrap(), U256::from(42));
    assert_eq!(api.transaction_count(contract, None).await.unwrap(), U256::ONE);
    assert_eq!(api.get_code(contract, None).await.unwrap(), COUNTER_RUNTIME_CODE);

    mine_transaction(
        &api,
        TransactionRequest::default()
            .with_from(origin.sender)
            .with_to(contract)
            .with_gas_limit(200_000),
    )
    .await;
    assert_eq!(
        api.storage_at(contract, U256::ZERO, None).await.unwrap(),
        B256::from(U256::from(2))
    );
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

    // Source eligibility is Cancun+, but local Shanghai execution can delete existing storage.
    let (api, _handle) =
        spawn(origin.config().with_hardfork(Some(EthereumHardfork::Shanghai.into()))).await;
    assert_eq!(cached_storage(&api, CONTRACT, U256::ZERO).await, Some(U256::from(2)));

    // Keep the old slot unread: only BAL prefill should add it to the remote snapshot.
    mine_transaction(
        &api,
        TransactionRequest::default()
            .with_from(origin.sender)
            .with_to(CONTRACT)
            .with_input(bytes!("01"))
            .with_gas_limit(200_000),
    )
    .await;
    let deleted_block = BlockId::number(api.block_number().unwrap().to());
    assert!(api.get_code(CONTRACT, None).await.unwrap().is_empty());
    assert_eq!(api.storage_at(CONTRACT, U256::ZERO, None).await.unwrap(), B256::ZERO);

    api.mine_one().await.unwrap();
    assert_eq!(
        api.storage_at(CONTRACT, U256::ZERO, Some(deleted_block)).await.unwrap(),
        B256::ZERO,
    );
}
