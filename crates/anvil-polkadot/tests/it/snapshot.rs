use std::time::Duration;

use crate::utils::{EXISTENTIAL_DEPOSIT, TestNode, assert_with_tolerance, unwrap_response};
use alloy_primitives::{Address, U256};
use alloy_rpc_types::TransactionRequest;
use anvil_core::eth::EthRequest;
use anvil_polkadot::{
    api_server::revive_conversions::{AlloyU256, ReviveAddress},
    config::{AnvilNodeConfig, SubstrateNodeConfig},
};
use polkadot_sdk::pallet_revive::{
    self,
    evm::{Account, Block},
};
use subxt::utils::H160;

async fn assert_block_number_is_best_and_finalized(node: &mut TestNode, n: u64) {
    assert_eq!(std::convert::Into::<u64>::into(node.best_block_number().await), n);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let best_block = unwrap_response::<Block>(
        node.eth_rpc(EthRequest::EthGetBlockByNumber(alloy_eips::BlockNumberOrTag::Latest, false))
            .await
            .unwrap(),
    )
    .unwrap();
    let n_as_u256 = pallet_revive::U256::from(n);
    assert_eq!(best_block.number, n_as_u256);

    let finalized_block = unwrap_response::<Block>(
        node.eth_rpc(EthRequest::EthGetBlockByNumber(
            alloy_eips::BlockNumberOrTag::Finalized,
            false,
        ))
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(finalized_block.number, n_as_u256);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_best_block_after_evm_revert() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();

    // Assert on initial best block number.
    assert_block_number_is_best_and_finalized(&mut node, 0).await;

    // Snapshot at genesis.
    let id = unwrap_response::<String>(node.eth_rpc(EthRequest::EvmSnapshot(())).await.unwrap())
        .unwrap();
    assert_eq!(id, "0x0".to_string());

    // Mine 5 blocks and assert on the new best block.
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(Some(U256::from(5)), None)).await.unwrap())
        .unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 5).await;

    // Snapshot at block number 5.
    let id = unwrap_response::<String>(node.eth_rpc(EthRequest::EvmSnapshot(())).await.unwrap())
        .unwrap();
    assert_eq!(id, "0x1".to_string());

    // Mine 5 more blocks.
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(Some(U256::from(5)), None)).await.unwrap())
        .unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 10).await;

    // Snapshot again at block number 10.
    let id = U256::from_str_radix(
        unwrap_response::<String>(node.eth_rpc(EthRequest::EvmSnapshot(())).await.unwrap())
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    assert_eq!(id, U256::from(2));
    assert_block_number_is_best_and_finalized(&mut node, 10).await;

    // Mine 5 more blocks.
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(Some(U256::from(5)), None)).await.unwrap())
        .unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 15).await;

    // Revert to the second snapshot and assert best block number is 10.
    let reverted =
        unwrap_response::<bool>(node.eth_rpc(EthRequest::EvmRevert(id)).await.unwrap()).unwrap();
    assert!(reverted);
    assert_block_number_is_best_and_finalized(&mut node, 10).await;

    // Check mining works fine after reverting.
    unwrap_response::<()>(
        node.eth_rpc(EthRequest::Mine(Some(U256::from(10)), None)).await.unwrap(),
    )
    .unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 20).await;

    // Revert immediatelly after a snapshot (same best number is expected after the revert).
    let id = U256::from_str_radix(
        unwrap_response::<String>(node.eth_rpc(EthRequest::EvmSnapshot(())).await.unwrap())
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    assert_eq!(id, U256::from(3));
    assert_block_number_is_best_and_finalized(&mut node, 20).await;

    let reverted =
        unwrap_response::<bool>(node.eth_rpc(EthRequest::EvmRevert(id)).await.unwrap()).unwrap();
    assert!(reverted);
    assert_block_number_is_best_and_finalized(&mut node, 20).await;

    // Test the case of revert id -> revert same id.
    let reverted =
        unwrap_response::<bool>(node.eth_rpc(EthRequest::EvmRevert(U256::ONE)).await.unwrap())
            .unwrap();
    assert!(reverted);
    assert_block_number_is_best_and_finalized(&mut node, 5).await;

    let reverted =
        unwrap_response::<bool>(node.eth_rpc(EthRequest::EvmRevert(U256::ONE)).await.unwrap())
            .unwrap();
    assert!(!reverted);
    assert_block_number_is_best_and_finalized(&mut node, 5).await;

    // Test reverting down to genesis.
    let reverted =
        unwrap_response::<bool>(node.eth_rpc(EthRequest::EvmRevert(U256::ZERO)).await.unwrap())
            .unwrap();
    assert!(reverted);
    assert_block_number_is_best_and_finalized(&mut node, 0).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_balances_and_txs_index_after_evm_revert() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();

    // Assert on initial best block number.
    assert_block_number_is_best_and_finalized(&mut node, 0).await;

    // Mine 5 blocks and assert on the new best block.
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(Some(U256::from(5)), None)).await.unwrap())
        .unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 5).await;

    // Snapshot at block number 5.
    let id = U256::from_str_radix(
        unwrap_response::<String>(node.eth_rpc(EthRequest::EvmSnapshot(())).await.unwrap())
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    assert_eq!(id, U256::ZERO);

    // Get known accounts initial balances.
    let alith_account = Account::from(subxt_signer::eth::dev::alith());
    let alith_addr = Address::from(ReviveAddress::new(alith_account.address()));
    let baltathar_account = Account::from(subxt_signer::eth::dev::baltathar());
    let baltathar_addr = Address::from(ReviveAddress::new(baltathar_account.address()));
    let alith_initial_balance = node.get_balance(alith_account.address(), None).await;
    let baltathar_initial_balance = node.get_balance(baltathar_account.address(), None).await;

    // Initialize a random account. Assume its initial balance is 0.
    let transfer_amount = U256::from(16e17);
    let (dest_addr, tx_hash) =
        node.eth_transfer_to_unitialized_random_account(alith_addr, transfer_amount, None).await;
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(None, None)).await.unwrap()).unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 6).await;
    let receipt_info = node.get_transaction_receipt(tx_hash).await;

    // Assert on balances after first transfer.
    let alith_balance_after_tx0 = node.get_balance(alith_account.address(), None).await;
    let dest_balance = node.get_balance(H160::from_slice(dest_addr.as_slice()), None).await;
    assert_eq!(
        alith_balance_after_tx0,
        alith_initial_balance
            - AlloyU256::from(receipt_info.effective_gas_price * receipt_info.gas_used).inner()
            - transfer_amount
            - U256::from(EXISTENTIAL_DEPOSIT),
        "alith's balance should have changed"
    );
    assert_eq!(dest_balance, transfer_amount, "dest's balance should have changed");

    let transaction_receipt = node.get_transaction_receipt(tx_hash).await;
    assert_eq!(transaction_receipt.block_number, pallet_revive::U256::from(6));
    assert_eq!(transaction_receipt.transaction_index, pallet_revive::U256::one());
    assert_eq!(transaction_receipt.transaction_hash, tx_hash);

    // Make another regular transfer between known accounts.
    let transfer_amount = U256::from(1e17);
    let transaction =
        TransactionRequest::default().value(transfer_amount).from(baltathar_addr).to(alith_addr);
    let tx_hash = node.send_transaction(transaction, None).await.unwrap();
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(None, None)).await.unwrap()).unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 7).await;

    tokio::time::sleep(Duration::from_millis(500)).await;
    let transaction_receipt = node.get_transaction_receipt(tx_hash).await;

    assert_eq!(transaction_receipt.block_number, pallet_revive::U256::from(7));
    assert_eq!(transaction_receipt.transaction_index, pallet_revive::U256::one());
    assert_eq!(transaction_receipt.transaction_hash, tx_hash);

    let alith_final_balance = node.get_balance(alith_account.address(), None).await;
    let baltathar_final_balance = node.get_balance(baltathar_account.address(), None).await;
    assert_eq!(
        baltathar_final_balance,
        baltathar_initial_balance
            - transfer_amount
            - AlloyU256::from(
                transaction_receipt.effective_gas_price * transaction_receipt.gas_used
            )
            .inner(),
        "Baltathar's balance should have changed"
    );
    assert_eq!(
        alith_final_balance,
        alith_balance_after_tx0 + transfer_amount,
        "Alith's balance should have changed"
    );

    // Revert to a block before the transactions have been mined.
    let reverted =
        unwrap_response::<bool>(node.eth_rpc(EthRequest::EvmRevert(id)).await.unwrap()).unwrap();
    assert!(reverted);
    assert_block_number_is_best_and_finalized(&mut node, 5).await;

    // Assert on accounts balances to be the initial balances.
    let alith_balance = node.get_balance(alith_account.address(), None).await;
    let baltathar_balance = node.get_balance(baltathar_account.address(), None).await;
    let dest_balance = node.get_balance(H160::from_slice(dest_addr.as_slice()), None).await;
    assert_eq!(alith_balance, alith_initial_balance);
    assert_eq!(baltathar_balance, baltathar_initial_balance);
    assert_eq!(dest_balance, U256::ZERO);
    assert_eq!(node.get_nonce(alith_addr).await, U256::ZERO);
    assert_eq!(node.get_nonce(baltathar_addr).await, U256::ZERO);
    assert_eq!(node.get_nonce(dest_addr).await, U256::ZERO);

    // Remine the 6th block with same txs above.
    let transaction =
        TransactionRequest::default().value(U256::from(16e17)).from(alith_addr).to(dest_addr);
    let tx_hash1 = node.send_transaction(transaction, None).await.unwrap();
    let transaction =
        TransactionRequest::default().value(U256::from(1e17)).from(baltathar_addr).to(alith_addr);
    let tx_hash2 = node.send_transaction(transaction, None).await.unwrap();
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(None, None)).await.unwrap()).unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 6).await;

    let receipt_info = node.get_transaction_receipt(tx_hash1).await;
    assert_eq!(receipt_info.block_number, pallet_revive::U256::from(6));
    assert_eq!(receipt_info.transaction_index, pallet_revive::U256::one());
    assert_eq!(receipt_info.transaction_hash, tx_hash1);
    let receipt_info = node.get_transaction_receipt(tx_hash2).await;
    assert_eq!(receipt_info.block_number, pallet_revive::U256::from(6));
    assert_eq!(receipt_info.transaction_index, pallet_revive::U256::from(2));
    assert_eq!(receipt_info.transaction_hash, tx_hash2);
    assert_eq!(node.get_nonce(alith_addr).await, U256::ONE);
    assert_eq!(node.get_nonce(baltathar_addr).await, U256::ONE);
    assert_eq!(node.get_nonce(dest_addr).await, U256::ZERO);

    let txs_in_block = unwrap_response::<U256>(
        node.eth_rpc(EthRequest::EthGetTransactionCountByNumber(
            alloy_eips::BlockNumberOrTag::Latest,
        ))
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(txs_in_block, U256::from(2));
}

