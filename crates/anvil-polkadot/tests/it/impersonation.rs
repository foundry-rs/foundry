use std::time::Duration;

use crate::utils::{BlockWaitTimeout, TestNode, unwrap_response};
use alloy_primitives::{Address, U256};
use alloy_rpc_types::TransactionRequest;
use anvil_core::eth::EthRequest;
use anvil_polkadot::{
    api_server::revive_conversions::{AlloyU256, ReviveAddress},
    config::{AnvilNodeConfig, SubstrateNodeConfig},
};
use anvil_rpc::error::ErrorCode;
use polkadot_sdk::pallet_revive::evm::Account;
use rstest::rstest;
use subxt::utils::H160;

#[tokio::test(flavor = "multi_thread")]
async fn test_impersonate_account() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();

    // Enable automine.
    unwrap_response::<()>(node.eth_rpc(EthRequest::SetAutomine(true)).await.unwrap()).unwrap();

    // Create a random account.
    let alith_account = Account::from(subxt_signer::eth::dev::alith());
    let alith_addr = Address::from(ReviveAddress::new(alith_account.address()));
    let transfer_amount = U256::from(16e17);
    let (dest_addr, _) = node
        .eth_transfer_to_unitialized_random_account(
            alith_addr,
            transfer_amount,
            Some(BlockWaitTimeout::new(1, Duration::from_secs(1))),
        )
        .await;
    let dest_h160 = H160::from_slice(dest_addr.as_slice());

    // Impersonate destination
    unwrap_response::<()>(node.eth_rpc(EthRequest::ImpersonateAccount(dest_addr)).await.unwrap())
        .unwrap();
    let transfer_amount = U256::from(1e11);
    let alith_balance = node.get_balance(alith_account.address(), None).await;
    let dest_balance = node.get_balance(dest_h160, None).await;
    let transaction =
        TransactionRequest::default().value(transfer_amount).from(dest_addr).to(alith_addr);
    let tx_hash = node
        .send_transaction(transaction, Some(BlockWaitTimeout::new(2, Duration::from_secs(1))))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let receipt_info = node.get_transaction_receipt(tx_hash).await;

    // Assert on balances after second transfer.
    let alith_final_balance = node.get_balance(alith_account.address(), None).await;
    let dest_final_balance = node.get_balance(dest_h160, None).await;
    assert_eq!(alith_final_balance, alith_balance + transfer_amount);
    assert_eq!(
        dest_final_balance,
        dest_balance
            - transfer_amount
            - AlloyU256::from(receipt_info.effective_gas_price * receipt_info.gas_used).inner()
    );

    // Stop impersonating destination, and assert on error when retrying the same transfer.
    unwrap_response::<()>(
        node.eth_rpc(EthRequest::StopImpersonatingAccount(dest_addr)).await.unwrap(),
    )
    .unwrap();
    let transaction =
        TransactionRequest::default().value(transfer_amount).from(dest_addr).to(alith_addr);
    let err = node.send_transaction(transaction.clone(), None).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InternalError);
    assert!(err.message.contains(
        format!("Account not found for address {}", dest_addr.to_string().to_lowercase()).as_str()
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[rstest]
#[case(false)]
#[case(true)]
async fn test_auto_impersonate(#[case] rpc_driven: bool) {
    let mut anvil_node_config = AnvilNodeConfig::test_config();
    if !rpc_driven {
        // Enable autoimpersonation via cli.
        anvil_node_config.enable_auto_impersonate = true;
    }

    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();

    // Enable automine.
    unwrap_response::<()>(node.eth_rpc(EthRequest::SetAutomine(true)).await.unwrap()).unwrap();

    let alith_account = Account::from(subxt_signer::eth::dev::alith());
    let alith_addr = Address::from(ReviveAddress::new(alith_account.address()));
    let transfer_amount = U256::from(16e17);
    let (dest_addr, _) = node
        .eth_transfer_to_unitialized_random_account(
            alith_addr,
            transfer_amount,
            Some(BlockWaitTimeout::new(1, Duration::from_secs(1))),
        )
        .await;

    // Start impersonating any address now
    if rpc_driven {
        unwrap_response::<()>(
            node.eth_rpc(EthRequest::AutoImpersonateAccount(true)).await.unwrap(),
        )
        .unwrap();
    }

    // Transfer at block 2.
    let dest_h160 = H160::from_slice(dest_addr.as_slice());
    let alith_balance = node.get_balance(alith_account.address(), None).await;
    let dest_balance = node.get_balance(dest_h160, None).await;
    let transfer_amount = U256::from(1e11);
    let transaction =
        TransactionRequest::default().value(transfer_amount).from(dest_addr).to(alith_addr);
    let tx_hash = node
        .send_transaction(transaction, Some(BlockWaitTimeout::new(2, Duration::from_secs(1))))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let receipt_info = node.get_transaction_receipt(tx_hash).await;

    // Assert on balances after third transfer.
    let alith_final_balance = node.get_balance(alith_account.address(), None).await;
    let dest_final_balance = node.get_balance(dest_h160, None).await;
    assert_eq!(alith_final_balance, alith_balance + transfer_amount);
    assert_eq!(
        dest_final_balance,
        dest_balance
            - transfer_amount
            - AlloyU256::from(receipt_info.effective_gas_price * receipt_info.gas_used).inner(),
    );

    // Stop impersonating destination, and assert on error when retrying the same transfer.
    unwrap_response::<()>(node.eth_rpc(EthRequest::AutoImpersonateAccount(false)).await.unwrap())
        .unwrap();
    let transaction =
        TransactionRequest::default().value(transfer_amount).from(dest_addr).to(alith_addr);
    let err = node.send_transaction(transaction.clone(), None).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InternalError);
    assert!(err.message.contains(
        format!("Account not found for address {}", dest_addr.to_string().to_lowercase()).as_str()
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_send_unsigned_tx() {
    let anvil_node_config = AnvilNodeConfig::test_config();
    let substrate_node_config = SubstrateNodeConfig::new(&anvil_node_config);
    let mut node = TestNode::new(anvil_node_config.clone(), substrate_node_config).await.unwrap();

    // Enable automine.
    unwrap_response::<()>(node.eth_rpc(EthRequest::SetAutomine(true)).await.unwrap()).unwrap();

    // Create a random account.
    let alith_account = Account::from(subxt_signer::eth::dev::alith());
    let alith_addr = Address::from(ReviveAddress::new(alith_account.address()));
    let transfer_amount = U256::from(16e17);
    let (dest_addr, _) = node
        .eth_transfer_to_unitialized_random_account(
            alith_addr,
            transfer_amount,
            Some(BlockWaitTimeout::new(1, Duration::from_secs(1))),
        )
        .await;
    let dest_h160 = H160::from_slice(dest_addr.as_slice());

    // Impersonate destination
    let transfer_amount = U256::from(1e11);
    let alith_balance = node.get_balance(alith_account.address(), None).await;
    let dest_balance = node.get_balance(dest_h160, None).await;
    let transaction =
        TransactionRequest::default().value(transfer_amount).from(dest_addr).to(alith_addr);
    let tx_hash = node
        .send_unsigned_transaction(
            transaction,
            Some(BlockWaitTimeout::new(2, Duration::from_secs(1))),
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let receipt_info = node.get_transaction_receipt(tx_hash).await;

    // Assert on balances after second transfer.
    let alith_final_balance = node.get_balance(alith_account.address(), None).await;
    let dest_final_balance = node.get_balance(dest_h160, None).await;
    assert_eq!(alith_final_balance, alith_balance + transfer_amount);
    assert_eq!(
        dest_final_balance,
        dest_balance
            - transfer_amount
            - AlloyU256::from(receipt_info.effective_gas_price * receipt_info.gas_used).inner()
    );
}
