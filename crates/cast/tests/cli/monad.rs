//! CLI tests for monad commands.

use super::*;

const MONAD_RESERVE_BALANCE_ADDRESS: Address =
    address!("0x0000000000000000000000000000000000001001");
const MONAD_STAKING_ADDRESS: Address = address!("0x0000000000000000000000000000000000001000");
const MONAD_SYSTEM_ADDRESS: Address = address!("0x6f49a8f621353f12378d0046e7d7e4b9b249dc9e");
const MONAD_TESTNET_CHAIN_ID: u64 = 10_143;
const MONAD_NINE_TESTNET_ACTIVATION_TIMESTAMP: u64 = 1_773_153_000;
const MONAD_DIPPED_INTO_RESERVE_SELECTOR: [u8; 4] = hex!("3a61584e");
const MONAD_RESERVE_PROBE_ADDRESS: Address = address!("0x0000000000000000000000000000000000002000");
const MONAD_RESERVE_RETURN_PROBE_CODE: [u8; 25] =
    hex!("633a61584e5f5260205f6004601c5f6110015af15060205ff3");

fn mon(value: u64) -> U256 {
    U256::from(value) * U256::from(1_000_000_000_000_000_000u128)
}

#[cfg(feature = "monad")]
casttest!(monad_call_trace_uses_monad_evm_network, async |_prj, cmd| {
    let config = NodeConfig::test_monad()
        .with_hardfork(Some(foundry_evm::hardfork::MonadHardfork::MonadNine.into()));
    let (_api, handle) = anvil::spawn(config).await;
    let endpoint = handle.http_endpoint();
    let reserve_balance_address = MONAD_RESERVE_BALANCE_ADDRESS.to_string();
    let input = format!("0x{}", hex::encode(MONAD_DIPPED_INTO_RESERVE_SELECTOR));
    let output = cmd
        .args([
            "call",
            &reserve_balance_address,
            "--data",
            &input,
            "--rpc-url",
            &endpoint,
            "--trace",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(output.contains("Traces:"), "{output}");
    assert!(output.contains("ReserveBalance::dippedIntoReserve()"), "{output}");
    assert!(output.contains("[Return] false"), "{output}");
});

#[cfg(feature = "monad")]
casttest!(monad_call_trace_resolves_effective_hardfork, async |_prj, cmd| {
    let config = NodeConfig::test_monad()
        .with_hardfork(Some(foundry_evm::hardfork::MonadHardfork::MonadEight.into()))
        .with_chain_id(Some(MONAD_TESTNET_CHAIN_ID))
        .with_genesis_timestamp(Some(MONAD_NINE_TESTNET_ACTIVATION_TIMESTAMP - 1));
    let (_api, monad_eight_handle) = anvil::spawn(config).await;
    let monad_eight_endpoint = monad_eight_handle.http_endpoint();
    let reserve_balance_address = MONAD_RESERVE_BALANCE_ADDRESS.to_string();
    let input = format!("0x{}", hex::encode(MONAD_DIPPED_INTO_RESERVE_SELECTOR));

    let monad_eight = cmd
        .args([
            "call",
            &reserve_balance_address,
            "--data",
            &input,
            "--rpc-url",
            &monad_eight_endpoint,
            "--trace",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(monad_eight.contains(&reserve_balance_address), "{monad_eight}");
    assert!(!monad_eight.contains("ReserveBalance"), "{monad_eight}");
    assert!(monad_eight.contains("[Stop]"), "{monad_eight}");
    assert!(!monad_eight.contains("[Return] false"), "{monad_eight}");

    let monad_eight_rpc = cmd
        .cast_fuse()
        .args([
            "call",
            &reserve_balance_address,
            "--data",
            &input,
            "--rpc-url",
            &monad_eight_endpoint,
            "--debug-trace-call",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(monad_eight_rpc.contains(&reserve_balance_address), "{monad_eight_rpc}");
    assert!(!monad_eight_rpc.contains("ReserveBalance"), "{monad_eight_rpc}");

    let activation = MONAD_NINE_TESTNET_ACTIVATION_TIMESTAMP.to_string();
    let monad_eight_with_later_timestamp = cmd
        .cast_fuse()
        .args([
            "call",
            &reserve_balance_address,
            "--data",
            &input,
            "--rpc-url",
            &monad_eight_endpoint,
            "--trace",
            "--block.time",
            &activation,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(
        monad_eight_with_later_timestamp.contains(&reserve_balance_address),
        "{monad_eight_with_later_timestamp}"
    );
    assert!(
        !monad_eight_with_later_timestamp.contains("ReserveBalance"),
        "{monad_eight_with_later_timestamp}"
    );
    assert!(
        monad_eight_with_later_timestamp.contains("[Stop]"),
        "{monad_eight_with_later_timestamp}"
    );
    assert!(
        !monad_eight_with_later_timestamp.contains("[Return] false"),
        "{monad_eight_with_later_timestamp}"
    );

    let config = NodeConfig::test_monad()
        .with_hardfork(Some(foundry_evm::hardfork::MonadHardfork::MonadNine.into()))
        .with_chain_id(Some(MONAD_TESTNET_CHAIN_ID))
        .with_genesis_timestamp(Some(MONAD_NINE_TESTNET_ACTIVATION_TIMESTAMP));
    let (_origin_api, monad_nine_origin) = anvil::spawn(config).await;
    let config = NodeConfig::test_monad()
        .with_chain_id(Some(1u64))
        .with_no_storage_caching(true)
        .with_eth_rpc_url(Some(monad_nine_origin.http_endpoint()))
        .with_fork_block_number(Some(0u64));
    let (_api, monad_nine_handle) = anvil::spawn(config).await;
    let monad_nine_endpoint = monad_nine_handle.http_endpoint();
    let monad_nine = cmd
        .cast_fuse()
        .args([
            "call",
            &reserve_balance_address,
            "--data",
            &input,
            "--rpc-url",
            &monad_nine_endpoint,
            "--trace",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(monad_nine.contains("ReserveBalance::dippedIntoReserve()"), "{monad_nine}");
    assert!(monad_nine.contains("[Return] false"), "{monad_nine}");

    let monad_nine_rpc = cmd
        .cast_fuse()
        .args([
            "call",
            &reserve_balance_address,
            "--data",
            &input,
            "--rpc-url",
            &monad_nine_endpoint,
            "--debug-trace-call",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(monad_nine_rpc.contains("ReserveBalance::dippedIntoReserve()"), "{monad_nine_rpc}");

    cmd.cast_fuse().env("FOUNDRY_HARDFORK", "monad:MonadEight");
    let explicit_monad_eight = cmd
        .args([
            "call",
            &reserve_balance_address,
            "--data",
            &input,
            "--rpc-url",
            &monad_nine_endpoint,
            "--trace",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(explicit_monad_eight.contains(&reserve_balance_address), "{explicit_monad_eight}");
    assert!(!explicit_monad_eight.contains("ReserveBalance"), "{explicit_monad_eight}");
    assert!(explicit_monad_eight.contains("[Stop]"), "{explicit_monad_eight}");
    assert!(!explicit_monad_eight.contains("[Return] false"), "{explicit_monad_eight}");
});

#[cfg(feature = "monad")]
casttest!(monad_call_trace_uses_parent_sender_context, async |_prj, cmd| {
    let config = NodeConfig::test_monad()
        .with_hardfork(Some(foundry_evm::hardfork::MonadHardfork::MonadNine.into()));
    let (api, handle) = anvil::spawn(config).await;
    let provider = handle.http_provider();
    let sender = provider.get_accounts().await.unwrap()[0];
    api.anvil_set_code(MONAD_RESERVE_PROBE_ADDRESS, MONAD_RESERVE_RETURN_PROBE_CODE.into())
        .await
        .unwrap();
    api.anvil_set_balance(sender, mon(13)).await.unwrap();

    let _ = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(MONAD_RESERVE_PROBE_ADDRESS)
                .with_value(mon(1))
                .with_gas_limit(100_000)
                .into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    let endpoint = handle.http_endpoint();
    let probe = MONAD_RESERVE_PROBE_ADDRESS.to_string();
    let sender = sender.to_string();
    let value = mon(3).to_string();
    let output = cmd
        .args([
            "call",
            &probe,
            "--data",
            "0x",
            "--from",
            &sender,
            "--value",
            &value,
            "--gas-limit",
            "100000",
            "--rpc-url",
            &endpoint,
            "--trace",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(output.contains("ReserveBalance::dippedIntoReserve()"), "{output}");
    assert!(output.contains("[Return] true"), "{output}");
});

#[cfg(feature = "monad")]
casttest!(monad_run_replays_reserve_balance_precompile_tx, async |_prj, cmd| {
    let config = NodeConfig::test_monad()
        .with_hardfork(Some(foundry_evm::hardfork::MonadHardfork::MonadNine.into()));
    let (_api, handle) = anvil::spawn(config).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];
    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(MONAD_RESERVE_BALANCE_ADDRESS)
        .with_input(MONAD_DIPPED_INTO_RESERVE_SELECTOR);
    let receipt = provider.send_transaction(tx.into()).await.unwrap().get_receipt().await.unwrap();

    assert!(receipt.status());

    let endpoint = handle.http_endpoint();
    let tx_hash = receipt.transaction_hash.to_string();
    let output = cmd
        .args(["run", &tx_hash, "--rpc-url", &endpoint, "--quick"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(output.contains("Transaction successfully executed."), "{output}");
    assert!(output.contains("ReserveBalance::dippedIntoReserve()"), "{output}");
    assert!(output.contains("[Return] false"), "{output}");
});

#[cfg(feature = "monad")]
casttest!(monad_run_preserves_endpoint_hardfork, async |_prj, cmd| {
    let origin_config = NodeConfig::test_monad()
        .with_hardfork(Some(foundry_evm::hardfork::MonadHardfork::MonadNine.into()))
        .with_chain_id(Some(MONAD_TESTNET_CHAIN_ID))
        .with_genesis_timestamp(Some(MONAD_NINE_TESTNET_ACTIVATION_TIMESTAMP - 2));
    let (_origin_api, origin_handle) = anvil::spawn(origin_config).await;
    let config = NodeConfig::test_monad()
        .with_chain_id(Some(1u64))
        .with_no_storage_caching(true)
        .with_eth_rpc_url(Some(origin_handle.http_endpoint()))
        .with_fork_block_number(Some(0u64));
    let (api, handle) = anvil::spawn(config).await;
    let provider = handle.http_provider();
    let from = provider.get_accounts().await.unwrap()[0];

    api.evm_set_next_block_timestamp(MONAD_NINE_TESTNET_ACTIVATION_TIMESTAMP - 1).unwrap();
    let pre_activation_receipt = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(from)
                .with_to(MONAD_RESERVE_BALANCE_ADDRESS)
                .with_input(MONAD_DIPPED_INTO_RESERVE_SELECTOR)
                .into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    api.evm_set_next_block_timestamp(MONAD_NINE_TESTNET_ACTIVATION_TIMESTAMP).unwrap();
    let post_activation_receipt = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(from)
                .with_to(MONAD_RESERVE_BALANCE_ADDRESS)
                .with_input(MONAD_DIPPED_INTO_RESERVE_SELECTOR)
                .into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(pre_activation_receipt.status());
    assert!(post_activation_receipt.status());

    let endpoint = handle.http_endpoint();
    let pre_activation_hash = pre_activation_receipt.transaction_hash.to_string();
    let pre_activation = cmd
        .args(["run", &pre_activation_hash, "--rpc-url", &endpoint, "--quick"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(pre_activation.contains("ReserveBalance::dippedIntoReserve()"), "{pre_activation}");
    assert!(pre_activation.contains("[Return] false"), "{pre_activation}");

    let post_activation_hash = post_activation_receipt.transaction_hash.to_string();
    let post_activation = cmd
        .cast_fuse()
        .args(["run", &post_activation_hash, "--rpc-url", &endpoint, "--quick"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(post_activation.contains("ReserveBalance::dippedIntoReserve()"), "{post_activation}");
    assert!(post_activation.contains("[Return] false"), "{post_activation}");

    let pre_activation_rpc_trace = cmd
        .cast_fuse()
        .args(["run", &pre_activation_hash, "--rpc-url", &endpoint, "--debug-trace-transaction"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(
        pre_activation_rpc_trace.contains("ReserveBalance::dippedIntoReserve()"),
        "{pre_activation_rpc_trace}"
    );

    let post_activation_rpc_trace = cmd
        .cast_fuse()
        .args(["run", &post_activation_hash, "--rpc-url", &endpoint, "--debug-trace-transaction"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(
        post_activation_rpc_trace.contains("ReserveBalance::dippedIntoReserve()"),
        "{post_activation_rpc_trace}"
    );
});

#[cfg(feature = "monad")]
casttest!(monad_run_traces_protocol_system_call, async |_prj, cmd| {
    let config = NodeConfig::test_monad()
        .with_hardfork(Some(foundry_evm::hardfork::MonadHardfork::MonadNine.into()));
    let (api, handle) = anvil::spawn(config).await;
    let provider = handle.http_provider();
    api.anvil_impersonate_account(MONAD_SYSTEM_ADDRESS).await.unwrap();
    api.anvil_set_balance(MONAD_SYSTEM_ADDRESS, mon(1)).await.unwrap();

    let snapshot_selector = &keccak256("syscallSnapshot()")[..4];
    let receipt = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(MONAD_SYSTEM_ADDRESS)
                .with_to(MONAD_STAKING_ADDRESS)
                .with_input(snapshot_selector.to_vec())
                .with_gas_limit(1_000_000)
                .into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status());

    let endpoint = handle.http_endpoint();
    let tx_hash = receipt.transaction_hash;
    let tx_hash_string = tx_hash.to_string();
    let original =
        cmd.args(["run", &tx_hash_string, "--rpc-url", &endpoint, "--quick"]).assert_failure();
    let original = original.get_output();
    assert!(
        original.stderr_lossy().contains("invalid Monad protocol system transaction"),
        "{}",
        original.stderr_lossy()
    );
    assert!(
        !original.stdout_lossy().contains("Staking::syscallSnapshot()"),
        "{}",
        original.stdout_lossy()
    );

    let canonical_endpoint =
        foundry_test_utils::rpc::spawn_canonical_monad_system_rpc(endpoint.clone(), tx_hash).await;
    let output = cmd
        .cast_fuse()
        .args(["run", &tx_hash_string, "--rpc-url", &canonical_endpoint, "--quick"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(output.contains("Staking::syscallSnapshot()"), "{output}");
    assert!(output.contains("Transaction successfully executed."), "{output}");
    assert!(!output.contains("0x0000000000000000000000000000000000000000::fallback()"), "{output}");

    let unrelated_system = address!("deaddeaddeaddeaddeaddeaddeaddeaddead0001");
    api.anvil_impersonate_account(unrelated_system).await.unwrap();
    api.anvil_set_balance(unrelated_system, mon(1)).await.unwrap();
    let unrelated_receipt = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(unrelated_system)
                .with_to(Address::with_last_byte(1))
                .with_gas_limit(100_000)
                .into(),
        )
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(unrelated_receipt.status());
    let unrelated_hash = unrelated_receipt.transaction_hash.to_string();

    cmd.cast_fuse()
        .args(["run", &unrelated_hash, "--rpc-url", &endpoint, "--quick"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: 0x[..] is a system transaction.
Replaying system transactions is currently not supported.

"#]]);
    cmd.cast_fuse()
        .args(["run", &unrelated_hash, "--rpc-url", &endpoint, "--quick", "--replay-system-txes"])
        .assert_success();
});

#[cfg(feature = "monad")]
casttest!(monad_run_replays_current_sender_context, async |_prj, cmd| {
    let config = NodeConfig::test_monad()
        .with_hardfork(Some(foundry_evm::hardfork::MonadHardfork::MonadNine.into()));
    let (api, handle) = anvil::spawn(config).await;
    let provider = handle.http_provider();
    let sender = provider.get_accounts().await.unwrap()[0];
    api.anvil_set_code(MONAD_RESERVE_PROBE_ADDRESS, MONAD_RESERVE_RETURN_PROBE_CODE.into())
        .await
        .unwrap();
    api.anvil_set_balance(sender, mon(12)).await.unwrap();
    api.mine_one().await.unwrap();
    api.anvil_set_auto_mine(false).await.unwrap();

    let _ = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(MONAD_RESERVE_PROBE_ADDRESS)
                .with_nonce(0)
                .with_value(mon(2))
                .with_gas_limit(100_000)
                .into(),
        )
        .await
        .unwrap();
    let second = provider
        .send_transaction(
            TransactionRequest::default()
                .with_from(sender)
                .with_to(MONAD_RESERVE_PROBE_ADDRESS)
                .with_nonce(1)
                .with_value(mon(1))
                .with_gas_limit(100_000)
                .into(),
        )
        .await
        .unwrap();
    api.mine_one().await.unwrap();
    let receipt = second.get_receipt().await.unwrap();

    let endpoint = handle.http_endpoint();
    let tx_hash = receipt.transaction_hash.to_string();
    let output = cmd
        .args(["run", &tx_hash, "--rpc-url", &endpoint])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(output.contains("ReserveBalance::dippedIntoReserve()"), "{output}");
    assert!(output.contains("[Return] true"), "{output}");
});
