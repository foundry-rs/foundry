//! tests against local ganache for local debug purposes
#![allow(unused)]
use crate::init_tracing;
use ethers::{
    contract::ContractFactory,
    core::k256::SecretKey,
    prelude::{abigen, Middleware, Signer, SignerMiddleware, TransactionRequest, Ws},
    providers::{Http, Provider},
    signers::LocalWallet,
    utils::hex,
};
use ethers_solc::{project_util::TempProject, Artifact};
use std::sync::Arc;

fn ganache_wallet() -> LocalWallet {
    let key_str = "3cb6fcd4261b21b2398fdbdc2b589d3b9aca4a63a6a40c8ab28773c1406b842b";
    let key_hex = hex::decode(key_str).expect("could not parse as hex");
    let key = SecretKey::from_be_bytes(&key_hex).expect("did not get private key");
    key.into()
}

fn http_client() -> Arc<SignerMiddleware<Provider<Http>, LocalWallet>> {
    let provider = Provider::<Http>::try_from("http://127.0.0.1:8545").unwrap();
    Arc::new(SignerMiddleware::new(provider, ganache_wallet()))
}

async fn ws_client() -> Arc<SignerMiddleware<Provider<Ws>, LocalWallet>> {
    let provider = Provider::<Ws>::connect("ws://127.0.0.1:8545").await.unwrap();
    Arc::new(SignerMiddleware::new(provider, ganache_wallet()))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_deploy() {
    abigen!(Greeter, "test-data/greeter.json");
    let client = http_client();

    let greeter_contract =
        Greeter::deploy(client, "Hello World!".to_string()).unwrap().legacy().send().await.unwrap();

    let greeting = greeter_contract.greet().call().await.unwrap();
    assert_eq!("Hello World!", greeting);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_emit_logs() {
    abigen!(EmitLogs, "test-data/emit_logs.json");
    let client = ws_client().await;

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

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_deploy_reverting() {
    let client = http_client();

    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r#"
pragma solidity 0.8.13;
contract Contract {
    constructor() {
      require(false, "");
    }
}
"#,
    )
    .unwrap();

    let mut compiled = prj.compile().unwrap();
    println!("{}", compiled);
    assert!(!compiled.has_compiler_errors());

    let contract = compiled.remove("Contract").unwrap();

    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let factory = ContractFactory::new(abi.unwrap(), bytecode.unwrap(), Arc::clone(&client));
    let contract = factory.deploy(()).unwrap().legacy().send().await;
    assert!(contract.is_err());
}
