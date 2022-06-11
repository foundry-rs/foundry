use crate::next_port;
use anvil::{spawn, NodeConfig};
use ethers::{
    contract::{Contract, ContractFactory},
    middleware::SignerMiddleware,
    types::U256,
};
use ethers_solc::{project_util::TempProject, Artifact};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn test_deploy_reverting() {
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
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.ws_provider().await;

    let wallet = handle.dev_wallets().next().unwrap();
    let client = Arc::new(SignerMiddleware::new(provider, wallet));

    let factory = ContractFactory::new(abi.unwrap(), bytecode.unwrap(), client);
    let contract = factory.deploy(()).unwrap().send().await;
    assert!(contract.is_err());

    // should catch the revert during estimation which results in an err
    let err = contract.unwrap_err();
    assert!(err.to_string().contains("execution reverted"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_revert_messages() {
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
    let contract = compiled.remove("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (_api, handle) = spawn(NodeConfig::test().with_port(next_port())).await;
    let provider = handle.ws_provider().await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let client = Arc::new(SignerMiddleware::new(provider, wallets[0].clone()));

    // deploy successfully
    let factory = ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), client);
    let contract = factory.deploy(()).unwrap().send().await.unwrap();

    let contract = Contract::new(
        contract.address(),
        abi.unwrap(),
        SignerMiddleware::new(handle.http_provider(), wallets[1].clone()),
    );

    let resp = contract.method::<_, U256>("getSecret", ()).unwrap().call().await;

    let err = resp.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("execution reverted: !authorized"));
}
