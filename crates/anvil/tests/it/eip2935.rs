use crate::utils::http_provider;
use alloy_eips::{
    BlockId, BlockNumberOrTag, eip2935::HISTORY_STORAGE_ADDRESS, eip4788::BEACON_ROOTS_ADDRESS,
    eip7002::WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
    eip7251::CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
};
use alloy_evm::precompiles::{DynPrecompile, PrecompileInput};
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, B256, Bytes, U256, address, bytes};
use alloy_provider::Provider;
use alloy_rpc_types::{
    TransactionRequest,
    simulate::{SimBlock, SimulatePayload},
    trace::{
        opcode::{BlockOpcodeGas, TransactionOpcodeGas},
        parity::{Delta, TraceResults, TraceResultsWithTransactionHash, TraceType},
    },
};
use alloy_serde::WithOtherFields;
use anvil::{NodeConfig, PrecompileFactory, spawn};
#[cfg(feature = "optimism")]
use foundry_evm::hardfork::OpHardfork;
use foundry_evm::hardfork::{EthereumHardfork, TempoHardfork};
#[cfg(feature = "optimism")]
use foundry_evm_networks::NetworkConfigs;
use revm::precompile::{PrecompileError, PrecompileOutput, PrecompileStatus};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

const REPLAY_PRE_EXECUTION_ERROR: &str = "replay pre-execution sentinel";

#[derive(Debug)]
struct FailingHistoryPrecompile(Arc<AtomicBool>);

impl PrecompileFactory for FailingHistoryPrecompile {
    fn precompiles(&self) -> Vec<(Address, DynPrecompile)> {
        let fail = Arc::clone(&self.0);
        let precompile = DynPrecompile::from(move |input: PrecompileInput<'_>| {
            if fail.load(Ordering::SeqCst) {
                return Err(PrecompileError::Fatal(REPLAY_PRE_EXECUTION_ERROR.to_string()));
            }
            Ok(PrecompileOutput {
                status: PrecompileStatus::Success,
                bytes: Bytes::new(),
                gas_used: 0,
                gas_refunded: 0,
                state_gas_used: 0,
                state_gas_spilled: 0,
                reservoir: input.reservoir,
            })
        });
        vec![(HISTORY_STORAGE_ADDRESS, precompile)]
    }
}

#[derive(Debug)]
struct OrderedBlockStartPrecompiles(Arc<AtomicUsize>);

impl PrecompileFactory for OrderedBlockStartPrecompiles {
    fn precompiles(&self) -> Vec<(Address, DynPrecompile)> {
        let history_order = Arc::clone(&self.0);
        let history = DynPrecompile::from(move |input: PrecompileInput<'_>| {
            if history_order.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                return Err(PrecompileError::Fatal("EIP-2935 did not execute first".to_string()));
            }
            Ok(PrecompileOutput {
                status: PrecompileStatus::Success,
                bytes: Bytes::new(),
                gas_used: 0,
                gas_refunded: 0,
                state_gas_used: 0,
                state_gas_spilled: 0,
                reservoir: input.reservoir,
            })
        });

        let beacon_order = Arc::clone(&self.0);
        let beacon = DynPrecompile::from(move |input: PrecompileInput<'_>| {
            if beacon_order.compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                return Err(PrecompileError::Fatal(
                    "EIP-4788 did not execute after EIP-2935".to_string(),
                ));
            }
            Ok(PrecompileOutput {
                status: PrecompileStatus::Success,
                bytes: Bytes::new(),
                gas_used: 0,
                gas_refunded: 0,
                state_gas_used: 0,
                state_gas_spilled: 0,
                reservoir: input.reservoir,
            })
        });

        vec![(HISTORY_STORAGE_ADDRESS, history), (BEACON_ROOTS_ADDRESS, beacon)]
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ethereum_block_start_transitions_use_consensus_order() {
    let order = Arc::new(AtomicUsize::new(0));
    let node_config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Prague.into()))
        .with_precompile_factory(OrderedBlockStartPrecompiles(Arc::clone(&order)));
    let (api, _) = spawn(node_config).await;

    api.mine_one().await.unwrap();

    assert_eq!(order.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn tempo_spec_id_does_not_enable_ethereum_block_transitions() {
    let order = Arc::new(AtomicUsize::new(0));
    let node_config = NodeConfig::test_tempo()
        .with_hardfork(Some(TempoHardfork::Genesis.into()))
        .with_precompile_factory(OrderedBlockStartPrecompiles(Arc::clone(&order)));
    let (api, handle) = spawn(node_config).await;

    handle.http_provider().get_block_by_number(BlockNumberOrTag::Pending).await.unwrap();
    api.simulate_v1(
        SimulatePayload { block_state_calls: vec![SimBlock::default()], ..Default::default() },
        None,
    )
    .await
    .unwrap();

    api.mine_one().await.unwrap();

    assert_eq!(order.load(Ordering::SeqCst), 0);
}

#[cfg(feature = "optimism")]
#[tokio::test(flavor = "multi_thread")]
async fn optimism_spec_id_does_not_enable_ethereum_block_transitions() {
    let order = Arc::new(AtomicUsize::new(0));
    let node_config = NodeConfig::test()
        .with_networks(NetworkConfigs::with_optimism())
        .with_hardfork(Some(OpHardfork::Isthmus.into()))
        .with_precompile_factory(OrderedBlockStartPrecompiles(Arc::clone(&order)));
    let (api, handle) = spawn(node_config).await;

    handle.http_provider().get_block_by_number(BlockNumberOrTag::Pending).await.unwrap();
    api.simulate_v1(
        SimulatePayload { block_state_calls: vec![SimBlock::default()], ..Default::default() },
        None,
    )
    .await
    .unwrap();

    api.mine_one().await.unwrap();

    assert_eq!(order.load(Ordering::SeqCst), 0);
}

#[derive(Debug)]
struct CountingPostBlockPrecompiles(Arc<AtomicUsize>);

impl PrecompileFactory for CountingPostBlockPrecompiles {
    fn precompiles(&self) -> Vec<(Address, DynPrecompile)> {
        [WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS, CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS]
            .into_iter()
            .map(|address| {
                let calls = Arc::clone(&self.0);
                let precompile = DynPrecompile::from(move |input: PrecompileInput<'_>| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(PrecompileOutput {
                        status: PrecompileStatus::Success,
                        bytes: Bytes::new(),
                        gas_used: 0,
                        gas_refunded: 0,
                        state_gas_used: 0,
                        state_gas_spilled: 0,
                        reservoir: input.reservoir,
                    })
                });
                (address, precompile)
            })
            .collect()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_block_paths_run_post_transitions_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let node_config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Prague.into()))
        .with_precompile_factory(CountingPostBlockPrecompiles(Arc::clone(&calls)));
    let (api, handle) = spawn(node_config).await;

    handle.http_provider().get_block_by_number(BlockNumberOrTag::Pending).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    api.simulate_v1(
        SimulatePayload { block_state_calls: vec![SimBlock::default()], ..Default::default() },
        None,
    )
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);

    api.mine_one().await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 6);
}

