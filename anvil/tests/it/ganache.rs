//! tests against local ganache for local debug purposes
#![allow(unused)]
use crate::init_tracing;
use ethers::{
    core::k256::SecretKey,
    prelude::{abigen, Middleware, Signer, SignerMiddleware, TransactionRequest, Ws},
    providers::{Http, Provider},
    signers::LocalWallet,
    utils::hex,
};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_deploy() {
    abigen!(Greeter, "test-data/greeter.json");

    let key_str = "d3631fc655238848b503f1d0cf0e0d4deabed6b5e8b2dfb4dc0c9346f5f3aac1";
    let key_hex = hex::decode(key_str).expect("could not parse as hex");
    let key = SecretKey::from_be_bytes(&key_hex).expect("did not get private key");
    let wallet: LocalWallet = key.into();

    let provider = Provider::<Http>::try_from("http://127.0.0.1:8545").unwrap();

    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let greeter_contract =
        Greeter::deploy(client, "Hello World!".to_string()).unwrap().legacy().send().await.unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_emit_logs() {
    abigen!(EmitLogs, "test-data/emit_logs.json");

    let key_str = "f3ce4cb221f6784021f3b7edfd6f8ac0c2c568cd7eb2b36f827449975489d234";
    let key_hex = hex::decode(key_str).expect("could not parse as hex");
    let key = SecretKey::from_be_bytes(&key_hex).expect("did not get private key");
    let wallet: LocalWallet = key.into();

    let provider = Provider::<Ws>::connect("ws://127.0.0.1:8545").await.unwrap();

    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let msg = "First Message".to_string();
    let contract =
        EmitLogs::deploy(Arc::clone(&client), msg.clone()).unwrap().legacy().send().await.unwrap();

    let val = contract.get_value().call().await.unwrap();
    assert_eq!(val, msg);

    let val = contract
        .set_value("Next Message".to_string())
        .legacy()
        .send()
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();
}
