use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_network::{
    AnyNetwork, AnyRpcTransaction, ReceiptResponse, TransactionBuilder, TxSignerSync,
};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, keccak256};
use alloy_provider::{Provider, RootProvider, ext::TxPoolApi};
use alloy_rpc_types::{BlockId, BlockNumberOrTag, BlockTransactions, TransactionRequest};
use alloy_rpc_types_mev::{EthBundleHash, EthSendBundle};
use alloy_signer_local::PrivateKeySigner;
use anvil::{CHAIN_ID, NodeConfig, spawn};
use futures::StreamExt;
use std::time::Duration;
use tokio::time::timeout;

fn signed_transaction(
    signer: &PrivateKeySigner,
    nonce: u64,
    to: Address,
    value: U256,
) -> (Bytes, B256) {
    let mut transaction = TxEip1559 {
        chain_id: CHAIN_ID,
        nonce,
        gas_limit: 100_000,
        max_fee_per_gas: 10_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(to),
        value,
        ..Default::default()
    };
    let signature = signer.sign_transaction_sync(&mut transaction).unwrap();
    let transaction = transaction.into_signed(signature);
    let mut encoded = Vec::new();
    transaction.eip2718_encode(&mut encoded);
    let encoded = Bytes::from(encoded);
    let hash = keccak256(&encoded);
    (encoded, hash)
}

async fn send_bundle(provider: &RootProvider<AnyNetwork>, bundle: EthSendBundle) -> EthBundleHash {
    provider.client().request("eth_sendBundle", (bundle,)).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn mines_private_bundle_in_order() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let first_recipient = wallets[1].address();
    let second_recipient = wallets[2].address();

    let (first, first_hash) = signed_transaction(&wallets[0], 0, first_recipient, U256::from(1));
    let (second, second_hash) = signed_transaction(&wallets[0], 1, second_recipient, U256::from(2));
    let request = EthSendBundle { txs: vec![first, second], block_number: 1, ..Default::default() };
    let expected_hash = request.bundle_hash();
    let mut pending_listener = api.new_ready_transactions();
    assert_eq!(send_bundle(&provider, request).await.bundle_hash, expected_hash);
    assert!(timeout(Duration::from_millis(100), pending_listener.next()).await.is_err());

    let pending: Vec<AnyRpcTransaction> =
        provider.client().request("eth_pendingTransactions", ()).await.unwrap();
    assert!(pending.is_empty());
    assert!(provider.get_transaction_by_hash(first_hash).await.unwrap().is_none());
    assert_eq!(provider.txpool_status().await.unwrap().pending, 0);

    let pending_block =
        provider.get_block(BlockId::Number(BlockNumberOrTag::Pending)).await.unwrap().unwrap();
    assert_eq!(
        pending_block.transactions,
        BlockTransactions::Hashes(vec![first_hash, second_hash])
    );

    api.mine_one().await;
    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.transactions, BlockTransactions::Hashes(vec![first_hash, second_hash]));
    assert_eq!(provider.get_transaction_count(wallets[0].address()).await.unwrap(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_private_bundle_atomically() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let recipient = wallets[1].address();
    let balance = provider.get_balance(recipient).await.unwrap();

    let (first, _) = signed_transaction(&wallets[0], 0, recipient, U256::from(1));
    let (invalid, _) = signed_transaction(&wallets[0], 0, recipient, U256::from(2));
    send_bundle(
        &provider,
        EthSendBundle { txs: vec![first, invalid], block_number: 1, ..Default::default() },
    )
    .await;

    api.mine_one().await;
    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.transactions, BlockTransactions::Hashes(Vec::new()));
    assert_eq!(provider.get_transaction_count(wallets[0].address()).await.unwrap(), 0);
    assert_eq!(provider.get_balance(recipient).await.unwrap(), balance);
}

