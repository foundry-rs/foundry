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
use alloy_sol_types::{sol, Revert};
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
    sol!(
        #[derive(Debug)]
        #[sol(rpc)]
        RevertingConstructor,
        "test-data/RevertingConstructor.json"
    );

    let wallet = ganache_wallet();
    let signer: EthereumSigner = wallet.into();
    let provider = http_provider("http://127.0.0.1:8545");
    let provider_with_signer = http_provider_with_signer("http://127.0.0.1:8545", signer);

    // deploy will fail
    let contract_builder = RevertingConstructor::deploy_builder(&provider_with_signer);
    contract_builder.deploy().await.unwrap_err();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_tx_reverting() {
    sol!(
        #[derive(Debug)]
        #[sol(rpc)]
        RevertingMethod,
        "test-data/RevertingMethod.json"
    );

    let wallet = ganache_wallet();
    let signer: EthereumSigner = wallet.into();
    let provider = http_provider("http://127.0.0.1:8545");
    let provider_with_signer = http_provider_with_signer("http://127.0.0.1:8545", signer);

    // deploy successfully
    let contract_builder = RevertingMethod::deploy_builder(&provider_with_signer);
    let contract_address = contract_builder.deploy().await.unwrap();
    let contract = RevertingMethod::new(contract_address, &provider);
    contract.getSecret().call().await.unwrap_err();

    // /*  Ganache rpc errors look like:
    // <   {
    // <     "id": 1627277502538,
    // <     "jsonrpc": "2.0",
    // <     "error": {
    // <       "message": "VM Exception while processing transaction: revert !authorized",
    // <       "code": -32000,
    // <       "data": {
    // <         "0x90264de254689f1d4e7f8670cd97f60d9bc803874fdecb34d249bd1cc3ca823a": {
    // <           "error": "revert",
    // <           "program_counter": 223,
    // <           "return":
    // "0x08c379a00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000b21617574686f72697a6564000000000000000000000000000000000000000000"
    // , <           "reason": "!authorized"
    // <         },
    // <         "stack": "c: VM Exception while processing transaction: revert !authorized\n    at
    // Function.c.fromResults
    // (/usr/local/lib/node_modules/ganache-cli/build/ganache-core.node.cli.js:4:192416)\n    at
    // /usr/local/lib/node_modules/ganache-cli/build/ganache-core.node.cli.js:42:50402", <
    // "name": "c" <       }
    // <     }
    //  */
}
