use crate::fork::fork_config;
use alloy_primitives::U256;
use alloy_signer::Signer as AlloySigner;
use anvil::{spawn, NodeConfig};
use ethers::{
    contract::ContractInstance,
    prelude::{
        Action, ContractFactory, GethTrace, GethTraceFrame, Middleware, Signer, SignerMiddleware,
        TransactionRequest, Wallet,
    },
    types::{ActionType, Address, GethDebugTracingCallOptions, Trace},
    utils::hex,
};
use ethers_solc::{project_util::TempProject, Artifact};
use foundry_common::types::{ToAlloy, ToEthers};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn test_get_transfer_parity_traces() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_http_provider();

    let alloy_wallets = handle.dev_wallets().collect::<Vec<_>>();
    let accounts = alloy_wallets
        .into_iter()
        .map(|w| {
            Wallet::new_with_signer(
                w.signer().clone(),
                w.address().to_ethers(),
                w.chain_id().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();
    // specify the `from` field so that the client knows which account to use
    let tx = TransactionRequest::new().to(to).value(amount.to_ethers()).from(from);

    // broadcast it via the eth_sendTransaction API
    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());

    match traces[0].action {
        Action::Call(ref call) => {
            assert_eq!(call.from, from);
            assert_eq!(call.to, to);
            assert_eq!(call.value, amount.to_ethers());
        }
        _ => unreachable!("unexpected action"),
    }

    let num = provider.get_block_number().await.unwrap();
    let block_traces = provider.trace_block(num.into()).await.unwrap();
    assert!(!block_traces.is_empty());

    assert_eq!(traces, block_traces);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_parity_suicide_trace() {
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

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_ws_provider();
    let alloy_wallets = handle.dev_wallets().collect::<Vec<_>>();
    let wallets = alloy_wallets
        .into_iter()
        .map(|w| {
            Wallet::new_with_signer(
                w.signer().clone(),
                w.address().to_ethers(),
                w.chain_id().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let client = Arc::new(SignerMiddleware::new(provider, wallets[0].clone()));

    // deploy successfully
    let factory = ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), client);
    let contract = factory.deploy(()).unwrap().send().await.unwrap();

    let contract = ContractInstance::new(
        contract.address(),
        abi.unwrap(),
        SignerMiddleware::new(handle.ethers_http_provider(), wallets[1].clone()),
    );
    let call = contract.method::<_, ()>("goodbye", ()).unwrap();
    let tx = call.send().await.unwrap().await.unwrap().unwrap();

    let traces =
        handle.ethers_http_provider().trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());
    assert_eq!(traces[1].action_type, ActionType::Suicide);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_transfer_debug_trace_call() {
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

    let mut compiled = prj.compile().unwrap();
    assert!(!compiled.has_compiler_errors());
    let contract = compiled.remove_first("Contract").unwrap();
    let (abi, bytecode, _) = contract.into_contract_bytecode().into_parts();

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ethers_ws_provider();
    let alloy_wallets = handle.dev_wallets().collect::<Vec<_>>();
    let wallets = alloy_wallets
        .into_iter()
        .map(|w| {
            Wallet::new_with_signer(
                w.signer().clone(),
                w.address().to_ethers(),
                w.chain_id().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let client = Arc::new(SignerMiddleware::new(provider, wallets[0].clone()));

    // deploy successfully
    let factory = ContractFactory::new(abi.clone().unwrap(), bytecode.unwrap(), client);
    let contract = factory.deploy(()).unwrap().send().await.unwrap();

    let contract = ContractInstance::new(
        contract.address(),
        abi.unwrap(),
        SignerMiddleware::new(handle.ethers_http_provider(), wallets[1].clone()),
    );
    let call = contract.method::<_, ()>("goodbye", ()).unwrap();

    let traces = handle
        .ethers_http_provider()
        .debug_trace_call(call.tx, None, GethDebugTracingCallOptions::default())
        .await
        .unwrap();
    match traces {
        GethTrace::Known(traces) => match traces {
            GethTraceFrame::Default(traces) => {
                assert!(!traces.failed);
            }
            _ => {
                unreachable!()
            }
        },
        GethTrace::Unknown(_) => {
            unreachable!()
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/2656>
#[tokio::test(flavor = "multi_thread")]
async fn test_trace_address_fork() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(15291050u64))).await;
    let provider = handle.ethers_http_provider();

    let input = hex::decode("43bcfab60000000000000000000000006b175474e89094c44da98b954eedeac495271d0f0000000000000000000000000000000000000000000000e0bd811c8769a824b00000000000000000000000000000000000000000000000e0ae9925047d8440b60000000000000000000000002e4777139254ff76db957e284b186a4507ff8c67").unwrap();

    let from: Address = "0x2e4777139254ff76db957e284b186a4507ff8c67".parse().unwrap();
    let to: Address = "0xe2f2a5c287993345a840db3b0845fbc70f5935a5".parse().unwrap();
    let tx = TransactionRequest::new().to(to).from(from).data(input).gas(300_000);

    api.anvil_impersonate_account(from.to_alloy()).await.unwrap();

    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());
    match traces[0].action {
        Action::Call(ref call) => {
            assert_eq!(call.from, from);
            assert_eq!(call.to, to);
        }
        _ => unreachable!("unexpected action"),
    }

    let json = serde_json::json!([
        {
            "action": {
                "callType": "call",
                "from": "0x2e4777139254ff76db957e284b186a4507ff8c67",
                "gas": "0x262b3",
                "input": "0x43bcfab60000000000000000000000006b175474e89094c44da98b954eedeac495271d0f0000000000000000000000000000000000000000000000e0bd811c8769a824b00000000000000000000000000000000000000000000000e0ae9925047d8440b60000000000000000000000002e4777139254ff76db957e284b186a4507ff8c67",
                "to": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x2131b",
                "output": "0x0000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d3"
            },
            "subtraces": 1,
            "traceAddress": [],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "delegatecall",
                "from": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "gas": "0x23d88",
                "input": "0x43bcfab60000000000000000000000006b175474e89094c44da98b954eedeac495271d0f0000000000000000000000000000000000000000000000e0bd811c8769a824b00000000000000000000000000000000000000000000000e0ae9925047d8440b60000000000000000000000002e4777139254ff76db957e284b186a4507ff8c67",
                "to": "0x15b2838cd28cc353afbe59385db3f366d8945aee",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x1f6e1",
                "output": "0x0000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d3"
            },
            "subtraces": 2,
            "traceAddress": [0],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "staticcall",
                "from": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "gas": "0x192ed",
                "input": "0x50494dc000000000000000000000000000000000000000000000000000000000000000c000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000e0b1ff65617f654b2f00000000000000000000000000000000000000000000000000000000000061a800000000000000000000000000000000000000000000000000b1a2bc2ec5000000000000000000000000000000000000000000000000000006f05b59d3b2000000000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000005f5e1000000000000000000000000000000000000000000000414ec22973db48fd3a3370000000000000000000000000000000000000000000000056bc75e2d6310000000000000000000000000000000000000000000000000000000000a7314a9ba5c0000000000000000000000000000000000000000000000000000000005f5e100000000000000000000000000000000000000000000095f783edc5a5dabcb4ba70000000000000000000000000000000000000000000000056bc75e2d6310000000000000000000000000000000000000000000000000000000000a3f42df4dab",
                "to": "0xca480d596e6717c95a62a4dc1bd4fbd7b7e7d705",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x661a",
                "output": "0x0000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d3"
            },
            "subtraces": 0,
            "traceAddress": [0, 0],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "delegatecall",
                "from": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "gas": "0xd2dc",
                "input": "0x4e331a540000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d30000000000000000000000006b175474e89094c44da98b954eedeac495271d0f000000000000000000000000a2a3cae63476891ab2d640d9a5a800755ee79d6e000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000005f5e100000000000000000000000000000000000000000000095f783edc5a5dabcb4ba70000000000000000000000002e4777139254ff76db957e284b186a4507ff8c6700000000000000000000000000000000000000000000f7be2b91f8a2e2df496e",
                "to": "0x1e91f826fa8aa4fa4d3f595898af3a64dd188848",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x7617",
                "output": "0x"
            },
            "subtraces": 2,
            "traceAddress": [0, 1],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "staticcall",
                "from": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "gas": "0xbf50",
                "input": "0x70a08231000000000000000000000000a2a3cae63476891ab2d640d9a5a800755ee79d6e",
                "to": "0x6b175474e89094c44da98b954eedeac495271d0f",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0xa2a",
                "output": "0x0000000000000000000000000000000000000000000020fe99f8898600d94750"
            },
            "subtraces": 0,
            "traceAddress": [0, 1, 0],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "call",
                "from": "0xe2f2a5c287993345a840db3b0845fbc70f5935a5",
                "gas": "0xa92a",
                "input": "0xa4e285950000000000000000000000002e4777139254ff76db957e284b186a4507ff8c670000000000000000000000006b175474e89094c44da98b954eedeac495271d0f0000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d3",
                "to": "0xa2a3cae63476891ab2d640d9a5a800755ee79d6e",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x4ed3",
                "output": "0x"
            },
            "subtraces": 1,
            "traceAddress": [0, 1, 1],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        },
        {
            "action": {
                "callType": "call",
                "from": "0xa2a3cae63476891ab2d640d9a5a800755ee79d6e",
                "gas": "0x8c90",
                "input": "0xa9059cbb0000000000000000000000002e4777139254ff76db957e284b186a4507ff8c670000000000000000000000000000000000000000000000e0e82ca52ec6e6a4d3",
                "to": "0x6b175474e89094c44da98b954eedeac495271d0f",
                "value": "0x0"
            },
            "blockHash": "0xa47c8f1d8c284cb614e9c8e10d260b33eae16b1957a83141191bc335838d7e29",
            "blockNumber": 15291051,
            "result": {
                "gasUsed": "0x2b42",
                "output": "0x0000000000000000000000000000000000000000000000000000000000000001"
            },
            "subtraces": 0,
            "traceAddress": [0, 1, 1, 0],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        }
    ]);

    let expected_traces: Vec<Trace> = serde_json::from_value(json).unwrap();

    // test matching traceAddress
    traces.into_iter().zip(expected_traces).for_each(|(a, b)| {
        assert_eq!(a.trace_address, b.trace_address);
        assert_eq!(a.subtraces, b.subtraces);
        match (a.action, b.action) {
            (Action::Call(a), Action::Call(b)) => {
                assert_eq!(a.from, b.from);
                assert_eq!(a.to, b.to);
            }
            _ => unreachable!("unexpected action"),
        }
    })
}