#[tokio::test(flavor = "multi_thread")]
async fn respects_bundle_constraints_and_replacement() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();

    let (too_early, _) = signed_transaction(&wallets[0], 0, wallets[1].address(), U256::from(1));
    send_bundle(
        &provider,
        EthSendBundle {
            txs: vec![too_early],
            block_number: 1,
            min_timestamp: Some(u64::MAX),
            ..Default::default()
        },
    )
    .await;
    let (expired, _) = signed_transaction(&wallets[0], 0, wallets[2].address(), U256::from(1));
    send_bundle(
        &provider,
        EthSendBundle {
            txs: vec![expired],
            block_number: 1,
            max_timestamp: Some(0),
            ..Default::default()
        },
    )
    .await;

    let replacement_uuid = Some("8a8f311e-2f02-4f2f-a706-1d09b47a3dce".to_string());
    let (replaced, replaced_hash) =
        signed_transaction(&wallets[0], 0, wallets[3].address(), U256::from(1));
    send_bundle(
        &provider,
        EthSendBundle {
            txs: vec![replaced],
            block_number: 2,
            replacement_uuid: replacement_uuid.clone(),
            ..Default::default()
        },
    )
    .await;
    let (replacement, replacement_hash) =
        signed_transaction(&wallets[0], 0, wallets[4].address(), U256::from(1));
    send_bundle(
        &provider,
        EthSendBundle {
            txs: vec![replacement],
            block_number: 2,
            replacement_uuid,
            ..Default::default()
        },
    )
    .await;

    api.mine_one().await;
    let first_block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(first_block.transactions, BlockTransactions::Hashes(Vec::new()));

    api.mine_one().await;
    let second_block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(second_block.transactions, BlockTransactions::Hashes(vec![replacement_hash]));
    assert!(!second_block.transactions.hashes().any(|hash| hash == replaced_hash));
}

#[tokio::test(flavor = "multi_thread")]
async fn permits_listed_reverting_transaction() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let reverting_contract = Address::random();
    api.anvil_set_code(reverting_contract, Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xfd]))
        .await
        .unwrap();

    let (reverting, reverting_hash) =
        signed_transaction(&wallets[0], 0, reverting_contract, U256::ZERO);
    let (success, success_hash) =
        signed_transaction(&wallets[0], 1, wallets[1].address(), U256::from(1));
    send_bundle(
        &provider,
        EthSendBundle {
            txs: vec![reverting, success],
            block_number: 1,
            reverting_tx_hashes: vec![reverting_hash],
            ..Default::default()
        },
    )
    .await;

    api.mine_one().await;
    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.transactions, BlockTransactions::Hashes(vec![reverting_hash, success_hash]));
    assert!(!provider.get_transaction_receipt(reverting_hash).await.unwrap().unwrap().status());
    assert!(provider.get_transaction_receipt(success_hash).await.unwrap().unwrap().status());
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_invalid_bundle_parameters() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let request: Result<EthBundleHash, _> = provider
        .client()
        .request("eth_sendBundle", (EthSendBundle { block_number: 1, ..Default::default() },))
        .await;
    let err = request.unwrap_err();
    assert_eq!(err.as_error_resp().unwrap().code, -32602);
    assert!(err.to_string().contains("at least one transaction"));
}

#[tokio::test(flavor = "multi_thread")]
async fn automines_eligible_private_bundle() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let (transaction, hash) =
        signed_transaction(&wallets[0], 0, wallets[1].address(), U256::from(1));

    send_bundle(
        &provider,
        EthSendBundle { txs: vec![transaction], block_number: 1, ..Default::default() },
    )
    .await;

    timeout(Duration::from_secs(5), async {
        loop {
            if provider.get_transaction_receipt(hash).await.unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn does_not_resurrect_consumed_bundle_after_revert() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let snapshot = api.evm_snapshot().await.unwrap();
    let (transaction, hash) =
        signed_transaction(&wallets[0], 0, wallets[1].address(), U256::from(1));

    send_bundle(
        &provider,
        EthSendBundle { txs: vec![transaction], block_number: 1, ..Default::default() },
    )
    .await;
    api.mine_one().await;
    assert!(provider.get_transaction_receipt(hash).await.unwrap().is_some());

    assert!(api.evm_revert(snapshot).await.unwrap());
    api.mine_one().await;
    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.transactions, BlockTransactions::Hashes(Vec::new()));
}

#[tokio::test(flavor = "multi_thread")]
async fn bundle_can_fund_later_sender() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let funded = PrivateKeySigner::random();
    let funding = U256::from(1_000_000_000_000_000_000u128);
    let (first, first_hash) = signed_transaction(&wallets[0], 0, funded.address(), funding);
    let (second, second_hash) = signed_transaction(&funded, 0, wallets[1].address(), U256::from(1));

    send_bundle(
        &provider,
        EthSendBundle { txs: vec![first, second], block_number: 1, ..Default::default() },
    )
    .await;
    api.mine_one().await;

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.transactions, BlockTransactions::Hashes(vec![first_hash, second_hash]));
    assert!(provider.get_transaction_receipt(second_hash).await.unwrap().unwrap().status());
}

