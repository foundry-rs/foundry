use alloy_consensus::Transaction;
use alloy_eips::Typed2718;
use alloy_primitives::{Address, B256, address, b256, hex};
use alloy_provider::Provider;
use alloy_rpc_types::BlockNumberOrTag;
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use anvil::NodeConfig;
use axum::{Json, Router, extract::RawQuery};
use foundry_cli::json::JsonEnvelope;
use foundry_test_utils::util::OutputExt;
use serde_json::{Value, json};

const ANVIL_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const SIMULATION_SAFE_CODE: &str = "0x63d8d11f785f3560e01c146051577333333333333333333333333333333333333333333214602b575f80fd5b60015f5260a0602052602a60405260016060526060608052602060a0523260c05260e05ffd5b5f805260205ff3";

fn safe_transaction(safe: Address, operation: u8) -> Value {
    json!({
        "safe": safe,
        "to": Address::ZERO,
        "value": "0",
        "data": "0x",
        "operation": operation,
        "safeTxGas": "0",
        "baseGas": "0",
        "gasPrice": "0",
        "gasToken": Address::ZERO,
        "refundReceiver": Address::ZERO,
        "nonce": "0",
        "safeTxHash": B256::ZERO,
    })
}

async fn spawn_safe_service(response: Value) -> String {
    let router = Router::new().fallback(move || {
        let response = response.clone();
        async move { Json(response) }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    endpoint
}

fn json_envelope(data: Value) -> String {
    serde_json::to_string(&JsonEnvelope::success(data)).unwrap()
}

casttest!(safe_commands_are_exposed, |_prj, cmd| {
    let output =
        cmd.cast_fuse().args(["safe", "--help"]).assert_success().get_output().stdout_lossy();
    for command in [
        "create",
        "add-delegate",
        "list-delegates",
        "remove-delegate",
        "propose",
        "sign",
        "simulate",
        "execute",
    ] {
        assert!(output.contains(command), "expected `cast safe {command}` in help:\n{output}");
    }
});

casttest!(safe_signing_commands_support_hardware_wallets, |_prj, cmd| {
    for command in ["create", "add-delegate", "remove-delegate", "propose", "sign", "execute"] {
        let output = cmd
            .cast_fuse()
            .args(["safe", command, "--help"])
            .assert_success()
            .get_output()
            .stdout_lossy();
        assert!(output.contains("--ledger"), "expected Ledger support in help:\n{output}");
        assert!(output.contains("--trezor"), "expected Trezor support in help:\n{output}");
    }
});

casttest!(safe_onchain_commands_support_tempo_transaction_options, |_prj, cmd| {
    for command in ["create", "execute"] {
        let output = cmd
            .cast_fuse()
            .args(["safe", command, "--help"])
            .assert_success()
            .get_output()
            .stdout_lossy();
        assert!(
            output.contains("--tempo.fee-token"),
            "expected Tempo fee-token support in help:\n{output}"
        );
        assert!(
            output.contains("--tempo.nonce-key"),
            "expected Tempo nonce-key support in help:\n{output}"
        );
    }
});

casttest!(safe_create_honors_transaction_options, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let singleton = address!("1111111111111111111111111111111111111111");
    let factory = address!("2222222222222222222222222222222222222222");

    api.anvil_set_code(singleton, "0x00".parse().unwrap()).await.unwrap();
    // mstore(singleton); emit ProxyCreation(proxy, singleton); return proxy.
    api.anvil_set_code(
        factory,
        "0x7311111111111111111111111111111111111111115f527333333333333333333333333333333333333333337f4f51faf6c4561ff95f067657e43439f0f856d97c04d9ec9070a6199ad418e23560205fa27333333333333333333333333333333333333333335f5260205ff3"
            .parse()
            .unwrap(),
    )
    .await
    .unwrap();

    let owner = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    let singleton = singleton.to_string();
    let factory = factory.to_string();
    let common_args = [
        "safe",
        "create",
        owner,
        "--singleton",
        &singleton,
        "--factory",
        &factory,
        "--fallback-handler",
        "0x0000000000000000000000000000000000000000",
        "--private-key",
        ANVIL_KEY,
        "--rpc-url",
        &rpc,
    ];

    cmd.cast_fuse().args(common_args).arg("--legacy").assert_success();
    let block =
        provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
    let transaction = block.transactions.as_transactions().unwrap().last().unwrap();
    assert_eq!(transaction.ty(), 0);

    cmd.cast_fuse()
        .args(common_args)
        .args([
            "--access-list",
            r#"[{"address":"0x4444444444444444444444444444444444444444","storageKeys":["0x5555555555555555555555555555555555555555555555555555555555555555"]}]"#,
        ])
        .assert_success();
    let block =
        provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
    let transaction = block.transactions.as_transactions().unwrap().last().unwrap();
    let access_list = transaction.access_list().expect("explicit access list");
    assert_eq!(access_list.len(), 1);
    assert_eq!(access_list[0].address, address!("4444444444444444444444444444444444444444"));
    assert_eq!(
        access_list[0].storage_keys,
        [b256!("5555555555555555555555555555555555555555555555555555555555555555")]
    );
});

casttest!(safe_service_mutations_emit_json_envelopes, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let safe = address!("1111111111111111111111111111111111111111");
    let delegate = address!("2222222222222222222222222222222222222222");
    let target = address!("3333333333333333333333333333333333333333");
    api.anvil_set_code(safe, "0x5f5f5260205ff3".parse().unwrap()).await.unwrap();
    let service = spawn_safe_service(safe_transaction(safe, 0)).await;
    let safe = safe.to_string();
    let delegate = delegate.to_string();
    let target = target.to_string();
    let safe_tx_hash = B256::ZERO.to_string();
    let signer_args =
        ["--service-url", service.as_str(), "--rpc-url", rpc.as_str(), "--private-key", ANVIL_KEY];

    cmd.cast_fuse()
        .args(["--json", "safe", "add-delegate", &safe, &delegate, "--label", "test"])
        .args(signer_args)
        .assert_json_stdout(json_envelope(json!(delegate)));

    cmd.cast_fuse()
        .args(["--json", "safe", "remove-delegate", &safe, &delegate])
        .args(signer_args)
        .assert_json_stdout(json_envelope(json!(delegate)));

    cmd.cast_fuse()
        .args(["--json", "safe", "propose", &safe, &target, "--nonce", "0"])
        .args(signer_args)
        .assert_json_stdout(json_envelope(json!(B256::ZERO)));

    let signer: PrivateKeySigner = ANVIL_KEY.parse().unwrap();
    let mut signature = signer.sign_message(B256::ZERO.as_slice()).await.unwrap().as_bytes();
    signature[64] += 4;
    let signature = hex::encode_prefixed(signature);
    cmd.cast_fuse()
        .args(["--json", "safe", "sign", &safe, &safe_tx_hash])
        .args(signer_args)
        .assert_json_stdout(json_envelope(json!(signature)));
});