// <https://github.com/foundry-rs/foundry/issues/2705>
// <https://etherscan.io/tx/0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845>
#[tokio::test(flavor = "multi_thread")]
async fn test_trace_address_fork2() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(15314401u64))).await;
    let provider = handle.ethers_http_provider();

    let input = hex::decode("30000003000000000000000000000000adda1059a6c6c102b0fa562b9bb2cb9a0de5b1f4000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000a300000004fffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb980c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2f4d2888d29d722226fafa5d9b24f9164c092421e000bb8000000000000004319b52bf08b65295d49117e790000000000000000000000000000000000000000000000008b6d9e8818d6141f000000000000000000000000000000000000000000000000000000086a23af210000000000000000000000000000000000000000000000000000000000").unwrap();

    let from: Address = "0xa009fa1ac416ec02f6f902a3a4a584b092ae6123".parse().unwrap();
    let to: Address = "0x99999999d116ffa7d76590de2f427d8e15aeb0b8".parse().unwrap();
    let tx = TransactionRequest::new().to(to).from(from).data(input).gas(350_000);

    api.anvil_impersonate_account(from.to_alloy()).await.unwrap();

    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();
    assert_eq!(tx.status, Some(1u64.into()));

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();

    assert!(!traces.is_empty());
    match traces[0].action {
        Action::Call(ref call) => {
            assert_eq!(call.from, from);
            assert_eq!(call.to, to);
        }
        _ => unreachable!("unexpected action"),
    }

    let json = serde_json::json!([
        {
            "action": {
                "from": "0xa009fa1ac416ec02f6f902a3a4a584b092ae6123",
                "callType": "call",
                "gas": "0x4fabc",
                "input": "0x30000003000000000000000000000000adda1059a6c6c102b0fa562b9bb2cb9a0de5b1f4000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000a300000004fffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb980c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2f4d2888d29d722226fafa5d9b24f9164c092421e000bb8000000000000004319b52bf08b65295d49117e790000000000000000000000000000000000000000000000008b6d9e8818d6141f000000000000000000000000000000000000000000000000000000086a23af210000000000000000000000000000000000000000000000000000000000",
                "to": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x1d51b",
                "output": "0x"
            },
            "subtraces": 1,
            "traceAddress": [],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "callType": "delegatecall",
                "gas": "0x4d594",
                "input": "0x00000004fffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb980c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2f4d2888d29d722226fafa5d9b24f9164c092421e000bb8000000000000004319b52bf08b65295d49117e790000000000000000000000000000000000000000000000008b6d9e8818d6141f000000000000000000000000000000000000000000000000000000086a23af21",
                "to": "0xadda1059a6c6c102b0fa562b9bb2cb9a0de5b1f4",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x1c35f",
                "output": "0x"
            },
            "subtraces": 3,
            "traceAddress": [0],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "callType": "call",
                "gas": "0x4b6d6",
                "input": "0x16b2da82000000000000000000000000000000000000000000000000000000086a23af21",
                "to": "0xd1663cfb8ceaf22039ebb98914a8c98264643710",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0xd6d",
                "output": "0x0000000000000000000000000000000000000000000000000000000000000000"
            },
            "subtraces": 0,
            "traceAddress": [0, 0],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "callType": "staticcall",
                "gas": "0x49c35",
                "input": "0x3850c7bd",
                "to": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0xa88",
                "output": "0x000000000000000000000000000000000000004319b52bf08b65295d49117e7900000000000000000000000000000000000000000000000000000000000148a0000000000000000000000000000000000000000000000000000000000000010e000000000000000000000000000000000000000000000000000000000000012c000000000000000000000000000000000000000000000000000000000000012c00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001"
            },
            "subtraces": 0,
            "traceAddress": [0, 1],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "callType": "call",
                "gas": "0x48d01",
                "input": "0x128acb0800000000000000000000000099999999d116ffa7d76590de2f427d8e15aeb0b80000000000000000000000000000000000000000000000000000000000000001fffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb98000000000000000000000000000000000000000000000000000000001000276a400000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000002bc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2f4d2888d29d722226fafa5d9b24f9164c092421e000bb8000000000000000000000000000000000000000000",
                "to": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x18c20",
                "output": "0x0000000000000000000000000000000000000000000000008b5116525f9edc3efffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb980"
            },
            "subtraces": 4,
            "traceAddress": [0, 2],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "callType": "call",
                "gas": "0x3802a",
                "input": "0xa9059cbb00000000000000000000000099999999d116ffa7d76590de2f427d8e15aeb0b8000000000000000000000000000000000000000000000986236e1301eaf04680",
                "to": "0xf4d2888d29d722226fafa5d9b24f9164c092421e",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x31b6",
                "output": "0x0000000000000000000000000000000000000000000000000000000000000001"
            },
            "subtraces": 0,
            "traceAddress": [0, 2, 0],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "callType": "staticcall",
                "gas": "0x34237",
                "input": "0x70a082310000000000000000000000004b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "to": "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x9e6",
                "output": "0x000000000000000000000000000000000000000000000091cda6c1ce33e53b89"
            },
            "subtraces": 0,
            "traceAddress": [0, 2, 1],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "callType": "call",
                "gas": "0x3357e",
                "input": "0xfa461e330000000000000000000000000000000000000000000000008b5116525f9edc3efffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb9800000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000002bc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2f4d2888d29d722226fafa5d9b24f9164c092421e000bb8000000000000000000000000000000000000000000",
                "to": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x2e8b",
                "output": "0x"
            },
            "subtraces": 1,
            "traceAddress": [0, 2, 2],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x99999999d116ffa7d76590de2f427d8e15aeb0b8",
                "callType": "call",
                "gas": "0x324db",
                "input": "0xa9059cbb0000000000000000000000004b5ab61593a2401b1075b90c04cbcdd3f87ce0110000000000000000000000000000000000000000000000008b5116525f9edc3e",
                "to": "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x2a6e",
                "output": "0x0000000000000000000000000000000000000000000000000000000000000001"
            },
            "subtraces": 0,
            "traceAddress": [0, 2, 2, 0],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        },
        {
            "action": {
                "from": "0x4b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "callType": "staticcall",
                "gas": "0x30535",
                "input": "0x70a082310000000000000000000000004b5ab61593a2401b1075b90c04cbcdd3f87ce011",
                "to": "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
                "value": "0x0"
            },
            "blockHash": "0xf689ba7749648b8c5c8f5eedd73001033f0aed7ea50b7c81048ad1533b8d3d73",
            "blockNumber": 15314402,
            "result": {
                "gasUsed": "0x216",
                "output": "0x00000000000000000000000000000000000000000000009258f7d820938417c7"
            },
            "subtraces": 0,
            "traceAddress": [0, 2, 3],
            "transactionHash": "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
            "transactionPosition": 289,
            "type": "call"
        }
    ]);

    let expected_traces: Vec<Trace> = serde_json::from_value(json).unwrap();

    // test matching traceAddress
    traces.into_iter().zip(expected_traces).for_each(|(a, b)| {
        assert_eq!(a.trace_address, b.trace_address);
        assert_eq!(a.subtraces, b.subtraces);
        match (a.action, b.action) {
            (Action::Call(a), Action::Call(b)) => {
                assert_eq!(a.from, b.from);
                assert_eq!(a.to, b.to);
            }
            _ => unreachable!("unexpected action"),
        }
    })
}
