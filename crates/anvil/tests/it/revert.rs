use crate::abi::VENDING_MACHINE_CONTRACT;
use anvil::{spawn, NodeConfig};
use ethers::{
    contract::{ContractFactory, ContractInstance},
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
    let contract = compiled.remove_first("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (_api, handle) = spawn(NodeConfig::test()).await;
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
    let contract = compiled.remove_first("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider().await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let client = Arc::new(SignerMiddleware::new(provider, wallets[0].clone()));

    // deploy successfully
    let factory = ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), client);
    let contract = factory.deploy(()).unwrap().send().await.unwrap();

    let contract = ContractInstance::new(
        contract.address(),
        abi.unwrap(),
        SignerMiddleware::new(handle.http_provider(), wallets[1].clone()),
    );

    let resp = contract.method::<_, U256>("getSecret", ()).unwrap().call().await;

    let err = resp.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("execution reverted: !authorized"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_solc_revert_example() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source("VendingMachine", VENDING_MACHINE_CONTRACT).unwrap();

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("VendingMachine").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider().await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let client = Arc::new(SignerMiddleware::new(provider, wallets[0].clone()));

    // deploy successfully
    let factory = ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), client);
    let contract = factory.deploy(()).unwrap().send().await.unwrap();

    let contract = ContractInstance::new(
        contract.address(),
        abi.unwrap(),
        SignerMiddleware::new(handle.http_provider(), wallets[1].clone()),
    );

    for fun in ["buyRevert", "buyRequire"] {
        let resp = contract.method::<_, ()>(fun, U256::zero()).unwrap().call().await;
        resp.unwrap();

        let ten = WEI_IN_ETHER.saturating_mul(10u64.into());
        let call = contract.method::<_, ()>(fun, ten).unwrap().value(ten);

        let resp = call.clone().call().await;
        let err = resp.unwrap_err().to_string();
        assert!(err.contains("execution reverted: Not enough Ether provided."));
        assert!(err.contains("code: 3"));
    }
}

// <https://github.com/foundry-rs/foundry/issues/1871>
#[tokio::test(flavor = "multi_thread")]
async fn test_another_revert_message() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r#"
pragma solidity 0.8.13;
contract Contract {
    uint256 public number;

    function setNumber(uint256 num) public {
        require(num != 0, "RevertStringFooBar");
        number = num;
    }
}
"#,
    )
    .unwrap();

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider().await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let client = Arc::new(SignerMiddleware::new(provider, wallets[0].clone()));

    // deploy successfully
    let factory = ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), client);
    let contract = factory.deploy(()).unwrap().send().await.unwrap();

    let contract = ContractInstance::new(
        contract.address(),
        abi.unwrap(),
        SignerMiddleware::new(handle.http_provider(), wallets[1].clone()),
    );
    let call = contract.method::<_, ()>("setNumber", U256::zero()).unwrap();
    let resp = call.send().await;

    let err = resp.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("execution reverted: RevertStringFooBar"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_solc_revert_custom_errors() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r#"
pragma solidity 0.8.13;
contract Contract {
    uint256 public number;
    error AddressRevert(address);

    function revertAddress() public {
         revert AddressRevert(address(1));
    }
}
"#,
    )
    .unwrap();

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (_api, handle) = spawn(NodeConfig::test()).await;

    let provider = handle.ws_provider().await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let client = Arc::new(SignerMiddleware::new(provider, wallets[0].clone()));

    // deploy successfully
    let factory =
        ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), Arc::clone(&client));
    let contract = factory.deploy(()).unwrap().send().await.unwrap();

    let call = contract.method::<_, ()>("revertAddress", ()).unwrap().gas(150000);

    let resp = call.call().await;

    let _ = resp.unwrap_err();
}
