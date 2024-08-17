use crate::{
    abi::{MulticallContract, SimpleStorage},
    fork::fork_config,
    utils::http_provider_with_signer,
};
use alloy_eips::BlockId;
use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{hex, Address, Bytes, U256};
use alloy_provider::{
    ext::{DebugApi, TraceApi},
    Provider,
};
use alloy_rpc_types::{
    trace::{
        filter::{TraceFilter, TraceFilterMode},
        geth::{
            CallConfig, GethDebugBuiltInTracerType, GethDebugTracerType,
            GethDebugTracingCallOptions, GethDebugTracingOptions, GethTrace,
        },
        parity::{Action, LocalizedTransactionTrace},
    },
    TransactionRequest,
};
use alloy_serde::WithOtherFields;
use alloy_sol_types::sol;
use anvil::{spawn, Hardfork, NodeConfig};

#[tokio::test(flavor = "multi_thread")]
async fn test_get_transfer_parity_traces() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let amount = handle.genesis_balance().checked_div(U256::from(2u64)).unwrap();
    // specify the `from` field so that the client knows which account to use
    let tx = TransactionRequest::default().to(to).value(amount).from(from);
    let tx = WithOtherFields::new(tx);

    // broadcast it via the eth_sendTransaction API
    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());

    match traces[0].trace.action {
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

sol!(
    #[sol(rpc, bytecode = "0x6080604052348015600f57600080fd5b50336000806101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff16021790555060a48061005e6000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c806375fc8e3c14602d575b600080fd5b60336035565b005b60008054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16fffea26469706673582212205006867290df97c54f2df1cb94fc081197ab670e2adf5353071d2ecce1d694b864736f6c634300080d0033")]
    contract SuicideContract {
        address payable private owner;
        constructor() public {
            owner = payable(msg.sender);
        }
        function goodbye() public {
            selfdestruct(owner);
        }
    }
);

#[tokio::test(flavor = "multi_thread")]
async fn test_parity_suicide_trace() {
    let (_api, handle) = spawn(NodeConfig::test().with_hardfork(Some(Hardfork::Shanghai))).await;
    let provider = handle.ws_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let owner = wallets[0].address();
    let destructor = wallets[1].address();

    let contract_addr =
        SuicideContract::deploy_builder(provider.clone()).from(owner).deploy().await.unwrap();
    let contract = SuicideContract::new(contract_addr, provider.clone());
    let call = contract.goodbye().from(destructor);
    let call = call.send().await.unwrap();
    let tx = call.get_receipt().await.unwrap();

    let traces = handle.http_provider().trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());
    assert!(traces[1].trace.action.is_selfdestruct());
}

sol!(
    #[sol(rpc, bytecode = "0x6080604052348015600f57600080fd5b50336000806101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff16021790555060a48061005e6000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c806375fc8e3c14602d575b600080fd5b60336035565b005b60008054906101000a900473ffffffffffffffffffffffffffffffffffffffff1673ffffffffffffffffffffffffffffffffffffffff16fffea26469706673582212205006867290df97c54f2df1cb94fc081197ab670e2adf5353071d2ecce1d694b864736f6c634300080d0033")]
    contract DebugTraceContract {
        address payable private owner;
        constructor() public {
            owner = payable(msg.sender);
        }
        function goodbye() public {
            selfdestruct(owner);
        }
    }
);