casttest!(safe_list_delegates_follows_pagination, async |_prj, cmd| {
    let safe = address!("1111111111111111111111111111111111111111");
    let first = json!({
        "safe": safe,
        "delegate": address!("2222222222222222222222222222222222222222"),
        "delegator": address!("3333333333333333333333333333333333333333"),
        "label": "first",
    });
    let second = json!({
        "safe": safe,
        "delegate": address!("4444444444444444444444444444444444444444"),
        "delegator": address!("5555555555555555555555555555555555555555"),
        "label": "second",
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let service = format!("http://{}", listener.local_addr().unwrap());
    let next = format!("{service}/api/v2/delegates/?page=2");
    let first_response = first.clone();
    let second_response = second.clone();
    let router = Router::new().fallback(move |RawQuery(query): RawQuery| {
        let first = first_response.clone();
        let second = second_response.clone();
        let next = next.clone();
        async move {
            let is_second_page =
                query.as_deref().is_some_and(|query| query.split('&').any(|pair| pair == "page=2"));
            Json(if is_second_page {
                json!({ "count": 2, "next": null, "previous": null, "results": [second] })
            } else {
                json!({ "count": 2, "next": next, "previous": null, "results": [first] })
            })
        }
    });
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    cmd.cast_fuse()
        .args(["--json", "safe", "list-delegates", &safe.to_string(), "--service-url", &service])
        .assert_json_stdout(json_envelope(json!([first, second])));
});

casttest!(safe_list_delegates_rejects_external_next_page, async |_prj, cmd| {
    let service = spawn_safe_service(json!({
        "count": 1,
        "next": "http://127.0.0.1:1/collect-api-key",
        "previous": null,
        "results": [],
    }))
    .await;
    let stderr = cmd
        .cast_fuse()
        .args([
            "safe",
            "list-delegates",
            "0x1111111111111111111111111111111111111111",
            "--service-url",
            &service,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        stderr.contains("delegate pagination URL points outside the Transaction Service endpoint"),
        "unexpected error: {stderr}"
    );
});

casttest!(safe_sign_rejects_service_selected_safe, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let expected = address!("1111111111111111111111111111111111111111");
    let malicious = address!("2222222222222222222222222222222222222222");
    api.anvil_set_code(malicious, "0x5f5f5260205ff3".parse().unwrap()).await.unwrap();
    let service = spawn_safe_service(safe_transaction(malicious, 0)).await;
    let expected_arg = expected.to_string();
    let safe_tx_hash = B256::ZERO.to_string();

    let stderr = cmd
        .cast_fuse()
        .args([
            "safe",
            "sign",
            &expected_arg,
            &safe_tx_hash,
            "--service-url",
            &service,
            "--rpc-url",
            &rpc,
            "--private-key",
            ANVIL_KEY,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        stderr.contains(&format!(
            "Transaction Service returned Safe {malicious}, expected {expected}"
        )),
        "unexpected error: {stderr}"
    );
});

casttest!(safe_simulation_requires_executor, |_prj, cmd| {
    let safe = address!("1111111111111111111111111111111111111111").to_string();
    let safe_tx_hash = B256::ZERO.to_string();

    cmd.cast_fuse();
    cmd.unset_env("ETH_FROM");
    let stderr = cmd
        .args(["safe", "simulate", &safe, &safe_tx_hash, "--rpc-url", "http://127.0.0.1:1"])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        stderr.contains("the following required arguments were not provided")
            && stderr.contains("--from <ADDRESS>"),
        "unexpected error: {stderr}"
    );
});

casttest!(safe_simulation_uses_executor_context, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let safe = address!("1111111111111111111111111111111111111111");
    let executor = address!("3333333333333333333333333333333333333333");
    let accessor = address!("4444444444444444444444444444444444444444");
    // Return a zero transaction hash, then require `tx.origin` to be `executor` and return it as
    // the simulated call result.
    api.anvil_set_code(safe, SIMULATION_SAFE_CODE.parse().unwrap()).await.unwrap();
    api.anvil_set_code(accessor, "0x00".parse().unwrap()).await.unwrap();
    let safe_arg = safe.to_string();
    let executor_arg = executor.to_string();
    let accessor_arg = accessor.to_string();
    let safe_tx_hash = B256::ZERO.to_string();

    for operation in [0, 1] {
        let service = spawn_safe_service(safe_transaction(safe, operation)).await;
        cmd.cast_fuse()
            .args([
                "--json",
                "safe",
                "simulate",
                &safe_arg,
                &safe_tx_hash,
                "--from",
                &executor_arg,
                "--accessor",
                &accessor_arg,
                "--service-url",
                &service,
                "--rpc-url",
                &rpc,
            ])
            .assert_json_stdout(json_envelope(json!({
                "safeTxHash": B256::ZERO,
                "success": true,
                "gasUsed": "42",
                "returnData": "0x0000000000000000000000003333333333333333333333333333333333333333",
            })));
    }
});

casttest!(safe_simulation_rejects_reimbursed_transaction, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let safe = address!("1111111111111111111111111111111111111111");
    let executor = address!("3333333333333333333333333333333333333333");
    let accessor = address!("4444444444444444444444444444444444444444");
    api.anvil_set_code(safe, SIMULATION_SAFE_CODE.parse().unwrap()).await.unwrap();
    api.anvil_set_code(accessor, "0x00".parse().unwrap()).await.unwrap();
    let mut transaction = safe_transaction(safe, 0);
    transaction["gasPrice"] = json!("1");
    let service = spawn_safe_service(transaction).await;

    let stderr = cmd
        .cast_fuse()
        .args([
            "safe",
            "simulate",
            &safe.to_string(),
            &B256::ZERO.to_string(),
            "--from",
            &executor.to_string(),
            "--accessor",
            &accessor.to_string(),
            "--service-url",
            &service,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        stderr.contains("cannot simulate reimbursed Safe transactions (gasPrice > 0)"),
        "unexpected error: {stderr}"
    );
});