#[tokio::test(flavor = "multi_thread")]
async fn transaction_prefix_replay_does_not_drain_post_block_queues() {
    let calls = Arc::new(AtomicUsize::new(0));
    let node_config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Prague.into()))
        .with_precompile_factory(CountingPostBlockPrecompiles(Arc::clone(&calls)));
    let (api, handle) = spawn(node_config).await;
    let provider = handle.http_provider();
    let mut wallets = handle.dev_wallets();
    let from = wallets.next().unwrap().address();
    let to = wallets.next().unwrap().address();
    let receipt = provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(from).to(to).value(U256::from(1)),
        ))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    calls.store(0, Ordering::SeqCst);

    api.trace_replay_transaction(
        receipt.transaction_hash,
        [TraceType::Trace].into_iter().collect(),
    )
    .await
    .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_contract_deployed_at_genesis() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    let code = provider.get_code_at(HISTORY_STORAGE_ADDRESS).await.unwrap();
    assert!(!code.is_empty(), "EIP-2935 history storage contract should be deployed at genesis");
}

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_stores_parent_block_hash() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    // Mine a few blocks so there are parent hashes to store
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();

    // Block 1's hash should be stored when block 2 was mined
    let block1 = provider
        .get_block_by_number(BlockNumberOrTag::from(1))
        .await
        .unwrap()
        .expect("block 1 should exist");
    let block1_hash = block1.header.hash;

    // Query the history storage contract for block 1's hash.
    // The EIP-2935 contract uses raw calldata (not ABI-encoded): pass the block number
    // as a 32-byte big-endian word directly.
    let call_data: [u8; 32] = U256::from(1).to_be_bytes();
    let tx = TransactionRequest::default().with_to(HISTORY_STORAGE_ADDRESS).with_input(call_data);
    let result = provider.call(tx.into()).await.unwrap();

    let stored_hash = alloy_primitives::B256::from_slice(&result);
    assert_eq!(stored_hash, block1_hash, "EIP-2935 contract should store parent block hash");
}

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_no_system_call_on_genesis() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    // At genesis (block 0), the contract should exist but no system call should have
    // written any parent hash into its storage. Check raw storage slot 0 directly.
    let slot = provider.get_storage_at(HISTORY_STORAGE_ADDRESS, U256::from(0)).await.unwrap();
    assert_eq!(slot, U256::ZERO, "No hash should be stored in the contract at genesis");
}

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_not_deployed_before_prague() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()));
    let (_api, handle) = spawn(node_config).await;
    let provider = http_provider(&handle.http_endpoint());

    let code = provider.get_code_at(HISTORY_STORAGE_ADDRESS).await.unwrap();
    assert!(code.is_empty(), "EIP-2935 contract should NOT be deployed before Prague");
}

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_local_block_replay_applies_pre_execution_changes() {
    let node_config = NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()));
    let (api, handle) = spawn(node_config).await;
    let provider = handle.http_provider();
    let probe = address!("0000000000000000000000000000000000001000");

    // Forward the calldata to the history contract and store the returned parent hash at slot zero
    // only when it is non-zero.
    api.anvil_set_code(
        probe,
        bytes!(
            "60205f5f3760205f60205f730000f90827f1c53a10cb7a02335b1753200029355afa505f518015602d575f55005b00"
        ),
    )
    .await
    .unwrap();
    api.mine_one().await.unwrap();

    let parent = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .expect("parent block should exist");
    api.anvil_set_auto_mine(false).await.unwrap();

    let from = handle.dev_wallets().next().unwrap().address();
    let call_data: [u8; 32] = U256::from(parent.header.number).to_be_bytes();
    let pending = provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default()
                .from(from)
                .to(probe)
                .with_input(call_data)
                .with_gas_limit(100_000),
        ))
        .await
        .unwrap();
    api.mine_one().await.unwrap();
    let receipt = pending.get_receipt().await.unwrap();
    let block_number = receipt.block_number.unwrap();

    let replay = api
        .trace_replay_block_transactions(
            block_number.into(),
            [TraceType::StateDiff].into_iter().collect(),
        )
        .await
        .unwrap();
    let storage = &replay[0]
        .full_trace
        .state_diff
        .as_ref()
        .expect("state diff should be present")
        .get(&probe)
        .expect("probe should change")
        .storage;
    assert_eq!(storage.get(&B256::ZERO), Some(&Delta::changed(B256::ZERO, parent.header.hash)));

    let opcode_gas: Option<BlockOpcodeGas> = provider
        .raw_request("trace_blockOpcodeGas".into(), (BlockId::number(block_number),))
        .await
        .unwrap();
    assert!(opcode_gas.expect("block should exist").contains("SSTORE"));
}