#[tokio::test(flavor = "multi_thread")]
async fn future_bundle_waits_for_next_block_eligibility() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let (transaction, hash) =
        signed_transaction(&wallets[0], 0, wallets[1].address(), U256::from(1));

    send_bundle(
        &provider,
        EthSendBundle { txs: vec![transaction], block_number: 2, ..Default::default() },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(provider.get_block_number().await.unwrap(), 0);

    let public: B256 = provider
        .client()
        .request(
            "eth_sendTransaction",
            (TransactionRequest::default()
                .with_from(wallets[1].address())
                .with_to(wallets[2].address())
                .with_value(U256::from(1)),),
        )
        .await
        .unwrap();

    timeout(Duration::from_secs(5), async {
        loop {
            if provider.get_transaction_receipt(hash).await.unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        provider.get_transaction_receipt(public).await.unwrap().unwrap().block_number(),
        Some(1)
    );
    assert_eq!(
        provider.get_transaction_receipt(hash).await.unwrap().unwrap().block_number(),
        Some(2)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mines_two_bundles_in_one_block() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let (first, first_hash) =
        signed_transaction(&wallets[0], 0, wallets[2].address(), U256::from(1));
    let (second, second_hash) =
        signed_transaction(&wallets[1], 0, wallets[3].address(), U256::from(2));

    send_bundle(
        &provider,
        EthSendBundle { txs: vec![first], block_number: 1, ..Default::default() },
    )
    .await;
    send_bundle(
        &provider,
        EthSendBundle { txs: vec![second], block_number: 1, ..Default::default() },
    )
    .await;
    api.mine_one().await;

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.transactions, BlockTransactions::Hashes(vec![first_hash, second_hash]));
}

#[tokio::test(flavor = "multi_thread")]
async fn orders_bundle_before_public_transaction() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let public_hash: B256 = provider
        .client()
        .request(
            "eth_sendTransaction",
            (TransactionRequest::default()
                .with_from(wallets[1].address())
                .with_to(wallets[3].address())
                .with_value(U256::from(2)),),
        )
        .await
        .unwrap();
    let (private, private_hash) =
        signed_transaction(&wallets[0], 0, wallets[2].address(), U256::from(1));
    send_bundle(
        &provider,
        EthSendBundle { txs: vec![private], block_number: 1, ..Default::default() },
    )
    .await;

    api.mine_one().await;
    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.transactions, BlockTransactions::Hashes(vec![private_hash, public_hash]));
}

#[tokio::test(flavor = "multi_thread")]
async fn drops_allowed_transaction_and_mines_remainder() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let (first, first_hash) =
        signed_transaction(&wallets[0], 0, wallets[1].address(), U256::from(1));
    let (droppable, droppable_hash) =
        signed_transaction(&wallets[0], 0, wallets[2].address(), U256::from(2));
    let (last, last_hash) = signed_transaction(&wallets[0], 1, wallets[3].address(), U256::from(3));

    send_bundle(
        &provider,
        EthSendBundle {
            txs: vec![first, droppable, last],
            block_number: 1,
            dropping_tx_hashes: vec![droppable_hash],
            ..Default::default()
        },
    )
    .await;
    api.mine_one().await;

    let block = provider.get_block(BlockId::latest()).await.unwrap().unwrap();
    assert_eq!(block.transactions, BlockTransactions::Hashes(vec![first_hash, last_hash]));
    assert!(provider.get_transaction_receipt(droppable_hash).await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_state_includes_private_bundle_effects() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    api.anvil_set_auto_mine(false).await.unwrap();
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let recipient = Address::random();
    let value = U256::from(1234);
    let initial_balance = provider.get_balance(recipient).await.unwrap();
    let (transaction, hash) = signed_transaction(&wallets[0], 0, recipient, value);
    send_bundle(
        &provider,
        EthSendBundle { txs: vec![transaction], block_number: 1, ..Default::default() },
    )
    .await;

    let pending_block = provider.get_block(BlockId::pending()).await.unwrap().unwrap();
    assert_eq!(pending_block.transactions, BlockTransactions::Hashes(vec![hash]));
    assert_eq!(
        provider.get_balance(recipient).block_id(BlockId::pending()).await.unwrap(),
        initial_balance + value
    );
    assert_eq!(
        provider
            .get_transaction_count(wallets[0].address())
            .block_id(BlockId::pending())
            .await
            .unwrap(),
        1
    );
}
