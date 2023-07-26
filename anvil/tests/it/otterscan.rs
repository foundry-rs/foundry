//! tests for otterscan endpoints
use crate::abi::MulticallContract;
use anvil::{spawn, NodeConfig};
use ethers::{
    prelude::{Middleware, SignerMiddleware},
    signers::Signer,
    types::BlockNumber,
    utils::get_contract_address,
};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn can_call_erigon_get_header_by_number() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.mine_one().await;

    let res0 = api.erigon_get_header_by_number(0.into()).await.unwrap().unwrap();
    let res1 = api.erigon_get_header_by_number(1.into()).await.unwrap().unwrap();

    assert_eq!(res0.number, Some(0.into()));
    assert_eq!(res1.number, Some(1.into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_api_level() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    assert_eq!(api.ots_get_api_level().await.unwrap(), 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_has_code() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));
    api.mine_one().await;

    let mut deploy_tx = MulticallContract::deploy(Arc::clone(&client), ()).unwrap().deployer.tx;
    deploy_tx.set_nonce(0);

    let pending_contract_address = get_contract_address(sender, deploy_tx.nonce().unwrap());

    // no code in the address before deploying
    assert!(!api
        .ots_has_code(pending_contract_address, BlockNumber::Number(1.into()))
        .await
        .unwrap());

    client.send_transaction(deploy_tx, None).await.unwrap();

    let num = client.get_block_number().await.unwrap();
    // code is detected after deploying
    assert!(api.ots_has_code(pending_contract_address, BlockNumber::Number(num)).await.unwrap());

    // code is not detected for the previous block
    assert!(!api
        .ots_has_code(pending_contract_address, BlockNumber::Number(num - 1))
        .await
        .unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_contract_creator() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();
    api.mine_one().await;

    let wallet = handle.dev_wallets().next().unwrap();
    let sender = wallet.address();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let mut deploy_tx = MulticallContract::deploy(Arc::clone(&client), ()).unwrap().deployer.tx;
    deploy_tx.set_nonce(0);

    let pending_contract_address = get_contract_address(sender, deploy_tx.nonce().unwrap());

    let receipt = client.send_transaction(deploy_tx, None).await.unwrap().await.unwrap().unwrap();

    let creator = api.ots_get_contract_creator(pending_contract_address).await.unwrap().unwrap();

    assert_eq!(creator.creator, sender);
    assert_eq!(creator.hash, receipt.transaction_hash);
}