#[tokio::test(flavor = "multi_thread")]
async fn eip2935_local_block_replay_propagates_pre_execution_errors() {
    let fail = Arc::new(AtomicBool::new(false));
    let node_config = NodeConfig::test()
        .with_hardfork(Some(EthereumHardfork::Prague.into()))
        .with_precompile_factory(FailingHistoryPrecompile(Arc::clone(&fail)));
    let (api, handle) = spawn(node_config).await;
    let provider = handle.http_provider();

    let mut wallets = handle.dev_wallets();
    let from = wallets.next().unwrap().address();
    let to = wallets.next().unwrap().address();
    let receipt = provider
        .send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(from).to(to).value(U256::from(1)),
        ))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let block_number = receipt.block_number.unwrap();
    api.mine_one().await.unwrap();
    let empty_block_number = provider.get_block_number().await.unwrap();
    assert_eq!(empty_block_number, block_number + 1);

    fail.store(true, Ordering::SeqCst);

    let block_replay: Result<Vec<TraceResultsWithTransactionHash>, _> = provider
        .client()
        .request("trace_replayBlockTransactions", (block_number, vec![TraceType::Trace]))
        .await;
    let transaction_replay: Result<TraceResults, _> = provider
        .client()
        .request("trace_replayTransaction", (receipt.transaction_hash, vec![TraceType::Trace]))
        .await;
    let transaction_opcode_replay: Result<Option<TransactionOpcodeGas>, _> = provider
        .raw_request("trace_transactionOpcodeGas".into(), (receipt.transaction_hash,))
        .await;
    let opcode_replay: Result<Option<BlockOpcodeGas>, _> =
        provider.raw_request("trace_blockOpcodeGas".into(), (BlockId::number(block_number),)).await;
    let empty_opcode_replay: Result<Option<BlockOpcodeGas>, _> = provider
        .raw_request("trace_blockOpcodeGas".into(), (BlockId::number(empty_block_number),))
        .await;

    for error in [
        block_replay.unwrap_err(),
        transaction_replay.unwrap_err(),
        transaction_opcode_replay.unwrap_err(),
        opcode_replay.unwrap_err(),
        empty_opcode_replay.unwrap_err(),
    ] {
        let response = error.as_error_resp().expect("should return a JSON-RPC error");
        assert_eq!(response.code, -32603);
        assert!(response.message.contains(REPLAY_PRE_EXECUTION_ERROR), "{response:?}");
    }

    let genesis_opcode_gas: Option<BlockOpcodeGas> =
        provider.raw_request("trace_blockOpcodeGas".into(), (BlockId::number(0),)).await.unwrap();
    assert!(genesis_opcode_gas.expect("genesis block should exist").transactions.is_empty());
    assert_eq!(provider.get_block_number().await.unwrap(), empty_block_number);
}