#[tokio::test(flavor = "multi_thread")]
async fn test_transfer_debug_trace_call() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let deployer: EthereumWallet = wallets[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), deployer);

    let contract_addr = DebugTraceContract::deploy_builder(provider.clone())
        .from(wallets[0].clone().address())
        .deploy()
        .await
        .unwrap();

    let caller: EthereumWallet = wallets[1].clone().into();
    let caller_provider = http_provider_with_signer(&handle.http_endpoint(), caller);
    let contract = DebugTraceContract::new(contract_addr, caller_provider);

    let call = contract.goodbye().from(wallets[1].address());
    let calldata = call.calldata().to_owned();

    let tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*contract.address())
        .with_input(calldata);

    let traces = handle
        .http_provider()
        .debug_trace_call(tx, BlockId::latest(), GethDebugTracingCallOptions::default())
        .await
        .unwrap();

    match traces {
        GethTrace::Default(default_frame) => {
            assert!(!default_frame.failed);
        }
        _ => {
            unreachable!()
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_call_tracer_debug_trace_call() {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let deployer: EthereumWallet = wallets[0].clone().into();
    let provider = http_provider_with_signer(&handle.http_endpoint(), deployer);

    let multicall_contract = MulticallContract::deploy(&provider).await.unwrap();

    let simple_storage_contract =
        SimpleStorage::deploy(&provider, "init value".to_string()).await.unwrap();

    let set_value = simple_storage_contract.setValue("bar".to_string());
    let set_value_calldata = set_value.calldata();

    let internal_call_tx_builder = multicall_contract.aggregate(vec![MulticallContract::Call {
        target: *simple_storage_contract.address(),
        callData: set_value_calldata.to_owned(),
    }]);

    let internal_call_tx_calldata = internal_call_tx_builder.calldata().to_owned();

    // calling SimpleStorage contract through Multicall should result in an internal call
    let internal_call_tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*multicall_contract.address())
        .with_input(internal_call_tx_calldata);

    let internal_call_tx_traces = handle
        .http_provider()
        .debug_trace_call(
            internal_call_tx.clone(),
            BlockId::latest(),
            GethDebugTracingCallOptions::default().with_tracing_options(
                GethDebugTracingOptions::default()
                    .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
                    .with_call_config(CallConfig::default().with_log()),
            ),
        )
        .await
        .unwrap();

    match internal_call_tx_traces {
        GethTrace::CallTracer(call_frame) => {
            assert!(call_frame.calls.len() == 1);
            assert!(
                call_frame.calls.first().unwrap().to.unwrap() == *simple_storage_contract.address()
            );
            assert!(call_frame.calls.first().unwrap().logs.len() == 1);
        }
        _ => {
            unreachable!()
        }
    }

    // only_top_call option - should not return any internal calls
    let internal_call_only_top_call_tx_traces = handle
        .http_provider()
        .debug_trace_call(
            internal_call_tx.clone(),
            BlockId::latest(),
            GethDebugTracingCallOptions::default().with_tracing_options(
                GethDebugTracingOptions::default()
                    .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
                    .with_call_config(CallConfig::default().with_log().only_top_call()),
            ),
        )
        .await
        .unwrap();

    match internal_call_only_top_call_tx_traces {
        GethTrace::CallTracer(call_frame) => {
            assert!(call_frame.calls.is_empty());
        }
        _ => {
            unreachable!()
        }
    }

    // directly calling the SimpleStorage contract should not result in any internal calls
    let direct_call_tx = TransactionRequest::default()
        .from(wallets[1].address())
        .to(*simple_storage_contract.address())
        .with_input(set_value_calldata.to_owned());

    let direct_call_tx_traces = handle
        .http_provider()
        .debug_trace_call(
            direct_call_tx,
            BlockId::latest(),
            GethDebugTracingCallOptions::default().with_tracing_options(
                GethDebugTracingOptions::default()
                    .with_tracer(GethDebugTracerType::from(GethDebugBuiltInTracerType::CallTracer))
                    .with_call_config(CallConfig::default().with_log()),
            ),
        )
        .await
        .unwrap();

    match direct_call_tx_traces {
        GethTrace::CallTracer(call_frame) => {
            assert!(call_frame.calls.is_empty());
            assert!(call_frame.to.unwrap() == *simple_storage_contract.address());
            assert!(call_frame.logs.len() == 1);
        }
        _ => {
            unreachable!()
        }
    }
}

// <https://github.com/foundry-rs/foundry/issues/2656>
#[tokio::test(flavor = "multi_thread")]
async fn test_trace_address_fork() {
    let (api, handle) = spawn(fork_config().with_fork_block_number(Some(15291050u64))).await;
    let provider = handle.http_provider();

    let input = hex::decode("43bcfab60000000000000000000000006b175474e89094c44da98b954eedeac495271d0f0000000000000000000000000000000000000000000000e0bd811c8769a824b00000000000000000000000000000000000000000000000e0ae9925047d8440b60000000000000000000000002e4777139254ff76db957e284b186a4507ff8c67").unwrap();

    let from: Address = "0x2e4777139254ff76db957e284b186a4507ff8c67".parse().unwrap();
    let to: Address = "0xe2f2a5c287993345a840db3b0845fbc70f5935a5".parse().unwrap();
    let tx = TransactionRequest::default()
        .to(to)
        .from(from)
        .with_input::<Bytes>(input.into())
        .with_gas_limit(300_000);

    let tx = WithOtherFields::new(tx);
    api.anvil_impersonate_account(from).await.unwrap();

    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();
    assert!(!traces.is_empty());
    match traces[0].trace.action {
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

    let expected_traces: Vec<LocalizedTransactionTrace> = serde_json::from_value(json).unwrap();

    // test matching traceAddress
    traces.into_iter().zip(expected_traces).for_each(|(a, b)| {
        assert_eq!(a.trace.trace_address, b.trace.trace_address);
        assert_eq!(a.trace.subtraces, b.trace.subtraces);
        match (a.trace.action, b.trace.action) {
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
    let provider = handle.http_provider();

    let input = hex::decode("30000003000000000000000000000000adda1059a6c6c102b0fa562b9bb2cb9a0de5b1f4000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000a300000004fffffffffffffffffffffffffffffffffffffffffffff679dc91ecfe150fb980c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2f4d2888d29d722226fafa5d9b24f9164c092421e000bb8000000000000004319b52bf08b65295d49117e790000000000000000000000000000000000000000000000008b6d9e8818d6141f000000000000000000000000000000000000000000000000000000086a23af210000000000000000000000000000000000000000000000000000000000").unwrap();

    let from: Address = "0xa009fa1ac416ec02f6f902a3a4a584b092ae6123".parse().unwrap();
    let to: Address = "0x99999999d116ffa7d76590de2f427d8e15aeb0b8".parse().unwrap();
    let tx = TransactionRequest::default()
        .to(to)
        .from(from)
        .with_input::<Bytes>(input.into())
        .with_gas_limit(350_000);

    let tx = WithOtherFields::new(tx);
    api.anvil_impersonate_account(from).await.unwrap();

    let tx = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();
    let status = tx.inner.inner.inner.receipt.status.coerce_status();
    assert!(status);

    let traces = provider.trace_transaction(tx.transaction_hash).await.unwrap();

    assert!(!traces.is_empty());
    match traces[0].trace.action {
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

    let expected_traces: Vec<LocalizedTransactionTrace> = serde_json::from_value(json).unwrap();

    // test matching traceAddress
    traces.into_iter().zip(expected_traces).for_each(|(a, b)| {
        assert_eq!(a.trace.trace_address, b.trace.trace_address);
        assert_eq!(a.trace.subtraces, b.trace.subtraces);
        match (a.trace.action, b.trace.action) {
            (Action::Call(a), Action::Call(b)) => {
                assert_eq!(a.from, b.from);
                assert_eq!(a.to, b.to);
            }
            _ => unreachable!("unexpected action"),
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn test_trace_filter() {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let provider = handle.ws_provider();

    let accounts = handle.dev_wallets().collect::<Vec<_>>();
    let from = accounts[0].address();
    let to = accounts[1].address();
    let from_two = accounts[2].address();
    let to_two = accounts[3].address();

    // Test default block ranges.
    // From will be earliest, to will be best/latest
    let tracer = TraceFilter {
        from_block: None,
        to_block: None,
        from_address: vec![],
        to_address: vec![],
        mode: TraceFilterMode::Intersection,
        after: None,
        count: None,
    };

    for i in 0..=5 {
        let tx = TransactionRequest::default().to(to).value(U256::from(i)).from(from);
        let tx = WithOtherFields::new(tx);
        api.send_transaction(tx).await.unwrap();
    }

    let traces = api.trace_filter(tracer).await.unwrap();
    assert_eq!(traces.len(), 5);

    // Test filtering by address
    let tracer = TraceFilter {
        from_block: Some(provider.get_block_number().await.unwrap()),
        to_block: None,
        from_address: vec![from_two],
        to_address: vec![to_two],
        mode: TraceFilterMode::Intersection,
        after: None,
        count: None,
    };

    for i in 0..=5 {
        let tx = TransactionRequest::default().to(to).value(U256::from(i)).from(from);
        let tx = WithOtherFields::new(tx);
        api.send_transaction(tx).await.unwrap();

        let tx = TransactionRequest::default().to(to_two).value(U256::from(i)).from(from_two);
        let tx = WithOtherFields::new(tx);
        api.send_transaction(tx).await.unwrap();
    }

    let traces = api.trace_filter(tracer).await.unwrap();
    assert_eq!(traces.len(), 5);

    // Test for the following actions:
    // Create (deploy the contract)
    // Call (goodbye function)
    // SelfDestruct (side-effect of goodbye)
    let contract_addr =
        SuicideContract::deploy_builder(provider.clone()).from(from).deploy().await.unwrap();
    let contract = SuicideContract::new(contract_addr, provider.clone());

    // Test TraceActions
    let tracer = TraceFilter {
        from_block: Some(provider.get_block_number().await.unwrap()),
        to_block: None,
        from_address: vec![from, contract_addr],
        to_address: vec![], // Leave as 0 address
        mode: TraceFilterMode::Union,
        after: None,
        count: None,
    };

    // Execute call
    let call = contract.goodbye().from(from);
    let call = call.send().await.unwrap();
    call.get_receipt().await.unwrap();

    // Mine transactions to filter against
    for i in 0..=5 {
        let tx = TransactionRequest::default().to(to_two).value(U256::from(i)).from(from_two);
        let tx = WithOtherFields::new(tx);
        api.send_transaction(tx).await.unwrap();
    }

    let traces = api.trace_filter(tracer).await.unwrap();
    assert_eq!(traces.len(), 8);

    // Test Range Error
    let latest = provider.get_block_number().await.unwrap();
    let tracer = TraceFilter {
        from_block: Some(latest),
        to_block: Some(latest + 301),
        from_address: vec![],
        to_address: vec![],
        mode: TraceFilterMode::Union,
        after: None,
        count: None,
    };

    let traces = api.trace_filter(tracer).await;
    assert!(traces.is_err());

    // Test invalid block range
    let latest = provider.get_block_number().await.unwrap();
    let tracer = TraceFilter {
        from_block: Some(latest + 10),
        to_block: Some(latest),
        from_address: vec![],
        to_address: vec![],
        mode: TraceFilterMode::Union,
        after: None,
        count: None,
    };

    let traces = api.trace_filter(tracer).await;
    assert!(traces.is_err());

    // Test after and count
    let tracer = TraceFilter {
        from_block: Some(provider.get_block_number().await.unwrap()),
        to_block: None,
        from_address: vec![],
        to_address: vec![],
        mode: TraceFilterMode::Union,
        after: Some(3),
        count: Some(5),
    };

    for i in 0..=10 {
        let tx = TransactionRequest::default().to(to).value(U256::from(i)).from(from);
        let tx = WithOtherFields::new(tx);
        api.send_transaction(tx).await.unwrap();
    }

    let traces = api.trace_filter(tracer).await.unwrap();
    assert_eq!(traces.len(), 5);
}
