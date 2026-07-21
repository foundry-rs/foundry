use alloy_consensus::{
    SidecarBuilder, SignableTransaction, SimpleCoder, Transaction, transaction::TxEip7702,
};
use alloy_network::{ReceiptResponse, TransactionBuilder, TransactionBuilder4844, TxSignerSync};
use alloy_primitives::{Address, B256, Bytes, U256, address, hex};
use alloy_provider::Provider;
use alloy_rpc_types::{
    Authorization, BlockId, BlockNumberOrTag, TransactionRequest,
    anvil::Forking,
    simulate::{SimBlock, SimulatePayload},
    trace::parity::{TraceResults, TraceType},
};
use alloy_serde::WithOtherFields;
use alloy_signer::SignerSync;
use anvil::{NodeConfig, spawn};
use foundry_evm::hardfork::MonadHardfork;

const STAKING_ADDRESS: Address = address!("0x0000000000000000000000000000000000001000");
const RESERVE_BALANCE_ADDRESS: Address = address!("0x0000000000000000000000000000000000001001");
const RESERVE_PROBE_ADDRESS: Address = address!("0x0000000000000000000000000000000000002000");
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
    api.mine_one().await;
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
    api.mine_one().await;
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
    let signature = wallets[0].sign_hash_sync(&authorization.signature_hash()).unwrap();
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
    provider.send_raw_transaction(&encoded).await.unwrap().get_receipt().await.unwrap();

    provider
        .send_transaction(
            reserve_probe_tx(authority, 1, 6, U256::from(3_000_000_000_000_000_000u128)).into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    assert_eq!(
        provider.get_storage_at(RESERVE_PROBE_ADDRESS, U256::from(6)).await.unwrap(),
        U256::ONE
    );
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
    origin_api.mine_one().await;

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
async fn monad_fork_reset_without_url_preserves_monad_execution() {
    let (origin_api, origin_handle) = spawn(monad_nine_config()).await;
    origin_api.mine_one().await;

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
async fn monad_reset_can_start_forking_with_monad_execution() {
    let (origin_api, origin_handle) = spawn(monad_nine_config()).await;
    origin_api.mine_one().await;

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
    let origin_config = monad_nine_config().with_chain_id(Some(143u64));
    let (_origin_api, origin_handle) = spawn(origin_config).await;
    let (api, handle) = spawn(NodeConfig::test()).await;

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
        "unexpected error: {err}"
    );

    let node_info = api.anvil_node_info().await.unwrap();
    assert_eq!(node_info.network, None);
    assert_eq!(node_info.fork_config.fork_url, None);

    let tx = TransactionRequest::default()
        .with_to(RESERVE_BALANCE_ADDRESS)
        .with_input(DIPPED_INTO_RESERVE_SELECTOR);
    let result = handle.http_provider().call(tx.into()).await.unwrap();
    assert!(result.is_empty());
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

fn mon(value: u64) -> U256 {
    U256::from(value) * U256::from(1_000_000_000_000_000_000u128)
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
