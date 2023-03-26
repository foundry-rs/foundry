//! tests against local geth for local debug purposes

use crate::abi::VENDING_MACHINE_CONTRACT;
use ethers::{
    abi::Address,
    contract::{Contract, ContractFactory},
    prelude::{Middleware, TransactionRequest},
    providers::Provider,
    types::U256,
    utils::WEI_IN_ETHER,
};
use ethers_solc::{project_util::TempProject, Artifact};
use futures::StreamExt;
use std::sync::Arc;
use tokio::time::timeout;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_geth_pending_transaction() {
    let client = Provider::try_from("http://127.0.0.1:8545").unwrap();
    let accounts = client.get_accounts().await.unwrap();
    let tx = TransactionRequest::new()
        .from(accounts[0])
        .to(Address::random())
        .value(1337u64)
        .nonce(2u64);

    let mut watch_tx_stream =
        client.watch_pending_transactions().await.unwrap().transactions_unordered(1).fuse();

    let _res = client.send_transaction(tx, None).await.unwrap();

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
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let client = Arc::new(Provider::try_from("http://127.0.0.1:8545").unwrap());

    let account = client.get_accounts().await.unwrap().remove(0);

    // deploy successfully
    let factory =
        ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), Arc::clone(&client));

    let mut tx = factory.deploy(()).unwrap().tx;
    tx.set_from(account);

    let resp = client.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let contract =
        Contract::<Provider<_>>::new(resp.contract_address.unwrap(), abi.unwrap(), client);

    let ten = WEI_IN_ETHER.saturating_mul(10u64.into());
    let call = contract.method::<_, ()>("buyRevert", ten).unwrap().value(ten).from(account);
    let resp = call.call().await;
    let err = resp.unwrap_err().to_string();
    assert!(err.contains("execution reverted: Not enough Ether provided."));
    assert!(err.contains("code: 3"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_geth_low_gas_limit() {
    let provider = Arc::new(Provider::try_from("http://127.0.0.1:8545").unwrap());

    let account = provider.get_accounts().await.unwrap().remove(0);

    let gas = 21_000u64 - 1;
    let tx = TransactionRequest::new()
        .to(Address::random())
        .value(U256::from(1337u64))
        .from(account)
        .gas(gas);

    let resp = provider.send_transaction(tx, None).await;

    let err = resp.unwrap_err().to_string();
    assert!(err.contains("intrinsic gas too low"));
}
