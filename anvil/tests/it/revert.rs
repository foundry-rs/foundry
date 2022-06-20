use crate::next_port;
use anvil::{spawn, NodeConfig};
use ethers::{
    contract::{Contract, ContractFactory},
    middleware::SignerMiddleware,
    types::U256,
    utils::WEI_IN_ETHER,
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

// <https://docs.soliditylang.org/en/latest/control-structures.html#revert>
pub(crate) const VENDING_MACHINE_CONTRACT: &str = r#"// SPDX-License-Identifier: GPL-3.0
pragma solidity ^0.8.13;

contract VendingMachine {
    address owner;
    error Unauthorized();
    function buyRevert(uint amount) public payable {
        if (amount > msg.value / 2 ether)
            revert("Not enough Ether provided.");
    }
    function buyRequire(uint amount) public payable {
        require(
            amount <= msg.value / 2 ether,
            "Not enough Ether provided."
        );
    }
    function withdraw() public {
        if (msg.sender != owner)
            revert Unauthorized();

        payable(msg.sender).transfer(address(this).balance);
    }
}"#;

#[tokio::test(flavor = "multi_thread")]
async fn test_solc_revert_example() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source("VendingMachine", VENDING_MACHINE_CONTRACT).unwrap();

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove("VendingMachine").unwrap();
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

    for fun in ["buyRevert", "buyRequire"] {
        let resp = contract.method::<_, ()>(fun, U256::zero()).unwrap().call().await;
        assert!(resp.is_ok());

        let ten = WEI_IN_ETHER.saturating_mul(10u64.into());
        let call = contract.method::<_, ()>(fun, ten).unwrap().value(ten);

        let resp = call.clone().call().await;
        let err = resp.unwrap_err().to_string();
        assert!(err.contains("execution reverted: Not enough Ether provided."));
        assert!(err.contains("code: 3"));
    }
}
