use std::time::Duration;

use crate::utils::{BlockWaitTimeout, TestNode, unwrap_response};
use alloy_primitives::{Address, U256};
use alloy_rpc_types::TransactionRequest;
use anvil_core::eth::EthRequest;
use anvil_polkadot::{
    api_server::revive_conversions::{AlloyU256, ReviveAddress},
    config::{AnvilNodeConfig, SubstrateNodeConfig},
};
use polkadot_sdk::pallet_revive::{
    self,
    evm::{Account, HashesOrTransactionInfos},
};
use subxt::utils::H160;

#[tokio::test(flavor = "multi_thread")]
async fn test_get_chain_id() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();

    // expected 420420420
    assert_eq!(
        unwrap_response::<String>(node.eth_rpc(EthRequest::EthChainId(())).await.unwrap()).unwrap(),
        "0x190f1b44"
    );
    // expected 420420420
    assert_eq!(
        unwrap_response::<u64>(node.eth_rpc(EthRequest::EthNetworkId(())).await.unwrap()).unwrap(),
        0x190f1b44
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_start_balance() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();

    assert_eq!(
        node.get_balance(
            H160::from_slice(subxt_signer::eth::dev::alith().public_key().to_account_id().as_ref()),
            None
        )
        .await,
        U256::from_str_radix("100000000000000000000000", 10).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_block_by_hash() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();

    let alith = Account::from(subxt_signer::eth::dev::alith());
    let baltathar = Account::from(subxt_signer::eth::dev::baltathar());
    let transfer_amount = U256::from_str_radix("100000000000000000", 10).unwrap();
    let alith_addr = Address::from(ReviveAddress::new(alith.address()));
    let baltathar_addr = Address::from(ReviveAddress::new(baltathar.address()));
    let transaction =
        TransactionRequest::default().value(transfer_amount).from(alith_addr).to(baltathar_addr);
    let tx_hash0 = node.send_transaction(transaction.clone(), None).await.unwrap();
    let tx_hash1 = node.send_transaction(transaction.clone().nonce(1), None).await.unwrap();
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(None, None)).await.unwrap()).unwrap();

    let tx_hash2 = node.send_transaction(transaction.nonce(2), None).await.unwrap();
    unwrap_response::<()>(node.eth_rpc(EthRequest::Mine(None, None)).await.unwrap()).unwrap();

    let hash1 = node.block_hash_by_number(1).await.unwrap();
    let hash2 = node.block_hash_by_number(2).await.unwrap();
    let block1 = node.get_block_by_hash(hash1).await;
    let block2 = node.get_block_by_hash(hash2).await;
    if let HashesOrTransactionInfos::Hashes(transactions) = block1.transactions {
        assert!(transactions.contains(&tx_hash0));
        assert!(transactions.contains(&tx_hash1));
    }
    if let HashesOrTransactionInfos::Hashes(transactions) = block2.transactions {
        assert!(transactions.contains(&tx_hash2));
        assert_eq!(transactions.len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_transaction() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();
    unwrap_response::<()>(node.eth_rpc(EthRequest::SetAutomine(true)).await.unwrap()).unwrap();

    let alith = Account::from(subxt_signer::eth::dev::alith());
    let baltathar = Account::from(subxt_signer::eth::dev::baltathar());
    let alith_initial_balance = node.get_balance(alith.address(), None).await;
    let baltathar_initial_balance = node.get_balance(baltathar.address(), None).await;
    let transfer_amount = U256::from_str_radix("100000000000000000", 10).unwrap();
    let transaction = TransactionRequest::default()
        .value(transfer_amount)
        .from(Address::from(ReviveAddress::new(alith.address())))
        .to(Address::from(ReviveAddress::new(baltathar.address())));
    let tx_hash = node
        .send_transaction(transaction, Some(BlockWaitTimeout::new(1, Duration::from_secs(1))))
        .await
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let transaction_receipt = node.get_transaction_receipt(tx_hash).await;

    assert_eq!(transaction_receipt.block_number, pallet_revive::U256::from(1));
    assert_eq!(transaction_receipt.transaction_index, pallet_revive::U256::from(1));
    assert_eq!(transaction_receipt.transaction_hash, tx_hash);

    let alith_final_balance = node.get_balance(alith.address(), None).await;
    let baltathar_final_balance = node.get_balance(baltathar.address(), None).await;
    assert_eq!(
        baltathar_final_balance,
        baltathar_initial_balance + transfer_amount,
        "Baltathar's balance should have changed"
    );
    assert_eq!(
        alith_final_balance,
        alith_initial_balance
            - transfer_amount
            - AlloyU256::from(
                transaction_receipt.effective_gas_price * transaction_receipt.gas_used
            )
            .inner(),
        "Alith's balance should have changed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_to_uninitialized() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();
    unwrap_response::<()>(node.eth_rpc(EthRequest::SetAutomine(true)).await.unwrap()).unwrap();

    let alith = Account::from(subxt_signer::eth::dev::alith());
    let charleth = Account::from(subxt_signer::eth::dev::charleth());

    let transfer_amount = U256::from_str_radix("1600000000000000000", 10).unwrap();
    let alith_addr = Address::from(ReviveAddress::new(alith.address()));
    let charleth_addr = Address::from(ReviveAddress::new(charleth.address()));
    let transaction =
        TransactionRequest::default().value(transfer_amount).from(alith_addr).to(charleth_addr);
    let _tx_hash = node
        .send_transaction(transaction, Some(BlockWaitTimeout::new(1, Duration::from_secs(1))))
        .await
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));

    let alith_final_balance = node.get_balance(alith.address(), None).await;
    assert_eq!(node.get_balance(charleth.address(), None).await, transfer_amount);

    let charlet_initial_balance = node.get_balance(charleth.address(), None).await;
    let transfer_amount = U256::from_str_radix("100000000000", 10).unwrap();
    let transaction =
        TransactionRequest::default().value(transfer_amount).from(charleth_addr).to(alith_addr);
    let tx_hash = node
        .send_transaction(transaction, Some(BlockWaitTimeout::new(2, Duration::from_secs(1))))
        .await
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let transaction_receipt = node.get_transaction_receipt(tx_hash).await;
    let alith_final_balance_2 = node.get_balance(alith.address(), None).await;
    let charlet_final_balance = node.get_balance(charleth.address(), None).await;
    assert_eq!(
        charlet_final_balance,
        charlet_initial_balance
            - transfer_amount
            - AlloyU256::from(
                transaction_receipt.gas_used * transaction_receipt.effective_gas_price
            )
            .inner()
    );
    assert_eq!(alith_final_balance_2, alith_final_balance + transfer_amount);
}
