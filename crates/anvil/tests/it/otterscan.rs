//! tests for otterscan endpoints
use crate::{
    abi::{AlloyMulticallContract, MulticallContract},
    utils::{
        ethers_ws_provider, http_provider, http_provider_with_signer, ws_provider_with_signer,
        ContractInstanceCompat, DeploymentTxFactoryCompat,
    },
};
use alloy_network::EthereumSigner;
use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, BlockTransactions, TransactionRequest, WithOtherFields};
use alloy_sol_types::sol;
use anvil::{
    eth::otterscan::types::{
        OtsInternalOperation, OtsInternalOperationType, OtsTrace, OtsTraceType,
    },
    spawn, NodeConfig,
};
// use ethers::{
//     abi::Address,
//     prelude::{ContractFactory, ContractInstance, Middleware, SignerMiddleware},
//     signers::Signer,
//     types::{Bytes, TransactionRequest},
//     utils::get_contract_address,
// };
use foundry_common::types::{ToAlloy, ToEthers};
use foundry_compilers::{project_util::TempProject, Artifact};
use std::{collections::VecDeque, str::FromStr, sync::Arc};

#[tokio::test(flavor = "multi_thread")]
async fn can_call_erigon_get_header_by_number() {
    let (api, _handle) = spawn(NodeConfig::test()).await;
    api.mine_one().await;

    let res0 = api.erigon_get_header_by_number(0.into()).await.unwrap().unwrap();
    let res1 = api.erigon_get_header_by_number(1.into()).await.unwrap().unwrap();

    assert_eq!(res0.header.number, Some(0));
    assert_eq!(res1.header.number, Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_api_level() {
    let (api, _handle) = spawn(NodeConfig::test()).await;

    assert_eq!(api.ots_get_api_level().await.unwrap(), 8);
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_internal_operations_contract_deploy() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    let contract_receipt = AlloyMulticallContract::deploy_builder(provider.clone())
        .from(sender)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let contract_address = sender.create(0);

    let res = api.ots_get_internal_operations(contract_receipt.transaction_hash).await.unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(
        res[0],
        OtsInternalOperation {
            r#type: OtsInternalOperationType::Create,
            from: sender,
            to: contract_address,
            value: U256::from(0)
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_internal_operations_contract_transfer() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    let provider = http_provider(&handle.http_endpoint());

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();

    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();

    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);

    let receipt = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let res = api.ots_get_internal_operations(receipt.transaction_hash).await.unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(
        res[0],
        OtsInternalOperation {
            r#type: OtsInternalOperationType::Transfer,
            from,
            to,
            value: amount
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_internal_operations_contract_create2() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r"
pragma solidity 0.8.13;
contract Contract {
    address constant CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;
    constructor() {}
    function deployContract() public {
        uint256 salt = 0;
        uint256 code = 0;
        bytes memory creationCode = abi.encodePacked(code);
        (bool success,) = address(CREATE2_DEPLOYER).call(abi.encodePacked(salt, creationCode));
        require(success);
    }
}
",
    )
    .unwrap();

    sol!(
        #[sol(rpc)]
        contract Contract {
            function deployContract() external;
        }
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (api, handle) = spawn(NodeConfig::test()).await;
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
    let receipt = contract.deployContract().send().await.unwrap().get_receipt().await.unwrap();

    let res = api.ots_get_internal_operations(receipt.transaction_hash).await.unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(
        res[0],
        OtsInternalOperation {
            r#type: OtsInternalOperationType::Create2,
            from: Address::from_str("0x4e59b44847b379578588920cA78FbF26c0B4956C").unwrap(),
            to: Address::from_str("0x347bcdad821abc09b8c275881b368de36476b62c").unwrap(),
            value: U256::from(0)
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_get_internal_operations_contract_selfdestruct() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r"
pragma solidity 0.8.13;
contract Contract {
    address payable private owner;
    constructor() public {
        owner = payable(msg.sender);
    }
    function goodbye() public {
        selfdestruct(owner);
    }
}
",
    )
    .unwrap();

    sol!(
        #[sol(rpc)]
        contract Contract {
            function goodbye() external;
        }
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (api, handle) = spawn(NodeConfig::test()).await;
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

    let receipt = contract.goodbye().send().await.unwrap().get_receipt().await.unwrap();

    let res = api.ots_get_internal_operations(receipt.transaction_hash).await.unwrap();

    assert_eq!(res.len(), 1);
    assert_eq!(
        res[0],
        OtsInternalOperation {
            r#type: OtsInternalOperationType::SelfDestruct,
            from: *contract.address(),
            to: Default::default(),
            value: U256::from(0)
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn can_call_ots_has_code() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let signer: EthereumSigner = wallet.clone().into();
    let sender = wallet.address();

    let provider = http_provider_with_signer(&handle.http_endpoint(), signer);

    api.mine_one().await;

    let contract_address = sender.create(0);

    // no code in the address before deploying
    assert!(!api.ots_has_code(contract_address, BlockNumberOrTag::Number(1)).await.unwrap());

    let contract_builder = AlloyMulticallContract::deploy_builder(provider.clone());
    let contract_receipt = contract_builder.send().await.unwrap().get_receipt().await.unwrap();

    let num = provider.get_block_number().await.unwrap();
    assert_eq!(num, contract_receipt.block_number.unwrap());

    // code is detected after deploying
    assert!(api.ots_has_code(contract_address, BlockNumberOrTag::Number(num)).await.unwrap());

    // code is not detected for the previous block
    assert!(!api.ots_has_code(contract_address, BlockNumberOrTag::Number(num - 1)).await.unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_call_call_ots_trace_transaction() {
    let prj = TempProject::dapptools().unwrap();
    prj.add_source(
        "Contract",
        r#"
pragma solidity 0.8.13;
contract Contract {
    address payable private owner;
    constructor() public {
        owner = payable(msg.sender);
    }
    function run() payable public {
        this.do_staticcall();
        this.do_call();
    }

    function do_staticcall() external view returns (bool) {
        return true;
    }

    function do_call() external {
        owner.call{value: address(this).balance}("");
        address(this).delegatecall(abi.encodeWithSignature("do_delegatecall()"));
    }
    
    function do_delegatecall() internal {
    }
}
"#,
    )
    .unwrap();

    sol!(
        #[sol(rpc)]
        contract Contract {
            function run() external payable;
            function do_staticcall() external view returns (bool);
            function do_call() external;
            function do_delegatecall() external;
        }
    );

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let bytecode = contract.into_bytecode_bytes().unwrap();

    let (api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let signer: EthereumSigner = wallets[0].clone().into();
    let sender = wallets[0].address();

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

    let receipt = contract
        .run()
        .from(sender)
        .value(U256::from(1337))
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let res = api.ots_trace_transaction(receipt.transaction_hash).await.unwrap();

    assert_eq!(
        res,
        vec![
            OtsTrace {
                r#type: OtsTraceType::Call,
                depth: 0,
                from: sender,
                to: contract_address,
                value: U256::from(1337),
                input: Bytes::from_str("0xc0406226").unwrap().0.into()
            },
            OtsTrace {
                r#type: OtsTraceType::StaticCall,
                depth: 1,
                from: contract_address,
                to: contract_address,
                value: U256::ZERO,
                input: Bytes::from_str("0x6a6758fe").unwrap().0.into()
            },
            OtsTrace {
                r#type: OtsTraceType::Call,
                depth: 1,
                from: contract_address,
                to: contract_address,
                value: U256::ZERO,
                input: Bytes::from_str("0x96385e39").unwrap().0.into()
            },
            OtsTrace {
                r#type: OtsTraceType::Call,
                depth: 2,
                from: contract_address,
                to: sender,
                value: U256::from(1337),
                input: Bytes::from_str("0x").unwrap().0.into()
            },
            OtsTrace {
                r#type: OtsTraceType::DelegateCall,
                depth: 2,
                from: contract_address,
                to: contract_address,
                value: U256::ZERO,
                input: Bytes::from_str("0xa1325397").unwrap().0.into()
            },
        ]
    );
}
