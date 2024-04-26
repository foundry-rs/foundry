//! tests against local geth for local debug purposes

use crate::{
    abi::{VendingMachine, VENDING_MACHINE_CONTRACT},
    utils::{http_provider, http_provider_with_signer},
};
use alloy_network::{EthereumSigner, TransactionBuilder};
use alloy_primitives::{Address, TxKind, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{TransactionRequest, WithOtherFields};
use anvil::{spawn, NodeConfig};
use foundry_compilers::{project_util::TempProject, Artifact};
use futures::StreamExt;
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_geth_pending_transaction() {
    let provider = http_provider("http://127.0.0.1:8545");

    let account = provider.get_accounts().await.unwrap().remove(0);

    let tx = TransactionRequest::default()
        .with_from(account)
        .with_to(Address::random())
        .with_value(U256::from(1337u64))
        .with_nonce(2u64);
    let tx = WithOtherFields::new(tx);

    let mut watch_tx_stream = provider.watch_pending_transactions().await.unwrap().into_stream();

    provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await.unwrap();

    let pending = timeout(std::time::Duration::from_secs(3), watch_tx_stream.next()).await;
    pending.unwrap_err();
}

// check how geth returns reverts
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_geth_revert_transaction() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source("VendingMachine", VENDING_MACHINE_CONTRACT).unwrap();

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("VendingMachine").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer("http://127.0.0.1:8545", signer);

    // deploy successfully
    provider
        .send_transaction(WithOtherFields::new(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Create),
            input: bytecode.into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);
    let contract = VendingMachine::new(contract_address, &provider);

    let res = contract
        .buyRevert(U256::from(100))
        .value(U256::from(1))
        .from(sender)
        .send()
        .await
        .unwrap_err();
    let msg = res.to_string();
    assert!(msg.contains("execution reverted: revert: Not enough Ether provided."));
    assert!(msg.contains("code: 3"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_geth_low_gas_limit() {
    let provider = http_provider("http://127.0.0.1:8545");

    let account = provider.get_accounts().await.unwrap().remove(0);

    let gas_limit = 21_000u128 - 1;
    let tx = TransactionRequest::default()
        .from(account)
        .to(Address::random())
        .value(U256::from(1337u64))
        .gas_limit(gas_limit);
    let tx = WithOtherFields::new(tx);

    let resp = provider.send_transaction(tx.clone()).await.unwrap().get_receipt().await;
    let err = resp.unwrap_err().to_string();
    assert!(err.contains("intrinsic gas too low"));
}
