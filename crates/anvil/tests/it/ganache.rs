//! tests against local ganache for local debug purposes
#![allow(unused)]
use crate::{
    abi::Greeter,
    init_tracing,
    utils::{http_provider, http_provider_with_signer, ws_provider, ws_provider_with_signer},
};
use alloy_contract::ContractInstance;
use alloy_network::EthereumSigner;
use alloy_primitives::{address, Address, TxKind};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest, WithOtherFields};
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

    let balance = provider.get_balance(Address::random(), BlockId::latest()).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_deploy() {
    let signer: EthereumSigner = ganache_wallet().into();
    let provider = http_provider_with_signer("http://127.0.0.1:8545", signer);

    let greeter_contract_builder = Greeter::deploy_builder(&provider, "Hello World!".to_string());
    let greeter_contract_address = greeter_contract_builder.deploy().await.unwrap();
    let greeter_contract = Greeter::new(greeter_contract_address, &provider);

    let Greeter::greetReturn { _0 } = greeter_contract.greet().call().await.unwrap();
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
    let provider = ws_provider_with_signer("ws://127.0.0.1:8545", signer);

    let first_msg = "First Message".to_string();
    let next_msg = "Next Message".to_string();
    let emit_logs_contract_builder = EmitLogs::deploy_builder(&provider, first_msg.clone());
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

    sol!(
        #[sol(rpc)]
        contract Contract {}
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let wallet = ganache_wallet();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer("http://127.0.0.1:8545", signer);

    // should catch the revert during estimation which results in an err
    let err = provider
        .send_transaction(WithOtherFields::new(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Create),
            input: bytecode.into(),
            ..Default::default()
        }))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("execution reverted"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_ganache_tx_reverting() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r#"
pragma solidity 0.8.13;
contract Contract {
    address owner;
    constructor() public {
        owner = msg.sender;
    }
    modifier onlyOwner() {
        require(msg.sender == owner, "!authorized");
        _;
    }
    function getSecret() public onlyOwner view returns(uint256 secret) {
        return 123;
    }
}
"#,
    )
    .unwrap();

    sol!(
        #[sol(rpc)]
        contract Contract {
            function getSecret() public view returns (uint256);
        }
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let wallet = ganache_wallet();
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
    let contract = Contract::new(contract_address, &provider);

    // should catch the revert during the call which results in an err
    contract.getSecret().send().await.unwrap_err();

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