#[tokio::test(flavor = "multi_thread")]
// TODO: add a test where we call a contract that queries the timestamp
// at a certain block before and after a revert, while mining blocks
async fn test_evm_revert_and_timestamp() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    // Generate the current timestamp and pass it to anvil config.
    let genesis_timestamp = anvil_node_config.get_genesis_timestamp();
    let anvil_node_config = anvil_node_config.with_genesis_timestamp(Some(genesis_timestamp));
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();

    // Do a first snapshot for genesis.
    let id = unwrap_response::<String>(node.eth_rpc(EthRequest::EvmSnapshot(())).await.unwrap())
        .unwrap();
    assert_eq!(id, "0x0".to_string());

    // Assert on first best block number.
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(None, None)).await.unwrap()).unwrap();
    assert_eq!(node.best_block_number().await, 1);
    let first_timestamp = node.get_decoded_timestamp(None).await;
    assert_with_tolerance(
        first_timestamp.saturating_div(1000),
        genesis_timestamp,
        0,
        "wrong timestamp at first block",
    );

    let second_timestamp = first_timestamp.saturating_add(3000);
    assert_with_tolerance(
        unwrap_response::<u64>(
            node.eth_rpc(EthRequest::EvmSetTime(U256::from(second_timestamp.saturating_div(1000))))
                .await
                .unwrap(),
        )
        .unwrap(),
        3,
        1,
        "Wrong offset 1",
    );

    // Mine 1 blocks and assert on the new best block.
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(None, None)).await.unwrap()).unwrap();
    assert_eq!(node.best_block_number().await, 2);
    let second_timestamp = node.get_decoded_timestamp(None).await;
    assert_with_tolerance(
        second_timestamp.saturating_sub(first_timestamp),
        3000,
        150,
        "wrong timestamp at second block",
    );
    // Snapshot at block number 2 and then mine 1 more block.
    let id = U256::from_str_radix(
        unwrap_response::<String>(node.eth_rpc(EthRequest::EvmSnapshot(())).await.unwrap())
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    assert_eq!(id, U256::ONE);

    // Seconds
    let third_timestamp = second_timestamp.saturating_add(3000);
    assert_with_tolerance(
        unwrap_response::<u64>(
            node.eth_rpc(EthRequest::EvmSetTime(U256::from(third_timestamp.saturating_div(1000))))
                .await
                .unwrap(),
        )
        .unwrap(),
        3,
        1,
        "Wrong offset 2",
    );

    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(None, None)).await.unwrap()).unwrap();
    assert_eq!(node.best_block_number().await, 3);
    let third_timestamp = node.get_decoded_timestamp(None).await;
    assert_with_tolerance(
        third_timestamp.saturating_sub(second_timestamp),
        3000,
        100,
        "wrong timestamp at third block",
    );

    // Revert to block number 2.
    let reverted =
        unwrap_response::<bool>(node.eth_rpc(EthRequest::EvmRevert(id)).await.unwrap()).unwrap();
    assert!(reverted);
    assert_eq!(node.best_block_number().await, 2);
    let seconds_ts_after_revert = node.get_decoded_timestamp(None).await;
    assert_with_tolerance(
        seconds_ts_after_revert.saturating_sub(second_timestamp),
        0,
        5,
        "wrong timestamp at reverted second block",
    );

    // Mine again 1 block and check again the timestamp. We should have the next block timestamp
    // with 1 second later than the second block timestamp.
    tokio::time::sleep(Duration::from_secs(1)).await;
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(None, None)).await.unwrap()).unwrap();
    assert_eq!(node.best_block_number().await, 3);
    let remined_third_block_ts = node.get_decoded_timestamp(None).await;
    assert_with_tolerance(
        remined_third_block_ts.saturating_sub(second_timestamp),
        1000,
        100,
        "wrong timestamp at remined third block",
    );

    // Revert to genesis block number.
    let reverted =
        unwrap_response::<bool>(node.eth_rpc(EthRequest::EvmRevert(U256::ZERO)).await.unwrap())
            .unwrap();
    assert!(reverted);
    assert_eq!(node.best_block_number().await, 0);
    let reverted_genesis_block_ts = node.get_decoded_timestamp(None).await;
    assert_with_tolerance(
        reverted_genesis_block_ts.saturating_div(1000),
        genesis_timestamp,
        0,
        "wrong timestamp at reverted genesis block",
    );

    // Mine 1 block and check the timestamp. We don't check on a specific
    // timestamp, but expect the time has increased a bit since the revert, which set the time back
    // to genesis timestamp.
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(None, None)).await.unwrap()).unwrap();
    assert_eq!(node.best_block_number().await, 1);
    let remined_first_block_ts = node.get_decoded_timestamp(None).await;
    // Here assert that the time is increasing.
    assert!(remined_first_block_ts > genesis_timestamp * 1000);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_rollback() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();

    // Assert on initial best block number.
    assert_block_number_is_best_and_finalized(&mut node, 0).await;

    // Mine 5 blocks and assert on the new best block.
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(Some(U256::from(5)), None)).await.unwrap())
        .unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 5).await;

    // Rollback 2 blocks.
    unwrap_response::<()>(node.eth_rpc(EthRequest::Rollback(Some(2))).await.unwrap()).unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 3).await;

    // Check mining works fine after reverting.
    unwrap_response::<()>(
        node.eth_rpc(EthRequest::Mine(Some(U256::from(10)), None)).await.unwrap(),
    )
    .unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 13).await;

    // Rollback 1 blocks.
    unwrap_response::<()>(node.eth_rpc(EthRequest::Rollback(None)).await.unwrap()).unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 12).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mine_with_txs_in_mempool_before_revert() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();

    // Assert on initial best block number.
    assert_block_number_is_best_and_finalized(&mut node, 0).await;

    // Mine 5 blocks and assert on the new best block.
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(Some(U256::from(5)), None)).await.unwrap())
        .unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 5).await;

    // Snapshot at block number 5.
    let id = U256::from_str_radix(
        unwrap_response::<String>(node.eth_rpc(EthRequest::EvmSnapshot(())).await.unwrap())
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    assert_eq!(id, U256::ZERO);

    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(Some(U256::from(5)), None)).await.unwrap())
        .unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 10).await;

    // Get known accounts.
    let alith_account = Account::from(subxt_signer::eth::dev::alith());
    let alith_addr = Address::from(ReviveAddress::new(alith_account.address()));
    let baltathar_account = Account::from(subxt_signer::eth::dev::baltathar());
    let baltathar_addr = Address::from(ReviveAddress::new(baltathar_account.address()));

    // Initialize a random account.
    let transfer_amount = U256::from(16e17);
    let (dest_addr, _) =
        node.eth_transfer_to_unitialized_random_account(alith_addr, transfer_amount, None).await;

    // Make another regular transfer between known accounts.
    let transfer_amount = U256::from(1e17);
    let transaction =
        TransactionRequest::default().value(transfer_amount).from(baltathar_addr).to(alith_addr);
    let _ = node.send_transaction(transaction, None).await.unwrap();

    // Revert to a block before the transactions have been sent.
    let reverted =
        unwrap_response::<bool>(node.eth_rpc(EthRequest::EvmRevert(id)).await.unwrap()).unwrap();
    assert!(reverted);
    assert_block_number_is_best_and_finalized(&mut node, 5).await;
    let id = U256::from_str_radix(
        unwrap_response::<String>(node.eth_rpc(EthRequest::EvmSnapshot(())).await.unwrap())
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    assert_eq!(id, U256::ONE);

    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(None, None)).await.unwrap()).unwrap();
    assert_block_number_is_best_and_finalized(&mut node, 6).await;

    let txs_in_block = unwrap_response::<U256>(
        node.eth_rpc(EthRequest::EthGetTransactionCountByNumber(
            alloy_eips::BlockNumberOrTag::Latest,
        ))
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(txs_in_block, U256::from(2));

    // Now make two more txs again with same senders, with different nonces than the actual
    // accounts nonces at block 5.
    let transfer_amount = U256::from(1e15);
    let transaction =
        TransactionRequest::default().value(transfer_amount).from(baltathar_addr).to(alith_addr);
    let _ = node.send_transaction(transaction, None).await.unwrap();
    let _ = node
        .send_transaction(
            TransactionRequest::default().value(transfer_amount).from(alith_addr).to(dest_addr),
            None,
        )
        .await
        .unwrap();

    let reverted =
        unwrap_response::<bool>(node.eth_rpc(EthRequest::EvmRevert(id)).await.unwrap()).unwrap();
    assert!(reverted);
    assert_block_number_is_best_and_finalized(&mut node, 5).await;

    let txs_in_block = unwrap_response::<U256>(
        node.eth_rpc(EthRequest::EthGetTransactionCountByNumber(
            alloy_eips::BlockNumberOrTag::Latest,
        ))
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(txs_in_block, U256::ZERO);
}
