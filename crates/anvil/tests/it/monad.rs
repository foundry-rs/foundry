use alloy_consensus::{
    SidecarBuilder, SignableTransaction, SimpleCoder, Transaction, TxEip1559, TxLegacy,
    transaction::TxEip7702,
};
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_network::{
    ReceiptResponse, TransactionBuilder, TransactionBuilder4844, TransactionResponse, TxSignerSync,
};
use alloy_primitives::{Address, B256, Bytes, Signature, TxKind, U256, address, hex};
use alloy_provider::{
    Provider,
    ext::{DebugApi, TraceApi},
};
use alloy_rpc_types::{
    AccessList, AccessListItem, Authorization, BlockId, BlockNumberOrTag, Index,
    TransactionRequest,
    anvil::Forking,
    simulate::{SimBlock, SimulatePayload},
    state::{AccountOverride, StateOverride},
    trace::{
        geth::{GethDebugTracingCallOptions, GethDebugTracingOptions, GethTrace},
        opcode::{BlockOpcodeGas, TransactionOpcodeGas},
        parity::{Action, TraceResults, TraceType},
    },
};
use alloy_rpc_types_eth::{AccountInfo as RpcAccountInfo, Bundle, EthCallResponse};
use alloy_serde::WithOtherFields;
use alloy_signer::Signer;
use alloy_sol_types::{SolCall, SolEvent};
use anvil::{
    NodeConfig, NodeHandle,
    eth::{
        error::BlockchainError,
        pool::transactions::{PoolTransaction, TransactionOrder},
    },
    spawn,
};
use anvil_core::{
    eth::transaction::PendingTransaction,
    types::{ReorgOptions, TransactionData},
};
use foundry_evm::hardfork::{FoundryHardfork, MonadHardfork};
use foundry_evm_networks::NetworkConfigs;
use foundry_primitives::FoundryTxEnvelope;
use foundry_test_utils::rpc::spawn_canonical_monad_system_rpc;
use monad_revm::{
    MONAD_TESTNET_CHAIN_ID,
    staking::{
        constants::SYSTEM_ADDRESS,
        interface::IMonadStaking::{
            EpochChanged, ValidatorRewarded, syscallOnEpochChangeCall, syscallRewardCall,
            syscallSnapshotCall,
        },
        storage::{
            consensus_view_key, global_slots, val_id_secp_key, validator_key, validator_offsets,
        },
    },
};
use std::sync::Arc;
const STAKING_ADDRESS: Address = address!("0x0000000000000000000000000000000000001000");
const RESERVE_BALANCE_ADDRESS: Address = address!("0x0000000000000000000000000000000000001001");
const RESERVE_PROBE_ADDRESS: Address = address!("0x0000000000000000000000000000000000002000");
const BALANCE_PROBE_ADDRESS: Address = address!("0x0000000000000000000000000000000000002001");
const CHAIN_ID_PROBE_ADDRESS: Address = address!("0x0000000000000000000000000000000000002002");
const CLZ_PROBE_ADDRESS: Address = address!("0x0000000000000000000000000000000000002003");
const STORAGE_GAS_PROBE_ADDRESS: Address = address!("0x0000000000000000000000000000000000002004");
const DIPPED_INTO_RESERVE_SELECTOR: [u8; 4] = hex!("3a61584e");
const RESERVE_RETURN_PROBE_CODE: [u8; 25] =
    hex!("633a61584e5f5260205f6004601c5f6110015af15060205ff3");
const EIP170_CODE_SIZE_LIMIT: usize = 0x6000;
const EIP3860_INITCODE_SIZE_LIMIT: usize = 0xc000;
const EIP7825_TX_GAS_LIMIT_CAP: u64 = 0x1000000;
const MONAD_TX_GAS_LIMIT_CAP: u64 = 30_000_000;

