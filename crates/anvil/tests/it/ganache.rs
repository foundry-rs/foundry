//! tests against local ganache for local debug purposes
#![allow(unused)]
use crate::{
    abi::AlloyGreeter,
    init_tracing,
    utils::{
        http_provider, http_provider_with_signer, ws_provider, ws_provider_with_signer,
        ContractInstanceCompat, DeploymentTxFactoryCompat,
    },
};
use alloy_contract::ContractInstance;
use alloy_network::EthereumSigner;
use alloy_primitives::{address, Address};
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use alloy_signer_wallet::{LocalWallet, MnemonicBuilder};
use alloy_sol_types::sol;
use foundry_compilers::{project_util::TempProject, Artifact};
use std::{str::FromStr, sync::Arc};

// the mnemonic used to start the local ganache instance
const MNEMONIC: &str =
    "amazing discover palace once resource choice flush horn wink shift planet relief";

fn ganache_wallet() -> LocalWallet {
    LocalWallet::from_str("552dd2534c4984f892191997d6b1dd9e6a23c7e07b908a6cebfad1d3f2af4c4c")
        .unwrap()
}

fn ganache_wallet2() -> LocalWallet {
    LocalWallet::from_str("305b526d493844b63466be6d48a424ab83f5216011eef860acc6db4c1821adc9")
        .unwrap()
}

fn wallet(key_str: &str) -> LocalWallet {
    LocalWallet::from_str(key_str).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_block_number() {
    let provider = http_provider("http://127.0.0.1:8545");

    let balance = provider.get_balance(Address::random(), Some(BlockId::number(100)).into()).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_deploy() {
    let signer: EthereumSigner = ganache_wallet().into();
    let provider = http_provider("http://127.0.0.1:8545");
    let provider_with_signer = http_provider_with_signer("http://127.0.0.1:8545", signer);

    let greeter_contract_builder =
        AlloyGreeter::deploy_builder(&provider_with_signer, "Hello World!".to_string());
    let greeter_contract_address = greeter_contract_builder.deploy().await.unwrap();
    let greeter_contract = AlloyGreeter::new(greeter_contract_address, &provider);

    let AlloyGreeter::greetReturn { _0 } = greeter_contract.greet().call().await.unwrap();
    let greeting = _0;
    assert_eq!("Hello World!", greeting);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_emit_logs() {
    sol!(
        #[sol(rpc)]
        EmitLogs,
        "test-data/emit_logs.json"
    );

    let signer: EthereumSigner = ganache_wallet().into();
    let provider = ws_provider("ws://127.0.0.1:8545");
    let provider_with_signer = ws_provider_with_signer("ws://127.0.0.1:8545", signer);

    let first_msg = "First Message".to_string();
    let next_msg = "Next Message".to_string();
    let emit_logs_contract_builder =
        EmitLogs::deploy_builder(&provider_with_signer, first_msg.clone());
    let emit_logs_contract_address = emit_logs_contract_builder.deploy().await.unwrap();
    let emit_logs_contract = EmitLogs::new(emit_logs_contract_address, &provider);

    let EmitLogs::getValueReturn { _0 } = emit_logs_contract.getValue().call().await.unwrap();
    let val = _0;
    assert_eq!(val, first_msg);

    emit_logs_contract.setValue(next_msg.clone()).send().await.unwrap();

    let EmitLogs::getValueReturn { _0 } = emit_logs_contract.getValue().call().await.unwrap();
    let val = _0;
    assert_eq!(val, next_msg);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_deploy_reverting() {
    let signer: EthereumSigner = ganache_wallet().into();
    let provider = http_provider("http://127.0.0.1:8545");
    let provider_with_signer = http_provider_with_signer("http://127.0.0.1:8545", signer);

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
    println!("{compiled}");
    assert!(!compiled.has_compiler_errors());

    let contract_artifact = compiled.remove_first("Contract").unwrap();

    let (abi, bytecode, _) = contract_artifact.into_contract_bytecode().into_parts();

    // let contract =
    //     ContractInstanceCompat::new(abi.unwrap(), bytecode.unwrap(), provider_with_signer);
    // contract.deploy(()).unwrap().send().await.unwrap_err();

    // let contract_builder = ContractInstance::deploy_builder(&provider_with_signer)
    //     .abi(abi.unwrap())
    //     .bytecode(bytecode.unwrap());
}
