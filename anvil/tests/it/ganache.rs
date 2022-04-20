//! tests against local ganache for local debug purposes
#![allow(unused)]
use ethers::{
    core::k256::SecretKey,
    prelude::{abigen, Middleware, Signer, SignerMiddleware, TransactionRequest},
    providers::{Http, Provider},
    signers::LocalWallet,
    utils::hex,
};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_deploy() {
    abigen!(Greeter, "test-data/greeter.json");

    let key_str = "d315e51b743a40a663947684eb6e32f106e0c58b6fd7ffb068202eab3744b88e";
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