#[tokio::test(flavor = "multi_thread")]
async fn monad_nine_exposes_reserve_balance_precompile_for_calls() {
    let config = NodeConfig::test_monad().with_hardfork(Some(MonadHardfork::MonadNine.into()));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    let tx = TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR);
    let result = provider.call(tx.into()).await.unwrap();

    assert_eq!(result, Bytes::from(vec![0; 32]));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_ten_applies_mip8_storage_gas() {
    for (config, hardfork, read_delta, write_delta) in [
        (
            NodeConfig::test_monad().with_hardfork(Some(MonadHardfork::MonadNine.into())),
            MonadHardfork::MonadNine,
            0,
            0,
        ),
        (NodeConfig::test_monad(), MonadHardfork::MonadTen, 8_000, 10_800),
    ] {
        let (api, handle) = spawn(config).await;
        let provider = handle.http_provider();

        assert_eq!(api.anvil_node_info().await.unwrap().hard_fork, hardfork.to_string());

        api.anvil_set_code(STORAGE_GAS_PROBE_ADDRESS, storage_read_probe_code(127)).await.unwrap();
        let same_page_read =
            storage_probe_gas(provider.call(storage_gas_probe_call()).await.unwrap());
        api.anvil_set_code(STORAGE_GAS_PROBE_ADDRESS, storage_read_probe_code(128)).await.unwrap();
        let different_page_read =
            storage_probe_gas(provider.call(storage_gas_probe_call()).await.unwrap());
        assert_eq!(different_page_read - same_page_read, read_delta);

        api.anvil_set_code(STORAGE_GAS_PROBE_ADDRESS, storage_write_probe_code(1)).await.unwrap();
        let same_page_write =
            storage_probe_gas(provider.call(storage_gas_probe_call()).await.unwrap());
        api.anvil_set_code(STORAGE_GAS_PROBE_ADDRESS, storage_write_probe_code(128)).await.unwrap();
        let different_page_write =
            storage_probe_gas(provider.call(storage_gas_probe_call()).await.unwrap());
        assert_eq!(different_page_write - same_page_write, write_delta);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_ten_deduplicates_access_list_storage_pages() {
    let one = B256::from(U256::ONE.to_be_bytes::<32>());
    let two = B256::from(U256::from(2).to_be_bytes::<32>());
    let page_one = B256::from(U256::from(129).to_be_bytes::<32>());

    for (config, expected_same_page_keys, gas_delta) in [
        (
            NodeConfig::test_monad().with_hardfork(Some(MonadHardfork::MonadNine.into())),
            vec![one, two],
            0,
        ),
        (NodeConfig::test_monad(), vec![one], 1_900),
    ] {
        let (api, handle) = spawn(config).await;
        let provider = handle.http_provider();

        api.anvil_set_code(STORAGE_GAS_PROBE_ADDRESS, storage_access_list_probe_code(2, 1))
            .await
            .unwrap();
        let same_page = provider.create_access_list(&storage_gas_probe_call()).await.unwrap();
        assert_eq!(
            same_page.access_list,
            AccessList::from(vec![AccessListItem {
                address: STORAGE_GAS_PROBE_ADDRESS,
                storage_keys: expected_same_page_keys,
            }])
        );

        api.anvil_set_code(STORAGE_GAS_PROBE_ADDRESS, storage_access_list_probe_code(129, 1))
            .await
            .unwrap();
        let different_page = provider.create_access_list(&storage_gas_probe_call()).await.unwrap();
        assert_eq!(
            different_page.access_list,
            AccessList::from(vec![AccessListItem {
                address: STORAGE_GAS_PROBE_ADDRESS,
                storage_keys: vec![one, page_one],
            }])
        );
        assert_eq!(different_page.gas_used - same_page.gas_used, U256::from(gas_delta));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_ten_rpc_simulations_apply_mip8_storage_gas() {
    for (config, read_delta, write_delta) in [
        (NodeConfig::test_monad().with_hardfork(Some(MonadHardfork::MonadNine.into())), 0, 0),
        (NodeConfig::test_monad(), 8_000, 10_800),
    ] {
        let (api, handle) = spawn(config).await;
        let provider = handle.http_provider();

        api.anvil_set_code(STORAGE_GAS_PROBE_ADDRESS, storage_read_probe_code(127)).await.unwrap();
        let same_page_read_estimate =
            provider.estimate_gas(storage_gas_probe_call()).await.unwrap();
        let same_page_read_trace: TraceResults = provider
            .client()
            .request(
                "trace_call",
                (storage_gas_probe_call(), vec![TraceType::Trace], BlockId::latest()),
            )
            .await
            .unwrap();

        api.anvil_set_code(STORAGE_GAS_PROBE_ADDRESS, storage_read_probe_code(128)).await.unwrap();
        let different_page_read_estimate =
            provider.estimate_gas(storage_gas_probe_call()).await.unwrap();
        let different_page_read_trace: TraceResults = provider
            .client()
            .request(
                "trace_call",
                (storage_gas_probe_call(), vec![TraceType::Trace], BlockId::latest()),
            )
            .await
            .unwrap();

        assert_eq!(different_page_read_estimate - same_page_read_estimate, read_delta);
        assert_eq!(
            root_trace_gas(&different_page_read_trace) - root_trace_gas(&same_page_read_trace),
            read_delta
        );

        api.anvil_set_code(STORAGE_GAS_PROBE_ADDRESS, storage_write_probe_code(1)).await.unwrap();
        let same_page_write_estimate =
            provider.estimate_gas(storage_gas_probe_call()).await.unwrap();
        let same_page_write_trace: TraceResults = provider
            .client()
            .request(
                "trace_call",
                (storage_gas_probe_call(), vec![TraceType::Trace], BlockId::latest()),
            )
            .await
            .unwrap();

        api.anvil_set_code(STORAGE_GAS_PROBE_ADDRESS, storage_write_probe_code(128)).await.unwrap();
        let different_page_write_estimate =
            provider.estimate_gas(storage_gas_probe_call()).await.unwrap();
        let different_page_write_trace: TraceResults = provider
            .client()
            .request(
                "trace_call",
                (storage_gas_probe_call(), vec![TraceType::Trace], BlockId::latest()),
            )
            .await
            .unwrap();

        assert_eq!(different_page_write_estimate - same_page_write_estimate, write_delta);
        assert_eq!(
            root_trace_gas(&different_page_write_trace) - root_trace_gas(&same_page_write_trace),
            write_delta
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_call_uses_parent_sender_context() {
    let (api, handle) = spawn(monad_nine_config()).await;
    let provider = handle.http_provider();
    let sender = provider.get_accounts().await.unwrap()[0];

    api.anvil_set_code(RESERVE_PROBE_ADDRESS, RESERVE_RETURN_PROBE_CODE.into()).await.unwrap();
    api.anvil_set_balance(sender, mon(13)).await.unwrap();

    provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(RESERVE_PROBE_ADDRESS)
                .with_value(mon(1))
                .with_gas_limit(100_000)
                .into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let result = provider
        .call(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(RESERVE_PROBE_ADDRESS)
                .with_value(mon(3))
                .with_gas_limit(100_000)
                .into(),
        )
        .await
        .unwrap();

    assert_eq!(result, Bytes::from(U256::ONE.to_be_bytes::<32>()));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_simulate_tracks_current_block_senders() {
    let (api, handle) = spawn(monad_nine_config()).await;
    let sender = handle.http_provider().get_accounts().await.unwrap()[0];

    api.anvil_set_code(RESERVE_PROBE_ADDRESS, RESERVE_RETURN_PROBE_CODE.into()).await.unwrap();
    api.anvil_set_balance(sender, mon(12)).await.unwrap();

    let calls = [mon(2), mon(1)]
        .into_iter()
        .map(|value| {
            TransactionRequest::default()
                .with_from(sender)
                .with_to(RESERVE_PROBE_ADDRESS)
                .with_value(value)
                .with_gas_limit(100_000)
        })
        .collect();
    let blocks = api
        .simulate_v1(
            SimulatePayload {
                block_state_calls: vec![SimBlock { calls, ..Default::default() }],
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();

    assert_eq!(blocks[0].calls[0].return_data, Bytes::from(U256::ZERO.to_be_bytes::<32>()));
    assert_eq!(blocks[0].calls[1].return_data, Bytes::from(U256::ONE.to_be_bytes::<32>()));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_simulate_ages_reserve_participants_across_empty_blocks() {
    let (api, handle) = spawn(monad_nine_config()).await;
    let sender = handle.http_provider().get_accounts().await.unwrap()[0];

    api.anvil_set_code(RESERVE_PROBE_ADDRESS, RESERVE_RETURN_PROBE_CODE.into()).await.unwrap();
    api.anvil_set_balance(sender, mon(12)).await.unwrap();

    let request = |value| {
        TransactionRequest::default()
            .with_from(sender)
            .with_to(RESERVE_PROBE_ADDRESS)
            .with_value(value)
            .with_gas_limit(100_000)
    };
    let blocks = api
        .simulate_v1(
            SimulatePayload {
                block_state_calls: vec![
                    SimBlock { calls: vec![request(mon(2))], ..Default::default() },
                    SimBlock::default(),
                    SimBlock::default(),
                    SimBlock { calls: vec![request(mon(1))], ..Default::default() },
                ],
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();

    assert_eq!(blocks[0].calls[0].return_data, Bytes::from(U256::ZERO.to_be_bytes::<32>()));
    assert_eq!(blocks[3].calls[0].return_data, Bytes::from(U256::ZERO.to_be_bytes::<32>()));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_call_many_ages_reserve_participants_across_empty_bundles() {
    let (api, handle) = spawn(monad_nine_config()).await;
    let provider = handle.http_provider();
    let sender = provider.get_accounts().await.unwrap()[0];
    api.anvil_set_code(RESERVE_PROBE_ADDRESS, RESERVE_RETURN_PROBE_CODE.into()).await.unwrap();
    api.anvil_set_balance(sender, mon(12)).await.unwrap();

    let request = |value| {
        WithOtherFields::new(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(RESERVE_PROBE_ADDRESS)
                .with_value(value)
                .with_gas_limit(100_000),
        )
    };
    let bundles = vec![
        Bundle { transactions: vec![request(mon(2))], block_override: None },
        Bundle { transactions: Vec::new(), block_override: None },
        Bundle { transactions: Vec::new(), block_override: None },
        Bundle { transactions: vec![request(mon(1))], block_override: None },
    ];

    let response: Vec<Vec<EthCallResponse>> =
        provider.client().request("eth_callMany", (bundles,)).await.unwrap();

    assert_eq!(
        response[0][0].clone().ensure_ok().unwrap(),
        Bytes::from(U256::ZERO.to_be_bytes::<32>())
    );
    assert_eq!(
        response[3][0].clone().ensure_ok().unwrap(),
        Bytes::from(U256::ZERO.to_be_bytes::<32>())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_mining_tracks_current_and_ancestor_senders() {
    let (api, handle) = spawn(monad_nine_config()).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();
    let parent_sender = accounts[0];
    let grandparent_sender = accounts[1];
    let current_sender = accounts[2];
    let initial_balance = U256::from(12_000_000_000_000_000_000u128);
    let first_value = U256::from(2_000_000_000_000_000_000u128);
    let second_value = U256::from(1_000_000_000_000_000_000u128);

    // Calls dippedIntoReserve(), then stores the returned bool at the calldata-provided slot.
    api.anvil_set_code(
        RESERVE_PROBE_ADDRESS,
        Bytes::from(hex!("633a61584e5f5260205f6004601c5f6110015af1505f515f355500")),
    )
    .await
    .unwrap();
    for sender in [parent_sender, grandparent_sender, current_sender] {
        api.anvil_set_balance(sender, initial_balance).await.unwrap();
    }

    provider
        .send_transaction(reserve_probe_tx(parent_sender, 0, 0, first_value).into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert_eq!(
        provider.get_storage_at(RESERVE_PROBE_ADDRESS, U256::ZERO).await.unwrap(),
        U256::ZERO
    );

    provider
        .send_transaction(reserve_probe_tx(parent_sender, 1, 1, second_value).into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert_eq!(
        provider.get_storage_at(RESERVE_PROBE_ADDRESS, U256::from(1)).await.unwrap(),
        U256::ONE
    );

    provider
        .send_transaction(reserve_probe_tx(grandparent_sender, 0, 2, first_value).into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert_eq!(
        provider.get_storage_at(RESERVE_PROBE_ADDRESS, U256::from(2)).await.unwrap(),
        U256::ZERO
    );
    api.mine_one().await.unwrap();
    provider
        .send_transaction(reserve_probe_tx(grandparent_sender, 1, 3, second_value).into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert_eq!(
        provider.get_storage_at(RESERVE_PROBE_ADDRESS, U256::from(3)).await.unwrap(),
        U256::ONE
    );

    api.anvil_set_auto_mine(false).await.unwrap();
    let _ = provider
        .send_transaction(reserve_probe_tx(current_sender, 0, 4, first_value).into())
        .await
        .unwrap();
    let second_pending = provider
        .send_transaction(reserve_probe_tx(current_sender, 1, 5, second_value).into())
        .await
        .unwrap();
    api.mine_one().await.unwrap();
    let second_receipt = second_pending.get_receipt().await.unwrap();
    assert_eq!(
        provider.get_storage_at(RESERVE_PROBE_ADDRESS, U256::from(4)).await.unwrap(),
        U256::ZERO
    );
    assert_eq!(
        provider.get_storage_at(RESERVE_PROBE_ADDRESS, U256::from(5)).await.unwrap(),
        U256::ONE
    );

    let replay: TraceResults = provider
        .client()
        .request(
            "trace_replayTransaction",
            (second_receipt.transaction_hash, vec![TraceType::StateDiff]),
        )
        .await
        .unwrap();
    let slot = B256::from(U256::from(5).to_be_bytes::<32>());
    let state_diff = replay.state_diff.unwrap();
    let delta = &state_diff.get(&RESERVE_PROBE_ADDRESS).unwrap().storage[&slot];
    let replayed_value =
        delta.as_added().copied().or_else(|| delta.as_changed().map(|change| change.to)).unwrap();
    assert_eq!(replayed_value, B256::from(U256::ONE.to_be_bytes::<32>()));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_reorg_replays_protocol_system_envelopes() {
    const BLOCK_AUTHOR: Address = address!("0x1111111111111111111111111111111111111111");
    const VALIDATOR_AUTH: Address = address!("0x2222222222222222222222222222222222222222");
    const VALIDATOR_ID: u64 = 7;
    const SYSTEM_NONCE: u64 = 11;

    let config = monad_nine_config().with_transaction_order(TransactionOrder::Fifo);
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();
    let participant = accounts[0];
    let recipient = accounts[1];
    let transfer = U256::from(123u64);
    let reward = mon(25);
    let initial_system_balance = mon(100);
    let initial_staking_balance = mon(3);
    let initial_recipient_balance = provider.get_balance(recipient).await.unwrap();

    api.anvil_set_nonce(SYSTEM_ADDRESS, U256::from(SYSTEM_NONCE)).await.unwrap();
    api.anvil_set_balance(SYSTEM_ADDRESS, initial_system_balance).await.unwrap();
    api.anvil_set_balance(STAKING_ADDRESS, initial_staking_balance).await.unwrap();
    api.anvil_set_storage_at(
        STAKING_ADDRESS,
        val_id_secp_key(&BLOCK_AUTHOR),
        storage_value(left_aligned_u64(VALIDATOR_ID)),
    )
    .await
    .unwrap();
    api.anvil_set_storage_at(
        STAKING_ADDRESS,
        consensus_view_key(VALIDATOR_ID, 0),
        storage_value(mon(100)),
    )
    .await
    .unwrap();
    api.anvil_set_storage_at(
        STAKING_ADDRESS,
        validator_key(VALIDATOR_ID, validator_offsets::ADDRESS_FLAGS),
        storage_value(address_and_flags(VALIDATOR_AUTH, 0)),
    )
    .await
    .unwrap();

    // Keep the configured staking state below the common ancestor of the one-block reorg.
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();
    let original_height = provider.get_block_number().await.unwrap();

    let reward_tx = monad_protocol_transaction(
        SYSTEM_NONCE,
        reward,
        syscallRewardCall { blockAuthor: BLOCK_AUTHOR }.abi_encode().into(),
    );
    let ordinary_tx = TransactionRequest::default()
        .with_from(participant)
        .with_to(recipient)
        .with_nonce(0)
        .with_value(transfer)
        .with_gas_limit(21_000)
        .with_gas_price(2_000_000_000);
    let snapshot_tx = monad_protocol_transaction(
        SYSTEM_NONCE + 1,
        U256::ZERO,
        syscallSnapshotCall {}.abi_encode().into(),
    );
    let epoch_tx = monad_protocol_transaction(
        SYSTEM_NONCE + 2,
        U256::ZERO,
        syscallOnEpochChangeCall { epoch: 1 }.abi_encode().into(),
    );

    api.anvil_reorg(ReorgOptions {
        depth: 1,
        tx_block_pairs: vec![
            (TransactionData::JSON(reward_tx), 0),
            (TransactionData::JSON(ordinary_tx), 0),
            (TransactionData::JSON(snapshot_tx), 0),
            (TransactionData::JSON(epoch_tx), 0),
        ],
    })
    .await
    .unwrap();

    assert_eq!(provider.get_block_number().await.unwrap(), original_height);
    let block =
        provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
    let transactions = block.transactions.as_transactions().unwrap();
    assert_eq!(transactions.len(), 4);
    assert_eq!(transactions[0].from(), SYSTEM_ADDRESS);
    assert_eq!(transactions[1].from(), participant);
    assert_eq!(transactions[2].from(), SYSTEM_ADDRESS);
    assert_eq!(transactions[3].from(), SYSTEM_ADDRESS);
    assert_eq!(transactions[0].to(), Some(STAKING_ADDRESS));
    assert_eq!(transactions[1].to(), Some(recipient));
    assert_eq!(transactions[2].to(), Some(STAKING_ADDRESS));
    assert_eq!(transactions[3].to(), Some(STAKING_ADDRESS));
    assert_eq!(&transactions[0].input()[..4], syscallRewardCall::SELECTOR);
    assert_eq!(&transactions[2].input()[..4], syscallSnapshotCall::SELECTOR);
    assert_eq!(&transactions[3].input()[..4], syscallOnEpochChangeCall::SELECTOR);

    let hashes = transactions.iter().map(TransactionResponse::tx_hash).collect::<Vec<_>>();
    let mut receipts = Vec::with_capacity(hashes.len());
    for hash in hashes {
        receipts.push(provider.get_transaction_receipt(hash).await.unwrap().unwrap());
    }
    for receipt in &receipts {
        assert!(receipt.status());
    }
    for receipt in [&receipts[0], &receipts[2], &receipts[3]] {
        assert_eq!(receipt.gas_used, 0);
        assert_eq!(receipt.effective_gas_price, 0);
    }
    assert_eq!(block.header.gas_used, receipts[1].gas_used);
    assert_eq!(receipts[0].inner.inner.logs().len(), 1);
    assert_eq!(receipts[0].inner.inner.logs()[0].topics()[0], ValidatorRewarded::SIGNATURE_HASH);
    assert!(receipts[2].inner.inner.logs().is_empty());
    assert_eq!(receipts[3].inner.inner.logs().len(), 1);
    assert_eq!(receipts[3].inner.inner.logs()[0].topics()[0], EpochChanged::SIGNATURE_HASH);

    assert_eq!(provider.get_transaction_count(SYSTEM_ADDRESS).await.unwrap(), SYSTEM_NONCE + 3);
    assert_eq!(provider.get_balance(SYSTEM_ADDRESS).await.unwrap(), initial_system_balance);
    assert_eq!(
        provider.get_balance(STAKING_ADDRESS).await.unwrap(),
        initial_staking_balance + reward
    );
    assert_eq!(
        provider.get_balance(recipient).await.unwrap(),
        initial_recipient_balance + transfer
    );
    assert_eq!(
        provider.get_storage_at(STAKING_ADDRESS, global_slots::PROPOSER_VAL_ID).await.unwrap(),
        left_aligned_u64(VALIDATOR_ID)
    );
    assert_eq!(
        provider
            .get_storage_at(
                STAKING_ADDRESS,
                validator_key(VALIDATOR_ID, validator_offsets::UNCLAIMED_REWARDS),
            )
            .await
            .unwrap(),
        reward
    );
    assert_eq!(
        provider.get_storage_at(STAKING_ADDRESS, global_slots::IN_BOUNDARY).await.unwrap(),
        U256::ZERO
    );
    assert_eq!(
        provider.get_storage_at(STAKING_ADDRESS, global_slots::EPOCH).await.unwrap(),
        left_aligned_u64(1)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_reorg_replays_raw_protocol_envelope_with_non_system_signature() {
    const SYSTEM_NONCE: u64 = 4;

    let (api, handle) = spawn(monad_nine_config()).await;
    let provider = handle.http_provider();
    let chain_id = provider.get_chain_id().await.unwrap();
    let wallet = handle.dev_wallets().next().unwrap().with_chain_id(Some(chain_id));

    api.anvil_set_nonce(SYSTEM_ADDRESS, U256::from(SYSTEM_NONCE)).await.unwrap();
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();

    let mut transaction = TxLegacy {
        chain_id: Some(chain_id),
        nonce: SYSTEM_NONCE,
        gas_price: 0,
        gas_limit: 0,
        to: TxKind::Call(STAKING_ADDRESS),
        value: U256::ZERO,
        input: syscallSnapshotCall {}.abi_encode().into(),
    };
    let signature = wallet.sign_transaction_sync(&mut transaction).unwrap();
    let transaction = transaction.into_signed(signature);
    let mut encoded = Vec::new();
    transaction.eip2718_encode(&mut encoded);

    let decoded = FoundryTxEnvelope::decode_2718(&mut encoded.as_slice()).unwrap();
    let normally_recovered = PendingTransaction::new(decoded).unwrap();
    let recovered_sender = *normally_recovered.sender();
    assert_ne!(recovered_sender, SYSTEM_ADDRESS);
    let transaction_hash = *normally_recovered.hash();

    api.anvil_reorg(ReorgOptions {
        depth: 1,
        tx_block_pairs: vec![(TransactionData::Raw(encoded.clone().into()), 0)],
    })
    .await
    .unwrap();

    let block_number = provider.get_block_number().await.unwrap();
    let block = provider
        .get_block_by_number(BlockNumberOrTag::Number(block_number))
        .full()
        .await
        .unwrap()
        .unwrap();
    let block_hash = block.header.hash;
    let transactions = block.transactions.as_transactions().unwrap();
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].tx_hash(), transaction_hash);
    assert_eq!(transactions[0].from(), SYSTEM_ADDRESS);

    let receipt = provider.get_transaction_receipt(transaction_hash).await.unwrap().unwrap();
    assert!(receipt.status());
    assert_eq!(receipt.gas_used, 0);
    assert_eq!(receipt.effective_gas_price, 0);
    assert_eq!(provider.get_transaction_count(SYSTEM_ADDRESS).await.unwrap(), SYSTEM_NONCE + 1);

    let local_block = api.backend.get_block(block_number).unwrap();
    let mut round_trip = Vec::new();
    local_block.body.transactions[0].encode_2718(&mut round_trip);
    assert_eq!(round_trip, encoded);

    let state = api.serialized_state(false).await.unwrap();
    let participants = &state.monad_block_participants[&block_hash];
    assert!(participants.contains(&SYSTEM_ADDRESS));
    assert!(!participants.contains(&recovered_sender));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_reorg_replays_raw_protocol_envelope_with_unrecoverable_signature() {
    const SYSTEM_NONCE: u64 = 4;

    let (api, handle) = spawn(monad_nine_config()).await;
    let provider = handle.http_provider();
    let chain_id = provider.get_chain_id().await.unwrap();

    api.anvil_set_nonce(SYSTEM_ADDRESS, U256::from(SYSTEM_NONCE)).await.unwrap();
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();

    let transaction = TxLegacy {
        chain_id: Some(chain_id),
        nonce: SYSTEM_NONCE,
        gas_price: 0,
        gas_limit: 0,
        to: TxKind::Call(STAKING_ADDRESS),
        value: U256::ZERO,
        input: syscallSnapshotCall {}.abi_encode().into(),
    }
    .into_signed(Signature::new(U256::ZERO, U256::ZERO, false));
    let transaction_hash = *transaction.hash();
    let mut encoded = Vec::new();
    transaction.eip2718_encode(&mut encoded);

    let decoded = FoundryTxEnvelope::decode_2718(&mut encoded.as_slice()).unwrap();
    assert!(PendingTransaction::new(decoded).is_err());

    api.anvil_reorg(ReorgOptions {
        depth: 1,
        tx_block_pairs: vec![(TransactionData::Raw(encoded.clone().into()), 0)],
    })
    .await
    .unwrap();

    let block_number = provider.get_block_number().await.unwrap();
    let block = provider
        .get_block_by_number(BlockNumberOrTag::Number(block_number))
        .full()
        .await
        .unwrap()
        .unwrap();
    let block_hash = block.header.hash;
    let transactions = block.transactions.as_transactions().unwrap();
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].tx_hash(), transaction_hash);
    assert_eq!(transactions[0].from(), SYSTEM_ADDRESS);

    let receipt = provider.get_transaction_receipt(transaction_hash).await.unwrap().unwrap();
    assert!(receipt.status());
    assert_eq!(receipt.gas_used, 0);
    assert_eq!(receipt.effective_gas_price, 0);
    assert_eq!(provider.get_transaction_count(SYSTEM_ADDRESS).await.unwrap(), SYSTEM_NONCE + 1);

    let local_block = api.backend.get_block(block_number).unwrap();
    let mut round_trip = Vec::new();
    local_block.body.transactions[0].encode_2718(&mut round_trip);
    assert_eq!(round_trip, encoded);

    let state = api.serialized_state(false).await.unwrap();
    let participants = &state.monad_block_participants[&block_hash];
    assert_eq!(participants.len(), 1);
    assert!(participants.contains(&SYSTEM_ADDRESS));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_reorg_rolls_back_failed_protocol_prestate() {
    const UNKNOWN_AUTHOR: Address = address!("0x3333333333333333333333333333333333333333");
    const SYSTEM_NONCE: u64 = 7;

    let (api, handle) = spawn(monad_nine_config()).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();
    let participant = accounts[0];
    let recipient = accounts[1];
    let reward = mon(25);
    let initial_staking_balance = mon(3);
    let initial_recipient_balance = provider.get_balance(recipient).await.unwrap();

    api.anvil_set_nonce(SYSTEM_ADDRESS, U256::from(SYSTEM_NONCE)).await.unwrap();
    api.anvil_set_balance(STAKING_ADDRESS, initial_staking_balance).await.unwrap();
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();

    let failed_reward = monad_protocol_transaction(
        SYSTEM_NONCE,
        reward,
        syscallRewardCall { blockAuthor: UNKNOWN_AUTHOR }.abi_encode().into(),
    );
    let ordinary_tx = TransactionRequest::default()
        .with_from(participant)
        .with_to(recipient)
        .with_nonce(0)
        .with_value(U256::ONE)
        .with_gas_limit(21_000)
        .with_gas_price(2_000_000_000);
    // Reuse the failed envelope's nonce. This can only succeed if replay restores the protocol
    // nonce and mint before the next candidate is validated and executed.
    let snapshot_tx = monad_protocol_transaction(
        SYSTEM_NONCE,
        U256::ZERO,
        syscallSnapshotCall {}.abi_encode().into(),
    );
    let epoch_tx = monad_protocol_transaction(
        SYSTEM_NONCE + 1,
        U256::ZERO,
        syscallOnEpochChangeCall { epoch: 1 }.abi_encode().into(),
    );

    api.anvil_reorg(ReorgOptions {
        depth: 1,
        tx_block_pairs: vec![
            (TransactionData::JSON(failed_reward), 0),
            (TransactionData::JSON(ordinary_tx), 0),
            (TransactionData::JSON(snapshot_tx), 0),
            (TransactionData::JSON(epoch_tx), 0),
        ],
    })
    .await
    .unwrap();

    let block =
        provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
    let transactions = block.transactions.as_transactions().unwrap();
    assert_eq!(transactions.len(), 3);
    assert_eq!(transactions[0].from(), participant);
    assert_eq!(transactions[1].from(), SYSTEM_ADDRESS);
    assert_eq!(transactions[2].from(), SYSTEM_ADDRESS);
    assert_eq!(&transactions[1].input()[..4], syscallSnapshotCall::SELECTOR);
    assert_eq!(&transactions[2].input()[..4], syscallOnEpochChangeCall::SELECTOR);

    let ordinary_hash = transactions[0].tx_hash();
    let parity_traces = provider.trace_transaction(ordinary_hash).await.unwrap();
    assert_eq!(parity_traces.len(), 1);
    let Action::Call(call) = &parity_traces[0].trace.action else {
        panic!("expected ordinary transfer call trace")
    };
    assert_eq!(call.from, participant);
    assert_eq!(call.to, recipient);
    assert_eq!(call.value, U256::ONE);

    let debug_trace = provider
        .debug_trace_transaction(ordinary_hash, GethDebugTracingOptions::default())
        .await
        .unwrap();
    let GethTrace::Default(debug_trace) = debug_trace else {
        panic!("expected default transaction trace")
    };
    assert!(debug_trace.struct_logs.is_empty());

    assert_eq!(provider.get_transaction_count(SYSTEM_ADDRESS).await.unwrap(), SYSTEM_NONCE + 2);
    assert_eq!(provider.get_balance(STAKING_ADDRESS).await.unwrap(), initial_staking_balance);
    assert_eq!(
        provider.get_balance(recipient).await.unwrap(),
        initial_recipient_balance + U256::ONE
    );
    assert_eq!(
        provider.get_storage_at(STAKING_ADDRESS, global_slots::PROPOSER_VAL_ID).await.unwrap(),
        U256::ZERO
    );
    assert_eq!(
        provider.get_storage_at(STAKING_ADDRESS, global_slots::EPOCH).await.unwrap(),
        left_aligned_u64(1)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_reorg_rejects_malformed_and_non_replay_system_envelopes() {
    const SYSTEM_NONCE: u64 = 5;

    let (api, handle) = spawn(monad_nine_config()).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();
    let participant = accounts[0];
    let recipient = accounts[1];
    let chain_id = provider.get_chain_id().await.unwrap();
    let initial_recipient_balance = provider.get_balance(recipient).await.unwrap();
    let initial_staking_balance = provider.get_balance(STAKING_ADDRESS).await.unwrap();

    api.anvil_set_nonce(SYSTEM_ADDRESS, U256::from(SYSTEM_NONCE)).await.unwrap();
    api.mine_one().await.unwrap();
    api.mine_one().await.unwrap();
    let original_height = provider.get_block_number().await.unwrap();

    let malformed_json = TransactionRequest::default()
        .with_from(SYSTEM_ADDRESS)
        .with_to(BALANCE_PROBE_ADDRESS)
        .with_nonce(SYSTEM_NONCE)
        .with_input(Bytes::from(syscallSnapshotCall {}.abi_encode()))
        .with_gas_limit(0)
        .with_gas_price(0);
    let error = api
        .anvil_reorg(ReorgOptions {
            depth: 1,
            tx_block_pairs: vec![(TransactionData::JSON(malformed_json), 0)],
        })
        .await
        .unwrap_err();
    assert!(matches!(error, BlockchainError::NoSignerAvailable));
    assert_eq!(provider.get_block_number().await.unwrap(), original_height);

    let mut malformed_raw = TxLegacy {
        chain_id: Some(chain_id),
        nonce: 0,
        gas_price: 0,
        gas_limit: 0,
        to: TxKind::Call(BALANCE_PROBE_ADDRESS),
        value: U256::ZERO,
        input: syscallSnapshotCall {}.abi_encode().into(),
    };
    let wallet = handle.dev_wallets().next().unwrap().with_chain_id(Some(chain_id));
    let signature = wallet.sign_transaction_sync(&mut malformed_raw).unwrap();
    let mut malformed_raw_encoded = Vec::new();
    malformed_raw.into_signed(signature).eip2718_encode(&mut malformed_raw_encoded);

    let ordinary_tx = TransactionRequest::default()
        .with_from(participant)
        .with_to(recipient)
        .with_nonce(0)
        .with_value(U256::ONE)
        .with_gas_limit(21_000)
        .with_gas_price(2_000_000_000);
    api.anvil_reorg(ReorgOptions {
        depth: 1,
        tx_block_pairs: vec![
            (TransactionData::Raw(malformed_raw_encoded.into()), 0),
            (TransactionData::JSON(ordinary_tx), 0),
        ],
    })
    .await
    .unwrap();

    let block =
        provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
    let transactions = block.transactions.as_transactions().unwrap();
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].from(), participant);
    assert_eq!(transactions[0].to(), Some(recipient));
    assert_eq!(provider.get_transaction_count(SYSTEM_ADDRESS).await.unwrap(), SYSTEM_NONCE);
    assert_eq!(provider.get_balance(STAKING_ADDRESS).await.unwrap(), initial_staking_balance);
    assert_eq!(
        provider.get_balance(recipient).await.unwrap(),
        initial_recipient_balance + U256::ONE
    );

    api.anvil_impersonate_account(SYSTEM_ADDRESS).await.unwrap();
    let error = provider
        .send_transaction(
            monad_protocol_transaction(
                SYSTEM_NONCE,
                U256::ZERO,
                syscallSnapshotCall {}.abi_encode().into(),
            )
            .into(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("intrinsic gas too low") || error.contains("max fee per gas less"),
        "unexpected non-replay validation error: {error}"
    );
    assert_eq!(provider.get_transaction_count(SYSTEM_ADDRESS).await.unwrap(), SYSTEM_NONCE);
    assert_eq!(
        provider.get_storage_at(STAKING_ADDRESS, global_slots::IN_BOUNDARY).await.unwrap(),
        U256::ZERO
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_fork_transaction_hash_replays_protocol_system_prefix_and_target() {
    const BLOCK_AUTHOR: Address = address!("0x1111111111111111111111111111111111111111");
    const VALIDATOR_AUTH: Address = address!("0x2222222222222222222222222222222222222222");
    const VALIDATOR_ID: u64 = 7;

    let origin_config = NodeConfig::test()
        .with_chain_id(Some(MONAD_TESTNET_CHAIN_ID))
        .with_genesis_timestamp(Some(
            MonadHardfork::MonadNine.testnet_activation_timestamp().unwrap(),
        ))
        .with_transaction_order(TransactionOrder::Fifo);
    let (origin_api, origin_handle) = spawn(origin_config).await;
    let origin_provider = origin_handle.http_provider();
    let participant = origin_provider.get_accounts().await.unwrap()[0];
    let reward = mon(25);
    let initial_system_balance = mon(100);
    let initial_staking_balance = mon(3);

    origin_api.anvil_impersonate_account(SYSTEM_ADDRESS).await.unwrap();
    origin_api.anvil_set_nonce(SYSTEM_ADDRESS, U256::from(11)).await.unwrap();
    origin_api.anvil_set_balance(SYSTEM_ADDRESS, initial_system_balance).await.unwrap();
    origin_api.anvil_set_balance(STAKING_ADDRESS, initial_staking_balance).await.unwrap();
    origin_api
        .anvil_set_code(RESERVE_PROBE_ADDRESS, Bytes::from(hex!("60015f355500")))
        .await
        .unwrap();
    origin_api
        .anvil_set_code(
            BALANCE_PROBE_ADDRESS,
            Bytes::from(hex!("730000000000000000000000000000000000001000315f5260205ff3")),
        )
        .await
        .unwrap();
    origin_api
        .anvil_set_code(CHAIN_ID_PROBE_ADDRESS, Bytes::from(hex!("465f5260205ff3")))
        .await
        .unwrap();
    origin_api
        .anvil_set_storage_at(
            STAKING_ADDRESS,
            val_id_secp_key(&BLOCK_AUTHOR),
            storage_value(left_aligned_u64(VALIDATOR_ID)),
        )
        .await
        .unwrap();
    origin_api
        .anvil_set_storage_at(
            STAKING_ADDRESS,
            consensus_view_key(VALIDATOR_ID, 0),
            storage_value(mon(100)),
        )
        .await
        .unwrap();
    origin_api
        .anvil_set_storage_at(STAKING_ADDRESS, consensus_view_key(VALIDATOR_ID, 1), B256::ZERO)
        .await
        .unwrap();
    origin_api
        .anvil_set_storage_at(
            STAKING_ADDRESS,
            validator_key(VALIDATOR_ID, validator_offsets::ADDRESS_FLAGS),
            storage_value(address_and_flags(VALIDATOR_AUTH, 0)),
        )
        .await
        .unwrap();

    origin_api.mine_one().await.unwrap();
    let parent_block = origin_provider.get_block_number().await.unwrap();
    origin_api.anvil_set_auto_mine(false).await.unwrap();

    let reward_tx = |nonce| {
        TransactionRequest::default()
            .with_from(SYSTEM_ADDRESS)
            .with_to(STAKING_ADDRESS)
            .with_nonce(nonce)
            .with_value(reward)
            .with_input(Bytes::from(syscallRewardCall { blockAuthor: BLOCK_AUTHOR }.abi_encode()))
            .with_gas_limit(1_000_000)
            .with_gas_price(2_000_000_000)
    };
    let first_reward = origin_provider.send_transaction(reward_tx(11).into()).await.unwrap();
    let first_reward_hash = *first_reward.tx_hash();
    let first_probe = origin_provider
        .send_transaction(
            reserve_probe_tx(participant, 0, 0, mon(2)).with_gas_price(2_000_000_000).into(),
        )
        .await
        .unwrap();
    let first_probe_hash = *first_probe.tx_hash();
    let second_probe = origin_provider
        .send_transaction(
            reserve_probe_tx(participant, 1, 1, mon(1)).with_gas_price(2_000_000_000).into(),
        )
        .await
        .unwrap();
    let second_probe_hash = *second_probe.tx_hash();
    let target_reward = origin_provider.send_transaction(reward_tx(12).into()).await.unwrap();
    let target_reward_hash = *target_reward.tx_hash();
    origin_api.mine_one().await.unwrap();

    let source_block = origin_api
        .block_by_number_full(BlockNumberOrTag::Number(parent_block + 1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        source_block
            .transactions
            .as_transactions()
            .unwrap()
            .iter()
            .map(TransactionResponse::tx_hash)
            .collect::<Vec<_>>(),
        [first_reward_hash, first_probe_hash, second_probe_hash, target_reward_hash]
    );

    let endpoint =
        spawn_canonical_monad_system_rpc(origin_handle.http_endpoint(), first_reward_hash).await;
    let endpoint = spawn_canonical_monad_system_rpc(endpoint, target_reward_hash).await;
    let canonical_provider = foundry_common::provider::get_http_provider(&endpoint);
    let canonical_target =
        canonical_provider.get_transaction_by_hash(target_reward_hash).await.unwrap().unwrap();
    assert_eq!(canonical_target.from(), SYSTEM_ADDRESS);
    let canonical_block = canonical_provider
        .get_block_by_number(BlockNumberOrTag::Number(parent_block + 1))
        .full()
        .await
        .unwrap()
        .unwrap();
    let canonical_target = canonical_block
        .transactions
        .as_transactions()
        .unwrap()
        .iter()
        .find(|transaction| transaction.tx_hash() == target_reward_hash)
        .unwrap();
    assert_eq!(canonical_target.from(), SYSTEM_ADDRESS);
    let config = monad_nine_config()
        .with_chain_id(Some(1u64))
        .with_genesis_accounts(Vec::new())
        .with_no_storage_caching(true)
        .with_eth_rpc_url(Some(endpoint.clone()))
        .with_fork_transaction_hash(Some(target_reward_hash))
        .with_no_mining(true);
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    assert_eq!(provider.get_chain_id().await.unwrap(), 1);

    let replayed_target =
        provider.get_transaction_by_hash(target_reward_hash).await.unwrap().unwrap();
    assert_eq!(replayed_target.from(), SYSTEM_ADDRESS);
    assert_eq!(replayed_target.tx_hash(), target_reward_hash);
    let first_receipt = provider.get_transaction_receipt(first_reward_hash).await.unwrap().unwrap();
    let target_receipt =
        provider.get_transaction_receipt(target_reward_hash).await.unwrap().unwrap();
    let replay_block_hash = target_receipt.block_hash.unwrap();
    let replay_block_number = target_receipt.block_number.unwrap();
    for receipt in [&first_receipt, &target_receipt] {
        assert!(receipt.status());
        assert_eq!(receipt.gas_used, 0);
        assert_eq!(receipt.effective_gas_price, 0);
    }
    for probe_hash in [first_probe_hash, second_probe_hash] {
        assert!(provider.get_transaction_receipt(probe_hash).await.unwrap().unwrap().status());
    }

    assert_eq!(provider.get_transaction_count(SYSTEM_ADDRESS).await.unwrap(), 13);
    assert_eq!(provider.get_balance(SYSTEM_ADDRESS).await.unwrap(), initial_system_balance);
    assert_eq!(
        provider.get_balance(STAKING_ADDRESS).await.unwrap(),
        initial_staking_balance + reward * U256::from(2)
    );
    assert_eq!(provider.get_transaction_count(participant).await.unwrap(), 2);
    assert_eq!(
        provider.get_storage_at(RESERVE_PROBE_ADDRESS, U256::ZERO).await.unwrap(),
        U256::ONE
    );
    assert_eq!(provider.get_storage_at(RESERVE_PROBE_ADDRESS, U256::ONE).await.unwrap(), U256::ONE);
    assert_eq!(
        provider.get_storage_at(STAKING_ADDRESS, global_slots::PROPOSER_VAL_ID).await.unwrap(),
        left_aligned_u64(VALIDATOR_ID)
    );
    assert_eq!(
        provider
            .get_storage_at(
                STAKING_ADDRESS,
                validator_key(VALIDATOR_ID, validator_offsets::UNCLAIMED_REWARDS),
            )
            .await
            .unwrap(),
        reward * U256::from(2)
    );

    let replay: TraceResults = provider
        .client()
        .request("trace_replayTransaction", (target_reward_hash, vec![TraceType::StateDiff]))
        .await
        .unwrap();
    assert!(replay.state_diff.unwrap().contains_key(&STAKING_ADDRESS));

    let _: GethTrace = provider
        .client()
        .request("debug_traceTransaction", (target_reward_hash, GethDebugTracingOptions::default()))
        .await
        .unwrap();

    let transaction_opcode_gas: Option<TransactionOpcodeGas> = provider
        .client()
        .request("trace_transactionOpcodeGas", (target_reward_hash,))
        .await
        .unwrap();
    assert_eq!(transaction_opcode_gas.unwrap().transaction_hash, target_reward_hash);

    let opcode_gas: Option<BlockOpcodeGas> = provider
        .client()
        .request("trace_blockOpcodeGas", (BlockId::hash(replay_block_hash),))
        .await
        .unwrap();
    let opcode_gas = opcode_gas.unwrap();
    assert_eq!(opcode_gas.transactions.len(), 4);
    assert_eq!(
        opcode_gas
            .transactions
            .iter()
            .map(|transaction| transaction.transaction_hash)
            .collect::<Vec<_>>(),
        [first_reward_hash, first_probe_hash, second_probe_hash, target_reward_hash]
    );

    let staking_after_target: Option<RpcAccountInfo> = provider
        .client()
        .request(
            "debug_accountInfoAt",
            (BlockId::hash(replay_block_hash), Index::from(3), STAKING_ADDRESS),
        )
        .await
        .unwrap();
    assert_eq!(
        staking_after_target.unwrap().balance,
        initial_staking_balance + reward * U256::from(2)
    );

    let balance_trace = provider
        .debug_trace_call(
            WithOtherFields::new(TransactionRequest::default().to(BALANCE_PROBE_ADDRESS)),
            BlockId::number(replay_block_number),
            GethDebugTracingCallOptions::default().with_tx_index(3),
        )
        .await
        .unwrap();
    let GethTrace::Default(balance_trace) = balance_trace else {
        panic!("expected default balance probe trace")
    };
    assert_eq!(U256::from_be_slice(&balance_trace.return_value), initial_staking_balance + reward);

    let chain_id_trace = provider
        .debug_trace_call(
            WithOtherFields::new(TransactionRequest::default().to(CHAIN_ID_PROBE_ADDRESS)),
            BlockId::number(replay_block_number),
            GethDebugTracingCallOptions::default().with_tx_index(3),
        )
        .await
        .unwrap();
    let GethTrace::Default(chain_id_trace) = chain_id_trace else {
        panic!("expected default chain ID probe trace")
    };
    assert_eq!(
        U256::from_be_slice(&chain_id_trace.return_value),
        U256::from(MONAD_TESTNET_CHAIN_ID)
    );

    let state = api.serialized_state(true).await.unwrap();
    for reward_hash in [first_reward_hash, target_reward_hash] {
        let reward_info = state
            .transactions
            .iter()
            .find(|transaction| transaction.info.transaction_hash == reward_hash)
            .unwrap();
        assert_eq!(reward_info.info.from, SYSTEM_ADDRESS);
    }
    let serialized_replay_block_hash = state
        .transactions
        .iter()
        .find(|transaction| transaction.info.transaction_hash == target_reward_hash)
        .unwrap()
        .block_hash;
    assert_eq!(serialized_replay_block_hash, replay_block_hash);
    let replay_profile = state.monad_block_replay_profiles[&replay_block_hash];
    assert_eq!(replay_profile.execution_chain_id, MONAD_TESTNET_CHAIN_ID);
    assert_eq!(replay_profile.hardfork, MonadHardfork::MonadNine);

    let loaded_config = monad_eight_config()
        .with_chain_id(Some(1u64))
        .with_genesis_accounts(Vec::new())
        .with_no_storage_caching(true)
        .with_eth_rpc_url(Some(endpoint))
        .with_fork_block_number(Some(parent_block))
        .with_transaction_block_keeper(Some(1usize))
        .with_init_state(Some(state))
        .with_no_mining(true);
    let (loaded_api, loaded_handle) = spawn(loaded_config).await;
    let loaded_provider = loaded_handle.http_provider();
    let loaded_state = loaded_api.serialized_state(false).await.unwrap();
    for reward_hash in [first_reward_hash, target_reward_hash] {
        let reward_info = loaded_state
            .transactions
            .iter()
            .find(|transaction| transaction.info.transaction_hash == reward_hash)
            .unwrap();
        assert_eq!(reward_info.info.from, SYSTEM_ADDRESS);
    }
    let loaded_replay_profile = loaded_state.monad_block_replay_profiles[&replay_block_hash];
    assert_eq!(loaded_replay_profile, replay_profile);
    let mut state_overrides = StateOverride::default();
    state_overrides.insert(
        CLZ_PROBE_ADDRESS,
        AccountOverride {
            code: Some(Bytes::from(hex!("60011e60005260206000f3"))),
            ..Default::default()
        },
    );
    let clz_trace = loaded_provider
        .debug_trace_call(
            WithOtherFields::new(TransactionRequest::default().to(CLZ_PROBE_ADDRESS)),
            BlockId::number(replay_block_number),
            GethDebugTracingCallOptions::default()
                .with_tx_index(3)
                .with_state_overrides(state_overrides),
        )
        .await
        .unwrap();
    let GethTrace::Default(clz_trace) = clz_trace else {
        panic!("expected default CLZ probe trace")
    };
    assert!(!clz_trace.failed);
    assert_eq!(U256::from_be_slice(&clz_trace.return_value), U256::from(255));
    let loaded_opcode_gas: Option<TransactionOpcodeGas> = loaded_provider
        .client()
        .request("trace_transactionOpcodeGas", (target_reward_hash,))
        .await
        .unwrap();
    assert_eq!(loaded_opcode_gas.unwrap().transaction_hash, target_reward_hash);

    loaded_api.mine_one().await.unwrap();
    let local_block_hash = loaded_provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await
        .unwrap()
        .unwrap()
        .header
        .hash;
    let pruned_state = loaded_api.serialized_state(false).await.unwrap();
    assert_eq!(pruned_state.monad_block_replay_profiles[&replay_block_hash], replay_profile);
    let local_profile = pruned_state.monad_block_replay_profiles[&local_block_hash];
    assert_eq!(local_profile.execution_chain_id, 1);
    assert_eq!(local_profile.hardfork, MonadHardfork::MonadEight);
    let participants = &pruned_state.monad_block_participants[&replay_block_hash];
    assert!(participants.contains(&SYSTEM_ADDRESS));
    assert!(participants.contains(&participant));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_fork_transaction_hash_preserves_hardfork_on_chain_id_collision() {
    let (_origin_api, origin_handle) = spawn(monad_eight_config().with_chain_id(Some(1u64))).await;
    let origin_provider = origin_handle.http_provider();
    let accounts = origin_provider.get_accounts().await.unwrap();
    let receipt = origin_provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(accounts[0])
                .with_to(accounts[1])
                .with_value(U256::ONE)
                .into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let config = NodeConfig::test()
        .with_no_storage_caching(true)
        .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
        .with_fork_transaction_hash(Some(receipt.transaction_hash))
        .with_no_mining(true);
    let (api, handle) = spawn(config).await;

    assert_eq!(api.backend.hardfork(), FoundryHardfork::Monad(MonadHardfork::MonadEight));
    assert!(handle.http_provider().call(reserve_balance_call()).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_mining_tracks_eip7702_authorities() {
    let (api, handle) = spawn(monad_nine_config()).await;
    let provider = handle.http_provider();
    let wallets = handle.dev_wallets().collect::<Vec<_>>();
    let authority = wallets[0].address();
    let initial_balance = U256::from(12_000_000_000_000_000_000u128);

    api.anvil_set_code(
        RESERVE_PROBE_ADDRESS,
        Bytes::from(hex!("633a61584e5f5260205f6004601c5f6110015af1505f515f355500")),
    )
    .await
    .unwrap();
    api.anvil_set_balance(authority, initial_balance).await.unwrap();

    let authorization =
        Authorization { chain_id: U256::from(31337), address: Address::ZERO, nonce: 0 };
    let signature = Signature::new(U256::ZERO, U256::ZERO, true);
    api.anvil_impersonate_signature(signature.as_bytes().into(), authority).await.unwrap();
    let mut tx = TxEip7702 {
        chain_id: 31337,
        nonce: 0,
        gas_limit: 100_000,
        max_fee_per_gas: 2_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: wallets[2].address(),
        authorization_list: vec![authorization.into_signed(signature)],
        ..Default::default()
    };
    let signature = wallets[1].sign_transaction_sync(&mut tx).unwrap();
    let mut encoded = Vec::new();
    tx.into_signed(signature).eip2718_encode(&mut encoded);
    let authorization =
        PendingTransaction::new(FoundryTxEnvelope::decode_2718(&mut encoded.as_slice()).unwrap())
            .unwrap();
    let authorization_hash = *authorization.hash();

    let mut probe = TxEip1559 {
        chain_id: 31337,
        nonce: 1,
        gas_limit: 100_000,
        max_fee_per_gas: 2_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(RESERVE_PROBE_ADDRESS),
        value: U256::from(3_000_000_000_000_000_000u128),
        input: Bytes::copy_from_slice(&U256::from(6).to_be_bytes::<32>()),
        ..Default::default()
    };
    let signature = wallets[0].sign_transaction_sync(&mut probe).unwrap();
    let mut encoded = Vec::new();
    probe.into_signed(signature).eip2718_encode(&mut encoded);
    let probe =
        PendingTransaction::new(FoundryTxEnvelope::decode_2718(&mut encoded.as_slice()).unwrap())
            .unwrap();
    let probe_hash = *probe.hash();

    let outcome = api
        .backend
        .mine_block(vec![
            Arc::new(PoolTransaction::new(authorization)),
            Arc::new(PoolTransaction::new(probe)),
        ])
        .await
        .unwrap();
    assert_eq!(outcome.included.len(), 2);
    assert!(outcome.invalid.is_empty());

    assert_eq!(
        provider.get_storage_at(RESERVE_PROBE_ADDRESS, U256::from(6)).await.unwrap(),
        U256::ONE
    );

    let replay: TraceResults = provider
        .client()
        .request("trace_replayTransaction", (probe_hash, vec![TraceType::StateDiff]))
        .await
        .unwrap();
    let slot = B256::from(U256::from(6).to_be_bytes::<32>());
    let state_diff = replay.state_diff.unwrap();
    let delta = &state_diff.get(&RESERVE_PROBE_ADDRESS).unwrap().storage[&slot];
    let replayed_value =
        delta.as_added().copied().or_else(|| delta.as_changed().map(|change| change.to)).unwrap();
    assert_eq!(replayed_value, B256::from(U256::ONE.to_be_bytes::<32>()));

    let state = api.serialized_state(false).await.unwrap();
    let authorization_block_hash = state
        .transactions
        .iter()
        .find(|transaction| transaction.info.transaction_hash == authorization_hash)
        .unwrap()
        .block_hash;
    assert!(state.monad_block_participants[&authorization_block_hash].contains(&authority));

    let (loaded_api, _) =
        spawn(monad_nine_config().with_genesis_accounts(Vec::new()).with_init_state(Some(state)))
            .await;
    let loaded_state = loaded_api.serialized_state(false).await.unwrap();
    assert!(loaded_state.monad_block_participants[&authorization_block_hash].contains(&authority));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_nine_config_reports_monad_precompiles() {
    let (api, _handle) = spawn(monad_nine_config()).await;
    let config = api.config().unwrap();

    assert_eq!(config.current.precompiles.get("MonadStaking"), Some(&STAKING_ADDRESS));
    assert_eq!(
        config.current.precompiles.get("MonadReserveBalance"),
        Some(&RESERVE_BALANCE_ADDRESS)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_eight_config_filters_reserve_balance_precompile() {
    let (api, _handle) = spawn(monad_eight_config()).await;
    let config = api.config().unwrap();

    assert_eq!(config.current.precompiles.get("MonadStaking"), Some(&STAKING_ADDRESS));
    assert!(!config.current.precompiles.contains_key("MonadReserveBalance"));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_can_mine_contract_larger_than_eip170_limit() {
    let config = monad_eight_config();
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let runtime_len = EIP170_CODE_SIZE_LIMIT + 1;

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_input(large_contract_init_code(runtime_len))
        .with_gas_limit(10_000_000);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());
    let contract = receipt.contract_address.expect("deployment should create a contract");
    let code = provider.get_code_at(contract).await.unwrap();
    assert_eq!(code.len(), runtime_len);
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_can_mine_contract_larger_than_eip3860_initcode_limit() {
    let config = monad_eight_config();
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let init_code = large_contract_init_code(EIP3860_INITCODE_SIZE_LIMIT);
    assert!(init_code.len() > EIP3860_INITCODE_SIZE_LIMIT);

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_input(init_code)
        .with_gas_limit(25_000_000);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());
    let contract = receipt.contract_address.expect("deployment should create a contract");
    let code = provider.get_code_at(contract).await.unwrap();
    assert_eq!(code.len(), EIP3860_INITCODE_SIZE_LIMIT);
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_allows_tx_gas_limit_above_eip7825_cap() {
    let config = monad_eight_config().enable_tx_gas_limit(true);
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();
    let gas_limit = 20_000_000;
    assert!(gas_limit > EIP7825_TX_GAS_LIMIT_CAP);

    let tx = TransactionRequest::default()
        .with_from(accounts[0])
        .with_to(accounts[1])
        .with_value(U256::from(1))
        .with_gas_limit(gas_limit);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_rejects_tx_gas_limit_above_monad_cap() {
    let config = monad_eight_config().enable_tx_gas_limit(true).with_gas_limit(Some(40_000_000));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();

    let tx = TransactionRequest::default()
        .with_from(accounts[0])
        .with_to(accounts[1])
        .with_value(U256::from(1))
        .with_gas_limit(MONAD_TX_GAS_LIMIT_CAP + 1);
    let err = provider.send_transaction(tx.into()).await.unwrap_err().to_string();

    assert!(err.contains("tx.gas_limit > resolved tx gas limit cap"), "unexpected error: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_omitted_gas_fallback_uses_resolved_tx_gas_cap() {
    let config = monad_eight_config().enable_tx_gas_limit(true).with_gas_limit(Some(40_000_000));
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];

    let tx =
        TransactionRequest::default().with_from(from).with_input(Bytes::from(hex!("60006000fd")));
    let pending = provider.send_transaction(tx.into()).await.unwrap();
    let sent = provider.get_transaction_by_hash(*pending.tx_hash()).await.unwrap().unwrap();

    assert_eq!(sent.inner.gas_limit(), MONAD_TX_GAS_LIMIT_CAP);
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_pool_accepts_balance_covering_effective_fee() {
    let (api, handle) = spawn(monad_nine_config()).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();
    let gas_limit = 21_000u64;
    let base_fee = 1_000_000_000u128;
    let priority_fee = 1_000_000_000u128;
    let max_fee = 100_000_000_000u128;
    let effective_fee = U256::from(gas_limit) * U256::from(base_fee + priority_fee);
    let max_fee_cost = U256::from(gas_limit) * U256::from(max_fee);
    assert!(effective_fee < max_fee_cost);

    api.anvil_set_next_block_base_fee_per_gas(U256::from(base_fee)).await.unwrap();
    api.anvil_set_balance(accounts[0], effective_fee).await.unwrap();

    let tx = TransactionRequest::default()
        .with_from(accounts[0])
        .with_to(accounts[1])
        .with_gas_limit(gas_limit)
        .with_max_fee_per_gas(max_fee)
        .with_max_priority_fee_per_gas(priority_fee);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_pool_admits_unaffordable_value_for_failed_receipt() {
    let (api, handle) = spawn(monad_nine_config()).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();
    let gas_limit = 21_000u64;
    let base_fee = 1_000_000_000u128;
    let priority_fee = 1_000_000_000u128;
    let max_fee = 100_000_000_000u128;
    let effective_fee = U256::from(gas_limit) * U256::from(base_fee + priority_fee);

    api.anvil_set_next_block_base_fee_per_gas(U256::from(base_fee)).await.unwrap();
    api.anvil_set_balance(accounts[0], effective_fee).await.unwrap();
    let recipient_balance = provider.get_balance(accounts[1]).await.unwrap();

    let tx = TransactionRequest::default()
        .with_from(accounts[0])
        .with_to(accounts[1])
        .with_value(U256::ONE)
        .with_gas_limit(gas_limit)
        .with_max_fee_per_gas(max_fee)
        .with_max_priority_fee_per_gas(priority_fee);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    assert!(!receipt.status());
    assert_eq!(provider.get_balance(accounts[1]).await.unwrap(), recipient_balance);
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_rejects_eip4844_blob_transactions() {
    let config = monad_nine_config();
    let (_api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let accounts = provider.get_accounts().await.unwrap();
    let eip1559_est = provider.estimate_eip1559_fees().await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();
    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Hello World");
    let sidecar = sidecar.build().unwrap();

    let tx = TransactionRequest::default()
        .with_from(accounts[0])
        .with_to(accounts[1])
        .with_nonce(0)
        .with_max_fee_per_blob_gas(gas_price + 1)
        .with_max_fee_per_gas(eip1559_est.max_fee_per_gas)
        .with_max_priority_fee_per_gas(eip1559_est.max_priority_fee_per_gas)
        .with_blob_sidecar_4844(sidecar)
        .with_gas_limit(100_000)
        .with_value(U256::from(5));
    let err = provider.send_transaction(WithOtherFields::new(tx)).await.unwrap_err().to_string();

    assert!(
        err.contains("EIP-4844 blob transactions are not supported on Monad"),
        "unexpected error: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_fork_uses_monad_execution() {
    let (origin_api, origin_handle) = spawn(monad_nine_config()).await;
    origin_api.mine_one().await.unwrap();

    let config = monad_nine_config().with_eth_rpc_url(Some(origin_handle.http_endpoint()));
    let (fork_api, fork_handle) = spawn(config).await;

    let node_info = fork_api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network, Some("monad".to_string()));
    assert_eq!(node_info.hard_fork, "MonadNine");
    assert!(node_info.fork_config.fork_block_number.is_some());

    let tx = TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR);
    let result = fork_handle.http_provider().call(tx.into()).await.unwrap();

    assert_eq!(result, Bytes::from(vec![0; 32]));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_fork_preserves_source_chain_with_execution_override() {
    let (_origin, endpoint) = monad_boundary_origin().await;
    let config = NodeConfig::test()
        .with_chain_id(Some(1u64))
        .with_eth_rpc_url(Some(endpoint))
        .with_fork_block_number(Some(1u64));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    let node_info = api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network, Some("monad".to_string()));
    assert_eq!(node_info.hard_fork, "MonadNine");
    assert_eq!(node_info.environment.chain_id, 1);
    assert_eq!(
        api.anvil_metadata().await.unwrap().forked_network.unwrap().chain_id,
        MONAD_TESTNET_CHAIN_ID
    );
    assert_eq!(provider.call(reserve_balance_call()).await.unwrap(), Bytes::from(vec![0; 32]));

    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(2) })).await.unwrap();

    let node_info = api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network, Some("monad".to_string()));
    assert_eq!(node_info.hard_fork, "MonadNine");
    assert_eq!(node_info.environment.chain_id, 1);
    assert_eq!(
        api.anvil_metadata().await.unwrap().forked_network.unwrap().chain_id,
        MONAD_TESTNET_CHAIN_ID
    );
    assert_eq!(provider.call(reserve_balance_call()).await.unwrap(), Bytes::from(vec![0; 32]));

    let nested_config = NodeConfig::test()
        .with_no_storage_caching(true)
        .with_eth_rpc_url(Some(handle.http_endpoint()))
        .with_fork_block_number(Some(2u64));
    let (nested_api, nested_handle) = spawn(nested_config).await;
    let nested_info = nested_api.anvil_node_info().await.unwrap();
    assert_eq!(nested_info.network.as_deref(), Some("monad"));
    assert_eq!(nested_info.environment.chain_id, 1);
    assert_eq!(
        nested_api.anvil_metadata().await.unwrap().forked_network.unwrap().chain_id,
        MONAD_TESTNET_CHAIN_ID
    );
    assert_eq!(
        nested_handle.http_provider().call(reserve_balance_call()).await.unwrap(),
        Bytes::from(vec![0; 32])
    );

    let (_ethereum_api, ethereum_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(1u64))).await;
    let err = api
        .anvil_reset(Some(Forking {
            json_rpc_url: Some(ethereum_handle.http_endpoint()),
            block_number: None,
        }))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("cannot reset Anvil across network families (monad -> ethereum)"));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_fork_infers_anvil_identity_and_exact_hardfork() {
    let activation = MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap();
    for (chain_id, genesis_timestamp) in [
        (1u64, Some(activation)),
        (143u64, Some(activation)),
        (31337u64, None),
        (98_765_432u64, None),
    ] {
        let mut origin_config = NodeConfig::test_monad()
            .with_chain_id(Some(chain_id))
            .with_hardfork(Some(MonadHardfork::MonadEight.into()));
        if let Some(genesis_timestamp) = genesis_timestamp {
            origin_config = origin_config.with_genesis_timestamp(Some(genesis_timestamp));
        }
        let (_origin_api, origin_handle) = spawn(origin_config).await;
        let fork_config = NodeConfig::test()
            .with_no_storage_caching(true)
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_block_number(Some(0u64));
        let (fork_api, fork_handle) = spawn(fork_config).await;

        let node_info = fork_api.anvil_node_info().await.unwrap();
        assert_eq!(node_info.network, Some("monad".to_string()));
        assert_eq!(node_info.hard_fork, MonadHardfork::MonadEight.to_string());
        assert_eq!(node_info.environment.chain_id, chain_id);
        assert_eq!(
            fork_api.anvil_metadata().await.unwrap().forked_network.unwrap().chain_id,
            chain_id
        );
        let result = fork_handle.http_provider().call(reserve_balance_call()).await.unwrap();
        assert!(result.is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_fork_execution_chain_override_does_not_select_network_family() {
    let (_origin_api, origin_handle) =
        spawn(monad_nine_config().with_chain_id(Some(MONAD_TESTNET_CHAIN_ID))).await;
    let endpoint = origin_handle.http_endpoint();
    let execution_chain_id = 4217u64;
    let configs = [
        NodeConfig::test()
            .with_chain_id(Some(execution_chain_id))
            .with_eth_rpc_url(Some(endpoint.clone())),
        NodeConfig::test().with_eth_rpc_url(Some(endpoint)).with_chain_id(Some(execution_chain_id)),
    ];

    for config in configs {
        let config = config.with_no_storage_caching(true).with_fork_block_number(Some(0u64));
        let (api, handle) = spawn(config).await;

        let node_info = api.anvil_node_info().await.unwrap();
        assert_eq!(node_info.network.as_deref(), Some("monad"));
        assert_eq!(node_info.environment.chain_id, execution_chain_id);
        assert_eq!(
            api.anvil_metadata().await.unwrap().forked_network.unwrap().chain_id,
            MONAD_TESTNET_CHAIN_ID
        );
        assert_eq!(
            handle.http_provider().call(reserve_balance_call()).await.unwrap(),
            Bytes::from(vec![0; 32])
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_fork_preserves_explicit_network_selection() {
    let (_ethereum_api, ethereum_handle) =
        spawn(NodeConfig::test().with_chain_id(Some(1u64))).await;
    let explicit_monad = monad_nine_config()
        .with_no_storage_caching(true)
        .with_eth_rpc_url(Some(ethereum_handle.http_endpoint()))
        .with_fork_block_number(Some(0u64));
    let (monad_api, monad_handle) = spawn(explicit_monad).await;

    assert_eq!(monad_api.anvil_node_info().await.unwrap().network.as_deref(), Some("monad"));
    assert_eq!(
        monad_handle.http_provider().call(reserve_balance_call()).await.unwrap(),
        Bytes::from(vec![0; 32])
    );
    monad_api
        .anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(0) }))
        .await
        .unwrap();
    monad_api
        .anvil_reset(Some(Forking {
            json_rpc_url: Some(ethereum_handle.http_endpoint()),
            block_number: Some(0),
        }))
        .await
        .unwrap();
    assert_eq!(
        monad_handle.http_provider().call(reserve_balance_call()).await.unwrap(),
        Bytes::from(vec![0; 32])
    );

    let (_monad_api, monad_origin) = spawn(monad_nine_config().with_chain_id(Some(1u64))).await;
    let config = NodeConfig::test()
        .with_networks(NetworkConfigs::with_ethereum())
        .with_no_storage_caching(true)
        .with_eth_rpc_url(Some(monad_origin.http_endpoint()))
        .with_fork_block_number(Some(0u64));
    let (api, handle) = spawn(config).await;

    assert_eq!(api.anvil_node_info().await.unwrap().network.as_deref(), Some("ethereum"));
    assert!(handle.http_provider().call(reserve_balance_call()).await.unwrap().is_empty());
    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(0) })).await.unwrap();
    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(monad_origin.http_endpoint()),
        block_number: Some(0),
    }))
    .await
    .unwrap();
    assert!(handle.http_provider().call(reserve_balance_call()).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_fork_reset_without_url_preserves_monad_execution() {
    let (origin_api, origin_handle) = spawn(monad_nine_config()).await;
    origin_api.mine_one().await.unwrap();

    let config = monad_nine_config().with_eth_rpc_url(Some(origin_handle.http_endpoint()));
    let (fork_api, fork_handle) = spawn(config).await;

    fork_api
        .anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(0) }))
        .await
        .unwrap();

    let node_info = fork_api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network, Some("monad".to_string()));
    assert_eq!(node_info.hard_fork, "MonadNine");
    assert_eq!(node_info.fork_config.fork_block_number, Some(0));

    let tx = TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR);
    let result = fork_handle.http_provider().call(tx.into()).await.unwrap();

    assert_eq!(result, Bytes::from(vec![0; 32]));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_reset_refreshes_ambiguous_metadata_hardfork() {
    let eight_chain_id = 98_765_432u64;
    let nine_chain_id = 98_765_433u64;
    let (_eight_api, eight_handle) =
        spawn(monad_eight_config().with_chain_id(Some(eight_chain_id)).with_genesis_timestamp(
            Some(MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap()),
        ))
        .await;
    let (_nine_api, nine_handle) =
        spawn(monad_nine_config().with_chain_id(Some(nine_chain_id))).await;

    let config = NodeConfig::test()
        .with_no_storage_caching(true)
        .with_eth_rpc_url(Some(eight_handle.http_endpoint()))
        .with_fork_block_number(Some(0u64));
    let (api, handle) = spawn(config).await;
    assert_eq!(api.anvil_node_info().await.unwrap().hard_fork, "MonadEight");
    assert!(handle.http_provider().call(reserve_balance_call()).await.unwrap().is_empty());

    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(nine_handle.http_endpoint()),
        block_number: Some(0),
    }))
    .await
    .unwrap();

    let node_info = api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.hard_fork, "MonadNine");
    assert_eq!(api.anvil_metadata().await.unwrap().forked_network.unwrap().chain_id, nine_chain_id);
    assert_eq!(
        handle.http_provider().call(reserve_balance_call()).await.unwrap(),
        Bytes::from(vec![0; 32])
    );

    let explicit_nine = NodeConfig::test_monad()
        .with_hardfork(Some(MonadHardfork::MonadNine.into()))
        .with_no_storage_caching(true)
        .with_eth_rpc_url(Some(eight_handle.http_endpoint()))
        .with_fork_block_number(Some(0u64));
    let (explicit_api, explicit_handle) = spawn(explicit_nine).await;
    assert_eq!(explicit_api.anvil_node_info().await.unwrap().hard_fork, "MonadNine");
    assert_eq!(
        explicit_handle.http_provider().call(reserve_balance_call()).await.unwrap(),
        Bytes::from(vec![0; 32])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_reset_to_memory_restores_local_hardfork() {
    let activation = MonadHardfork::MonadNine.testnet_activation_timestamp().unwrap();
    assert_monad_reset_to_memory(activation, MonadHardfork::MonadNine, MonadHardfork::MonadEight)
        .await;
    assert_monad_reset_to_memory(
        activation - 1,
        MonadHardfork::MonadEight,
        MonadHardfork::MonadNine,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn direct_monad_fork_reset_to_memory_publishes_local_hardfork() {
    let timestamp = MonadHardfork::MonadNine.testnet_activation_timestamp().unwrap();
    assert!(
        timestamp < MonadHardfork::MonadNine.mainnet_activation_timestamp().unwrap(),
        "test timestamp must resolve differently on Monad mainnet and testnet"
    );
    let (_origin_api, origin_handle) = spawn(
        NodeConfig::test_monad()
            .with_chain_id(Some(143u64))
            .with_genesis_timestamp(Some(timestamp)),
    )
    .await;
    let (api, handle) = spawn(
        NodeConfig::test_monad()
            .with_chain_id(Some(MONAD_TESTNET_CHAIN_ID))
            .with_fork_chain_id(Some(U256::from(143u64)))
            .with_genesis_timestamp(Some(timestamp))
            .with_no_storage_caching(true)
            .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
            .with_fork_block_number(Some(0u64)),
    )
    .await;
    let provider = handle.http_provider();

    assert_eq!(api.anvil_node_info().await.unwrap().hard_fork, "MonadEight");
    assert!(provider.call(reserve_balance_call()).await.unwrap().is_empty());

    api.anvil_reset(None).await.unwrap();

    assert_eq!(api.anvil_node_info().await.unwrap().hard_fork, "MonadNine");
    assert_eq!(provider.call(reserve_balance_call()).await.unwrap(), Bytes::from(vec![0; 32]));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_fork_resets_preserve_endpoint_hardfork() {
    let (_origin, endpoint) = monad_boundary_origin().await;

    {
        let config = NodeConfig::test_monad()
            .with_chain_id(Some(MONAD_TESTNET_CHAIN_ID))
            .with_eth_rpc_url(Some(endpoint.clone()))
            .with_fork_block_number(Some(1u64));
        let (api, handle) = spawn(config).await;
        let provider = handle.http_provider();

        assert!(handle.config().hardfork.is_none());
        assert_eq!(api.anvil_node_info().await.unwrap().hard_fork, "MonadNine");
        assert_eq!(provider.call(reserve_balance_call()).await.unwrap(), Bytes::from(vec![0; 32]));

        api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(2) })).await.unwrap();

        assert_eq!(api.anvil_node_info().await.unwrap().hard_fork, "MonadNine");
        assert_eq!(provider.call(reserve_balance_call()).await.unwrap(), Bytes::from(vec![0; 32]));

        api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(1) })).await.unwrap();

        assert_eq!(api.anvil_node_info().await.unwrap().hard_fork, "MonadNine");
        assert_eq!(provider.call(reserve_balance_call()).await.unwrap(), Bytes::from(vec![0; 32]));
    }

    let config = NodeConfig::test_monad()
        .with_chain_id(Some(MONAD_TESTNET_CHAIN_ID))
        .with_hardfork(Some(MonadHardfork::MonadNine.into()))
        .with_eth_rpc_url(Some(endpoint))
        .with_fork_block_number(Some(1u64));
    let (api, handle) = spawn(config).await;

    assert_eq!(api.anvil_node_info().await.unwrap().hard_fork, "MonadNine");
    assert_eq!(
        handle.http_provider().call(reserve_balance_call()).await.unwrap(),
        Bytes::from(vec![0; 32])
    );

    api.anvil_reset(Some(Forking { json_rpc_url: None, block_number: Some(2) })).await.unwrap();

    assert_eq!(api.anvil_node_info().await.unwrap().hard_fork, "MonadNine");
    assert_eq!(
        handle.http_provider().call(reserve_balance_call()).await.unwrap(),
        Bytes::from(vec![0; 32])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_reset_can_start_forking_with_monad_execution() {
    let (origin_api, origin_handle) = spawn(monad_nine_config()).await;
    origin_api.mine_one().await.unwrap();

    let (api, handle) = spawn(monad_nine_config()).await;

    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(origin_handle.http_endpoint()),
        block_number: Some(0),
    }))
    .await
    .unwrap();

    let node_info = api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network, Some("monad".to_string()));
    assert_eq!(node_info.hard_fork, "MonadNine");
    assert_eq!(node_info.fork_config.fork_block_number, Some(0));

    let tx = TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR);
    let result = handle.http_provider().call(tx.into()).await.unwrap();

    assert_eq!(result, Bytes::from(vec![0; 32]));
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_anvil_rejects_reset_to_monad_fork() {
    let (api, handle) = spawn(NodeConfig::test()).await;

    for chain_id in [1u64, 143u64, 31337u64, 98_765_432u64] {
        let origin_config = monad_nine_config().with_chain_id(Some(chain_id));
        let (_origin_api, origin_handle) = spawn(origin_config).await;
        let err = api
            .anvil_reset(Some(Forking {
                json_rpc_url: Some(origin_handle.http_endpoint()),
                block_number: Some(0),
            }))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("cannot reset Anvil across network families (ethereum -> monad)"),
            "unexpected error for chain {chain_id}: {err}"
        );

        let node_info = api.anvil_node_info().await.unwrap();
        assert_eq!(node_info.network.as_deref(), Some("ethereum"));
        assert_eq!(node_info.fork_config.fork_url, None);
    }

    let tx = TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR);
    let result = handle.http_provider().call(tx.into()).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_anvil_rejects_monad_reset_hidden_by_fork_chain_id() {
    let (_origin_api, origin_handle) = spawn(monad_nine_config()).await;
    let (api, handle) = spawn(NodeConfig::test().with_fork_chain_id(Some(U256::from(1u64)))).await;
    let provider = handle.http_provider();
    let marker = Address::random();
    let balance = U256::from(123_456u64);
    api.anvil_set_balance(marker, balance).await.unwrap();
    let instance_id = api.anvil_metadata().await.unwrap().instance_id;

    let err = api
        .anvil_reset(Some(Forking {
            json_rpc_url: Some(origin_handle.http_endpoint()),
            block_number: Some(0),
        }))
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("cannot reset Anvil across network families (ethereum -> monad)"));
    assert_eq!(provider.get_balance(marker).await.unwrap(), balance);
    assert_eq!(api.anvil_metadata().await.unwrap().instance_id, instance_id);
    assert_eq!(api.anvil_node_info().await.unwrap().network.as_deref(), Some("ethereum"));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_anvil_rejects_reset_to_default_and_custom_ethereum_forks() {
    let (_monad_origin_api, monad_origin) = spawn(monad_nine_config()).await;
    let config = NodeConfig::test()
        .with_no_storage_caching(true)
        .with_eth_rpc_url(Some(monad_origin.http_endpoint()))
        .with_fork_block_number(Some(0u64));
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();
    let marker = address!("0000000000000000000000000000000000001234");
    let marker_balance = U256::from(123_456);
    api.anvil_set_balance(marker, marker_balance).await.unwrap();
    let instance_id = api.anvil_metadata().await.unwrap().instance_id;

    for chain_id in [1u64, 31337u64, 98_765_432u64] {
        let (_origin_api, origin_handle) =
            spawn(NodeConfig::test().with_chain_id(Some(chain_id))).await;
        let err = api
            .anvil_reset(Some(Forking {
                json_rpc_url: Some(origin_handle.http_endpoint()),
                block_number: Some(0),
            }))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("cannot reset Anvil across network families (monad -> ethereum)"),
            "unexpected error for chain {chain_id}: {err}"
        );

        let node_info = api.anvil_node_info().await.unwrap();
        assert_eq!(node_info.network, Some("monad".to_string()));
        assert_eq!(node_info.fork_config.fork_url, Some(monad_origin.http_endpoint()));
        assert_eq!(api.anvil_metadata().await.unwrap().instance_id, instance_id);
        assert_eq!(provider.get_balance(marker).await.unwrap(), marker_balance);
    }

    assert_eq!(provider.call(reserve_balance_call()).await.unwrap(), Bytes::from(vec![0; 32]));
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_safe_and_finalized_block_tags_use_configured_epoch_slots() {
    let slots_in_an_epoch = 3;
    let config = monad_eight_config().with_slots_in_an_epoch(slots_in_an_epoch);
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    api.anvil_mine(Some(U256::from(8)), None).await.unwrap();
    let latest = provider.get_block_number().await.unwrap();
    assert_eq!(latest, 8);

    let safe = provider.get_block(BlockId::Number(BlockNumberOrTag::Safe)).await.unwrap().unwrap();
    assert_eq!(safe.header.number, latest - slots_in_an_epoch);

    let finalized =
        provider.get_block(BlockId::Number(BlockNumberOrTag::Finalized)).await.unwrap().unwrap();
    assert_eq!(finalized.header.number, latest - slots_in_an_epoch * 2);

    let fee_history = api.fee_history(U256::from(1), BlockNumberOrTag::Safe, vec![]).await.unwrap();
    assert_eq!(fee_history.oldest_block, latest - slots_in_an_epoch);
}

#[tokio::test(flavor = "multi_thread")]
async fn monad_safe_and_finalized_block_tags_fall_back_to_genesis_before_epoch() {
    let config = monad_eight_config().with_slots_in_an_epoch(3);
    let (api, handle) = spawn(config).await;
    let provider = handle.http_provider();

    api.anvil_mine(Some(U256::from(2)), None).await.unwrap();
    let genesis = provider.get_block(BlockId::number(0)).await.unwrap().unwrap();

    let safe = provider.get_block(BlockId::Number(BlockNumberOrTag::Safe)).await.unwrap().unwrap();
    assert_eq!(safe.header.number, genesis.header.number);
    assert_eq!(safe.header.hash, genesis.header.hash);

    let finalized =
        provider.get_block(BlockId::Number(BlockNumberOrTag::Finalized)).await.unwrap().unwrap();
    assert_eq!(finalized.header.number, genesis.header.number);
    assert_eq!(finalized.header.hash, genesis.header.hash);

    let fee_history =
        api.fee_history(U256::from(1), BlockNumberOrTag::Finalized, vec![]).await.unwrap();
    assert_eq!(fee_history.oldest_block, genesis.header.number);
}

fn monad_nine_config() -> NodeConfig {
    NodeConfig::test_monad().with_hardfork(Some(MonadHardfork::MonadNine.into()))
}

fn monad_eight_config() -> NodeConfig {
    NodeConfig::test_monad().with_hardfork(Some(MonadHardfork::MonadEight.into()))
}

fn monad_protocol_transaction(nonce: u64, value: U256, input: Bytes) -> TransactionRequest {
    TransactionRequest::default()
        .with_from(SYSTEM_ADDRESS)
        .with_to(STAKING_ADDRESS)
        .with_nonce(nonce)
        .with_value(value)
        .with_input(input)
        .with_gas_limit(0)
        .with_gas_price(0)
}

fn mon(value: u64) -> U256 {
    U256::from(value) * U256::from(1_000_000_000_000_000_000u128)
}

fn left_aligned_u64(value: u64) -> U256 {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&value.to_be_bytes());
    U256::from_be_bytes(bytes)
}

fn address_and_flags(address: Address, flags: u64) -> U256 {
    let mut bytes = [0u8; 32];
    bytes[..20].copy_from_slice(address.as_slice());
    bytes[20..28].copy_from_slice(&flags.to_be_bytes());
    U256::from_be_bytes(bytes)
}

fn storage_value(value: U256) -> B256 {
    B256::from(value.to_be_bytes::<32>())
}

async fn assert_monad_reset_to_memory(
    local_timestamp: u64,
    local_hardfork: MonadHardfork,
    fork_hardfork: MonadHardfork,
) {
    let (_origin_api, origin_handle) = spawn(
        NodeConfig::test_monad()
            .with_chain_id(Some(MONAD_TESTNET_CHAIN_ID))
            .with_hardfork(Some(fork_hardfork.into())),
    )
    .await;
    let (api, handle) = spawn(
        NodeConfig::test_monad()
            .with_chain_id(Some(MONAD_TESTNET_CHAIN_ID))
            .with_genesis_timestamp(Some(local_timestamp))
            .with_no_storage_caching(true),
    )
    .await;
    let provider = handle.http_provider();

    api.anvil_reset(Some(Forking {
        json_rpc_url: Some(origin_handle.http_endpoint()),
        block_number: Some(0),
    }))
    .await
    .unwrap();
    assert_eq!(api.anvil_node_info().await.unwrap().hard_fork, fork_hardfork.to_string());
    let fork_result = provider.call(reserve_balance_call()).await.unwrap();
    assert_eq!(fork_result.is_empty(), fork_hardfork == MonadHardfork::MonadEight);

    api.anvil_reset(None).await.unwrap();
    assert_eq!(api.anvil_node_info().await.unwrap().hard_fork, local_hardfork.to_string());
    let local_result = provider.call(reserve_balance_call()).await.unwrap();
    assert_eq!(local_result.is_empty(), local_hardfork == MonadHardfork::MonadEight);
}

async fn monad_boundary_origin() -> (NodeHandle, String) {
    let activation = MonadHardfork::MonadNine.testnet_activation_timestamp().unwrap();
    let config = monad_nine_config()
        .with_chain_id(Some(MONAD_TESTNET_CHAIN_ID))
        .with_genesis_timestamp(Some(activation - 2));
    let (api, handle) = spawn(config).await;

    api.evm_set_next_block_timestamp(activation - 1).unwrap();
    api.mine_one().await.unwrap();
    api.evm_set_next_block_timestamp(activation).unwrap();
    api.mine_one().await.unwrap();

    let endpoint = handle.http_endpoint();
    (handle, endpoint)
}

fn reserve_balance_call() -> WithOtherFields<TransactionRequest> {
    TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR)
        .into()
}

fn storage_gas_probe_call() -> WithOtherFields<TransactionRequest> {
    TransactionRequest::default().with_to(STORAGE_GAS_PROBE_ADDRESS).into()
}

fn storage_probe_gas(result: Bytes) -> u64 {
    U256::from_be_slice(&result).to::<u64>()
}

fn root_trace_gas(result: &TraceResults) -> u64 {
    result.trace[0].result.as_ref().expect("root call trace should contain a result").gas_used()
}

fn storage_read_probe_code(second_slot: u8) -> Bytes {
    let mut code = hex!("5a5f5450600054505a90035f5260205ff3");
    code[5] = second_slot;
    Bytes::from(code)
}

fn storage_access_list_probe_code(first_slot: u8, second_slot: u8) -> Bytes {
    let mut code = hex!("600054506000545000");
    code[1] = first_slot;
    code[5] = second_slot;
    Bytes::from(code)
}

fn storage_write_probe_code(second_slot: u8) -> Bytes {
    let mut code = hex!("5a60015f5560016000555a90035f5260205ff3");
    code[8] = second_slot;
    Bytes::from(code)
}

fn reserve_probe_tx(from: Address, nonce: u64, slot: u64, value: U256) -> TransactionRequest {
    TransactionRequest::default()
        .with_from(from)
        .with_to(RESERVE_PROBE_ADDRESS)
        .with_nonce(nonce)
        .with_value(value)
        .with_gas_limit(100_000)
        .with_input(Bytes::copy_from_slice(&U256::from(slot).to_be_bytes::<32>()))
}

fn large_contract_init_code(runtime_len: usize) -> Bytes {
    assert!(runtime_len <= u16::MAX as usize);
    const HEADER_LEN: usize = 15;
    let runtime_len = runtime_len as u16;

    let mut code = Vec::with_capacity(HEADER_LEN + runtime_len as usize);
    code.extend_from_slice(&[0x61, (runtime_len >> 8) as u8, runtime_len as u8]);
    code.extend_from_slice(&[0x61, 0x00, HEADER_LEN as u8]);
    code.extend_from_slice(&[0x60, 0x00, 0x39]);
    code.extend_from_slice(&[0x61, (runtime_len >> 8) as u8, runtime_len as u8]);
    code.extend_from_slice(&[0x60, 0x00, 0xf3]);
    code.resize(HEADER_LEN + runtime_len as usize, 0);
    Bytes::from(code)
}
