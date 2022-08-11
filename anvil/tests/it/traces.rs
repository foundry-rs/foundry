use crate::fork::fork_config;
use anvil::{spawn, NodeConfig};
use ethers::{
    contract::Contract,
    prelude::{Action, ContractFactory, Middleware, Signer, SignerMiddleware, TransactionRequest},
    types::{ActionType, Address, Trace},
    utils::hex,
};
use ethers_solc::{project_util::TempProject, Artifact};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn test_get_transfer_parity_traces() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.http_provider();

    let accounts: Vec<_> = handle.dev_wallets().collect();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = handle.genesis_balance().checked_div(2u64.into()).unwrap();
    // specify the `from` field so that the client knows which account to use
    let tx = TransactionRequest::new().to(to).value(amount).from(from);

    // broadcast it via the eth_sendTransaction API
    let tx = provider.send_transaction(tx, None).await.unwrap().await.unwrap().unwrap();

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());

    match traces[0].action {
        Action::Call(ref call) => {
            assert_eq!(call.from, from);
            assert_eq!(call.to, to);
            assert_eq!(call.value, amount);
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
        r#"
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

    let contract = Contract::new(
        contract.address(),
        abi.unwrap(),
        SignerMiddleware::new(handle.http_provider(), wallets[1].clone()),
    );
    let call = contract.method::<_, ()>("goodbye", ()).unwrap();
    let tx = call.send().await.unwrap().await.unwrap().unwrap();

    let traces = handle.http_provider().trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());
    assert_eq!(traces[0].action_type, ActionType::Suicide);
}

// <https://github.com/foundry-rs/foundry/issues/2656>
#[tokio::test(flavor = "multi_thread")]
async fn test_trace_address_fork() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(15291050u64))).await;
    let provider = handle.http_provider();

    let input = hex::decode("43bcfab60000000000000000000000006b175474e89094c44da98b954eedeac495271d0f0000000000000000000000000000000000000000000000e0bd811c8769a824b00000000000000000000000000000000000000000000000e0ae9925047d8440b60000000000000000000000002e4777139254ff76db957e284b186a4507ff8c67").unwrap();

    let from: Address = "0x2e4777139254ff76db957e284b186a4507ff8c67".parse().unwrap();
    let to: Address = "0xe2f2a5c287993345a840db3b0845fbc70f5935a5".parse().unwrap();
    let tx = TransactionRequest::new().to(to).from(from).data(input);

    api.anvil_impersonate_account(from).await.unwrap();

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

    let json = serde_json::json!(
        [
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
            "traceAddress": [
                0
            ],
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
            "traceAddress": [
                0,
                0
            ],
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
            "traceAddress": [
                0,
                1
            ],
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
            "traceAddress": [
                0,
                1,
                0
            ],
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
            "traceAddress": [
                0,
                1,
                1
            ],
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
            "traceAddress": [
                0,
                1,
                1,
                0
            ],
            "transactionHash": "0x3255cce7312e9c4470e1a1883be13718e971f6faafb96199b8bd75e5b7c39e3a",
            "transactionPosition": 19,
            "type": "call"
        }
    ]
    );

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
