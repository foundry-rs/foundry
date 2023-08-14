//! tests against local ganache for local debug purposes
#![allow(unused)]
use crate::init_tracing;
use ethers::{
    abi::Address,
    contract::{Contract, ContractFactory, ContractInstance},
    core::k256::SecretKey,
    prelude::{abigen, Middleware, Signer, SignerMiddleware, TransactionRequest, Ws},
    providers::{Http, Provider},
    signers::LocalWallet,
    types::{BlockNumber, U256},
    utils::hex,
};
use ethers_solc::{project_util::TempProject, Artifact};
use std::sync::Arc;

// the mnemonic used to start the local ganache instance
const MNEMONIC: &str =
    "amazing discover palace once resource choice flush horn wink shift planet relief";

fn ganache_wallet() -> LocalWallet {
    wallet("552dd2534c4984f892191997d6b1dd9e6a23c7e07b908a6cebfad1d3f2af4c4c")
}

fn ganache_wallet2() -> LocalWallet {
    wallet("305b526d493844b63466be6d48a424ab83f5216011eef860acc6db4c1821adc9")
}

fn wallet(key_str: &str) -> LocalWallet {
    let key_hex = hex::decode(key_str).expect("could not parse as hex");
    let key = SecretKey::from_bytes(key_hex.as_slice().into()).expect("did not get private key");
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
async fn test_ganache_block_number() {
    let client = http_client();
    let balance = client
        .get_balance(Address::random(), Some(BlockNumber::Number(100u64.into()).into()))
        .await;
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
    println!("{compiled}");
    assert!(!compiled.has_compiler_errors());

    let contract = compiled.remove_first("Contract").unwrap();

    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let factory = ContractFactory::new(abi.unwrap(), bytecode.unwrap(), Arc::clone(&client));
    let contract = factory.deploy(()).unwrap().legacy().send().await;
    contract.unwrap_err();
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

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let client = Arc::new(http_client());

    // deploy successfully
    let factory = ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), client);
    let contract = factory.deploy(()).unwrap().legacy().send().await.unwrap();
    let provider = SignerMiddleware::new(
        Provider::<Http>::try_from("http://127.0.0.1:8545").unwrap(),
        ganache_wallet2(),
    );
    let contract = ContractInstance::new(contract.address(), abi.unwrap(), provider);
    let resp = contract.method::<_, U256>("getSecret", ()).unwrap().legacy().call().await;
    resp.unwrap_err();

    /*  Ganache rpc errors look like:
    <   {
    <     "id": 1627277502538,
    <     "jsonrpc": "2.0",
    <     "error": {
    <       "message": "VM Exception while processing transaction: revert !authorized",
    <       "code": -32000,
    <       "data": {
    <         "0x90264de254689f1d4e7f8670cd97f60d9bc803874fdecb34d249bd1cc3ca823a": {
    <           "error": "revert",
    <           "program_counter": 223,
    <           "return": "0x08c379a00000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000b21617574686f72697a6564000000000000000000000000000000000000000000",
    <           "reason": "!authorized"
    <         },
    <         "stack": "c: VM Exception while processing transaction: revert !authorized\n    at Function.c.fromResults (/usr/local/lib/node_modules/ganache-cli/build/ganache-core.node.cli.js:4:192416)\n    at /usr/local/lib/node_modules/ganache-cli/build/ganache-core.node.cli.js:42:50402",
    <         "name": "c"
    <       }
    <     }
     */
}
