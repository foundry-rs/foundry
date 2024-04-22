use crate::utils::ws_provider_with_signer;
use alloy_network::EthereumSigner;
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types::{TransactionRequest, WithOtherFields};
use alloy_sol_types::sol;
use anvil::{spawn, NodeConfig};
use foundry_compilers::{project_util::TempProject, Artifact};

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
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = ws_provider_with_signer(&handle.ws_endpoint(), signer);

    // should catch the revert during estimation which results in an err
    let err = provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().input(bytecode.into()).from(sender),
        ))
        .await
        .unwrap_err();
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
        owner = address(1);
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
            function getSecret() external;
        }
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = ws_provider_with_signer(&handle.ws_endpoint(), signer);

    // deploy successfully
    provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().input(bytecode.into()).from(sender),
        ))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);
    let contract = Contract::new(contract_address, &provider);

    let res = contract.getSecret().send().await.unwrap_err();

    let msg = res.to_string();
    assert!(msg.contains("execution reverted: revert: !authorized"));
}

// <https://docs.soliditylang.org/en/latest/control-structures.html#revert>
const VENDING_MACHINE_CONTRACT: &str = r#"// SPDX-License-Identifier: GPL-3.0
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

sol!(
    #[sol(rpc)]
    contract VendingMachine {
        function buyRevert(uint amount) external payable;
        function buyRequire(uint amount) external payable;
        function withdraw() external;
    }
);

#[tokio::test(flavor = "multi_thread")]
async fn test_solc_revert_example() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source("VendingMachine", VENDING_MACHINE_CONTRACT).unwrap();

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("VendingMachine").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = ws_provider_with_signer(&handle.ws_endpoint(), signer);

    // deploy successfully
    provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().input(bytecode.into()).from(sender),
        ))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);
    let contract = VendingMachine::new(contract_address, &provider);

    let res = contract
        .buyRevert(U256::from(100))
        .value(U256::from(1))
        .from(sender)
        .send()
        .await
        .unwrap_err();
    let msg = res.to_string();
    assert!(msg.contains("execution reverted: revert: Not enough Ether provided."));

    let res = contract
        .buyRequire(U256::from(100))
        .value(U256::from(1))
        .from(sender)
        .send()
        .await
        .unwrap_err();
    let msg = res.to_string();
    assert!(msg.contains("execution reverted: revert: Not enough Ether provided."));
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

    sol!(
        #[sol(rpc)]
        contract Contract {
            function setNumber(uint256 num) external;
        }
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = ws_provider_with_signer(&handle.ws_endpoint(), signer);

    // deploy successfully
    provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().input(bytecode.into()).from(sender),
        ))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);
    let contract = Contract::new(contract_address, &provider);

    let res = contract.setNumber(U256::from(0)).send().await.unwrap_err();

    let msg = res.to_string();
    assert!(msg.contains("execution reverted: revert: RevertStringFooBar"));
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

    sol!(
        #[sol(rpc)]
        contract Contract {
            function revertAddress() external;
        }
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = ws_provider_with_signer(&handle.ws_endpoint(), signer);

    // deploy successfully
    provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().input(bytecode.into()).from(sender),
        ))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);
    let contract = Contract::new(contract_address, &provider);

    let res = contract.revertAddress().send().await.unwrap_err();
    assert!(res.to_string().contains("execution reverted"));
}
