//! CLI tests for call trace commands.

use super::*;

// https://github.com/foundry-rs/foundry/issues/9476
forgetest_async!(cast_call_custom_chain_id, |_prj, cmd| {
    let chain_id = 55555u64;
    let (_api, handle) = anvil::spawn(NodeConfig::test().with_chain_id(Some(chain_id))).await;

    let http_endpoint = handle.http_endpoint();

    cmd.cast_fuse()
        .args([
            "call",
            "5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &http_endpoint,
            "--chain",
            &chain_id.to_string(),
        ])
        .assert_success();
});

// https://github.com/foundry-rs/foundry/issues/10848
forgetest_async!(cast_call_disable_labels, |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.initialize_default_contracts();
    prj.add_source(
        "Counter",
        r#"
contract Counter {
    uint256 public number;

    function getBalance(address target) public returns (uint256) {
        return target.balance;
    }
}
   "#,
    );

    // Deploy counter contract.
    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "CounterScript",
    ])
    .assert_success();

    // Override state, `number()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x1234",
            "number()(uint256)",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
4660

"#]]);

    // Override state, `number()` should return overridden value.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--labels",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:WETH",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x1234",
            "number()(uint256)",
            "--trace",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [2402] WETH::number()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000001234


Transaction successfully executed.
[GAS]

"#]]);

    // Override state, `number()` with `disable_labels`.
    cmd.cast_fuse()
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--labels",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:WETH",
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-state",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3:0x0:0x1234",
            "number()(uint256)",
            "--trace",
            "--disable-labels",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [2402] 0x5FbDB2315678afecb367f032d93F642f64180aa3::number()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000001234


Transaction successfully executed.
[GAS]

"#]]);
});

// --debug-trace-call with --with-local-artifacts labels the called contract by its local
// artifact name (Counter::) instead of the raw address. Without the RPC bytecode-map fetch the
// trace falls back to the bare address, so this test can fail.
forgetest_async!(cast_call_debug_trace_call_with_local_artifacts, |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.initialize_default_contracts();
    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "CounterScript",
    ])
    .assert_success();

    cmd.cast_fuse();
    cmd.set_current_dir(prj.root());
    cmd.args([
        "call",
        "0x5FbDB2315678afecb367f032d93F642f64180aa3",
        "number()(uint256)",
        "--debug-trace-call",
        "--with-local-artifacts",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Traces:
  [23488] Counter::number()
    └─ ← [Return] 0


Transaction successfully executed.
[GAS]

"#]]);
});

// `--debug-trace-call --with-local-artifacts` must label a contract that only exists through a
// `--override-code` state override: the trace runs the override code, so artifact matching must
// see that code instead of the (empty) on-chain code.
forgetest_async!(cast_call_debug_trace_call_override_code_local_artifacts, |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.initialize_default_contracts();

    // Deploy counter contract, only to read its runtime bytecode back.
    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "CounterScript",
    ])
    .assert_success();

    let runtime_code = cmd
        .cast_fuse()
        .args([
            "code",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    // Call an address that has no code on chain, overriding it with Counter's runtime code.
    cmd.cast_fuse();
    cmd.set_current_dir(prj.root());
    let output = cmd
        .args([
            "call",
            "0x00000000000000000000000000000000000000aa",
            "number()(uint256)",
            "--debug-trace-call",
            "--with-local-artifacts",
            "--override-code",
            &format!("0x00000000000000000000000000000000000000aa:{runtime_code}"),
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(
        output.contains("Counter::number()"),
        "expected the override-code contract to be labeled from local artifacts:\n{output}"
    );
});

// `--json --debug-trace-call --with-local-artifacts` must keep stdout machine-readable: the
// compile banner/progress goes to stderr, so stdout is exactly one JSON document.
forgetest_async!(cast_call_debug_trace_call_local_artifacts_json_stdout, |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.initialize_default_contracts();

    // Deploy counter contract.
    cmd.args([
        "script",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &handle.http_endpoint(),
        "--broadcast",
        "CounterScript",
    ])
    .assert_success();

    cmd.cast_fuse();
    cmd.set_current_dir(prj.root());
    let output = cmd
        .args([
            "call",
            "0x5FbDB2315678afecb367f032d93F642f64180aa3",
            "number()(uint256)",
            "--debug-trace-call",
            "--with-local-artifacts",
            "--json",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    serde_json::from_str::<serde_json::Value>(output.trim()).unwrap_or_else(|err| {
        panic!("expected stdout to be a single JSON document ({err}):\n{output}")
    });
});

// `cast call --trace` decodes custom errors through the local signatures cache that `forge build`
// populates, without requiring `--with-local-artifacts`.
// <https://github.com/foundry-rs/foundry/issues/11085>
forgetest_async!(flaky_cast_call_trace_decodes_error_from_signatures_cache, |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source(
        "CustomErrorContract",
        r#"
contract CustomErrorContract {
    error WTF273987(uint256 a, uint256 b);
    error PrintableError18();

    function wtf2890230(uint256 a, uint128 b) external pure {
        revert WTF273987(a, b);
    }

    function printableError() external pure {
        revert PrintableError18();
    }
}
"#,
    );

    // `forge build` caches the project's signatures, including custom errors.
    cmd.args(["build"]).assert_success();

    // Deploy the contract.
    cmd.forge_fuse()
        .args([
            "create",
            "./src/CustomErrorContract.sol:CustomErrorContract",
            "--broadcast",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success();

    // The revert reason is decoded from the cached signatures, even offline and without
    // `--with-local-artifacts`.
    cmd.cast_fuse().env("FOUNDRY_OFFLINE", "true");
    cmd.args([
        "call",
        "0x5FbDB2315678afecb367f032d93F642f64180aa3",
        "wtf2890230(uint256,uint128)",
        "42",
        "69",
        "--trace",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Traces:
  [..] 0x5FbDB2315678afecb367f032d93F642f64180aa3::wtf2890230(42, 69)
    └─ ← [Revert] WTF273987(42, 69)


[GAS]

"#]])
    .stderr_eq(str![[r#"
Error: Transaction failed.

"#]]);

    // A custom error whose selector is valid ASCII must also be decoded from the cache rather than
    // rendered as a raw string. `PrintableError18()` has selector `0x2e2c426f` (`.,Bo`).
    cmd.cast_fuse().env("FOUNDRY_OFFLINE", "true");
    cmd.args([
        "call",
        "0x5FbDB2315678afecb367f032d93F642f64180aa3",
        "printableError()",
        "--trace",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Traces:
  [..] 0x5FbDB2315678afecb367f032d93F642f64180aa3::printableError()
    └─ ← [Revert] PrintableError18()


[GAS]

"#]])
    .stderr_eq(str![[r#"
Error: Transaction failed.

"#]]);
});

// tests that cast call --trace selects TempoEvmNetwork when Tempo is inferred from
// the fork RPC, or when a Tempo chain ID is provided explicitly via --chain.
casttest!(cast_call_trace_selects_tempo_network, async |_prj, cmd| {
    let (_, tempo_handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let (_, eth_handle) = anvil::spawn(NodeConfig::test()).await;

    let token = PATH_USD_ADDRESS.to_string();
    for (name, rpc, extra_args) in [
        ("inferred Tempo RPC", tempo_handle.http_endpoint(), Vec::<&str>::new()),
        ("explicit Tempo --chain", eth_handle.http_endpoint(), vec!["--chain", "4217"]),
    ] {
        cmd.cast_fuse();
        let mut args = vec!["call", &token, "decimals()(uint8)", "--rpc-url", &rpc, "--trace"];
        args.extend(extra_args);

        let output = cmd.args(args).assert_success().get_output().stdout_lossy();

        assert!(
            output.contains("PathUSD::decimals()") && output.contains("← [Return] 6"),
            "expected traced Tempo TIP20 call to execute successfully for {name}, got:\n{output}"
        );
    }
});

// tests that `cast call --trace` executes the call with the configured gas limit or the limit given
// via `--gas-limit` rather than running with an unbounded gas limit.
// <https://github.com/foundry-rs/foundry/issues/15357>
forgetest_async!(cast_call_trace_respects_gas_limits, |prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    prj.update_config(|config| config.gas_limit = 1_000_000.into());

    // Contract that reverts when the call is given an unrealistically large gas limit.
    prj.add_source(
        "GasDependent",
        r#"
contract GasDependent {
    function run(uint256 maxGas) external view returns (uint256) {
        require(gasleft() < maxGas, "unrealistic gas limit");
        return gasleft();
    }
}
"#,
    );
    cmd.forge_fuse().args(["build"]).assert_success();

    let bytecode_path = prj.root().join("out/GasDependent.sol/GasDependent.json");
    let contract_json = std::fs::read_to_string(bytecode_path).unwrap();
    let contract_data: serde_json::Value = serde_json::from_str(&contract_json).unwrap();
    let bytecode = contract_data["bytecode"]["object"].as_str().unwrap();

    let deploy_output = cmd
        .cast_fuse()
        .args([
            "send",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--rpc-url",
            &endpoint,
            "--json",
            "--create",
            bytecode,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let receipt: serde_json::Value = serde_json::from_str(&deploy_output).unwrap();
    let address = receipt["contractAddress"].as_str().unwrap().to_string();

    // A plain `cast call` (eth_call) uses a realistic gas limit, so it succeeds.
    cmd.cast_fuse()
        .args(["call", &address, "run(uint256)(uint256)", "30000000", "--rpc-url", &endpoint])
        .assert_success();

    // `--trace` uses the configured gas limit when no CLI limit is provided.
    cmd.cast_fuse()
        .args([
            "call",
            &address,
            "run(uint256)(uint256)",
            "2000000",
            "--trace",
            "--rpc-url",
            &endpoint,
        ])
        .assert_success();

    // `--trace` with an explicit `--gas-limit` executes the call with that limit, so it succeeds
    // too instead of reverting.
    let trace_output = cmd
        .cast_fuse()
        .args([
            "call",
            &address,
            "run(uint256)(uint256)",
            "750000",
            "--trace",
            "--gas-limit",
            "500000",
            "--rpc-url",
            &endpoint,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(
        trace_output.contains("[Return]") && !trace_output.contains("unrealistic gas limit"),
        "expected traced call to respect --gas-limit and succeed, got:\n{trace_output}"
    );
});

casttest!(cast_call_disables_external_identification, async |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    // Leave the listener unserved: correct flag propagation prevents a connection, while an
    // enabled identifier connects before exhausting the configured timeout.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let etherscan_url = format!("http://{}", listener.local_addr().unwrap());
    let target = Address::random().to_string();
    let override_code = format!("{target}:0x60006000f3");
    fs::write(
        prj.root().join("foundry.toml"),
        format!(
            r#"[profile.default]
etherscan_api_key = "local"
eth_rpc_no_proxy = true
offline = false

[tracing]
external_identification_timeout = 1

[etherscan]
local = {{ key = "test", url = "{etherscan_url}" }}
"#,
        ),
    )
    .unwrap();

    for var in [
        "ETHERSCAN_API_KEY",
        "FOUNDRY_CONFIG",
        "FOUNDRY_ETHERSCAN_API_KEY",
        "FOUNDRY_OFFLINE",
        "FOUNDRY_TRACING_EXTERNAL_IDENTIFICATION_TIMEOUT",
    ] {
        cmd.unset_env(var);
    }
    let assert = cmd
        .args([
            "call",
            &target,
            "--rpc-url",
            &handle.http_endpoint(),
            "--override-code",
            &override_code,
            "--trace",
            "--disable-external-identification",
        ])
        .assert_success();
    let stdout = assert.get_output().stdout_lossy().to_lowercase();
    assert!(
        stdout.contains("traces:") && stdout.contains(&target.to_lowercase()),
        "expected trace for {target}, got:\n{stdout}"
    );

    match listener.accept() {
        Err(err) if err.kind() == ErrorKind::WouldBlock => {}
        Ok(_) => panic!("external identification made an Etherscan request"),
        Err(err) => panic!("failed to inspect mock Etherscan listener: {err}"),
    }
});

// https://github.com/foundry-rs/foundry/issues/10189
// `cast call --debug-trace-call` fetches the call trace from the node via `debug_traceCall`
// (callTracer) and renders it with the same decoding/rendering machinery as `--trace`. The call
// targets the identity precompile so the test needs no deployed contract.
casttest!(cast_call_debug_trace_call, async |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    fs::write(
        prj.root().join("foundry.toml"),
        r#"[labels]
        0x0000000000000000000000000000000000000001 = "unused"

        [tracing]
        decode_internal = true
        "#,
    )
    .unwrap();

    cmd.cast_fuse();
    cmd.set_current_dir(prj.root());
    cmd
        .args([
            "call",
            "0x0000000000000000000000000000000000000004",
            "--data",
            "0xdeadbeef",
            "--debug-trace-call",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [21160] PRECOMPILES::identity(0xdeadbeef)
    └─ ← [Return] 0xdeadbeef


Transaction successfully executed.
[GAS]

"#]])
        .stderr_eq(str![[r#"
Warning: Key `[labels]` is being deprecated in favor of `[tracing.labels]`. It will be removed in future versions.

"#]]);
});

// `--debug-trace-call` must honour state overrides: here we override the code of an address with a
// tiny runtime that returns storage slot 0, and override slot 0 itself, then check the traced call
// returns the overridden value. If the overrides were not forwarded to `debug_traceCall`, the
// address would have no code and the return would not be the overridden value, so this test can
// fail.
casttest!(cast_call_debug_trace_call_applies_overrides, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    cmd.cast_fuse()
        .args([
            "call",
            "0x00000000000000000000000000000000000000aa",
            "number()(uint256)",
            "--debug-trace-call",
            // runtime: PUSH1 0 SLOAD PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN
            "--override-code",
            "0x00000000000000000000000000000000000000aa:0x60005460005260206000f3",
            "--override-state",
            "0x00000000000000000000000000000000000000aa:0x0:0x1234",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [23182] 0x00000000000000000000000000000000000000AA::number()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000001234


Transaction successfully executed.
[GAS]

"#]]);
});

// `--debug-trace-call` must also forward block overrides: override an address with a runtime that
// returns `block.number` and pass `--block.number`, then check the traced call returns that number.
// If `with_block_overrides` were not forwarded to `debug_traceCall`, the call would run at anvil's
// real block number and the return would differ, so this test can fail.
casttest!(cast_call_debug_trace_call_applies_block_overrides, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    cmd.cast_fuse()
        .args([
            "call",
            "0x00000000000000000000000000000000000000bb",
            "number()(uint256)",
            "--debug-trace-call",
            // runtime: NUMBER PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN
            "--override-code",
            "0x00000000000000000000000000000000000000bb:0x4360005260206000f3",
            "--block.number",
            "1234",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [21160] 0x00000000000000000000000000000000000000bb::number()
    └─ ← [Return] 0x00000000000000000000000000000000000000000000000000000000000004d2


Transaction successfully executed.
[GAS]

"#]]);
});

// The motivating invocation in issue #11521 is `cast call --trace --from <safe> <to>`, so the
// delegate override must reach the local tracing executor as well. The trace shows the call
// executing at the sender address, which is where the delegated code runs.
casttest!(cast_call_delegate_trace_uses_sender_storage, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let from = "0x00000000000000000000000000000000000000d7";
    let to = "0x00000000000000000000000000000000000000d8";

    // runtime: PUSH1 0 SLOAD PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN
    api.anvil_set_code(to.parse().unwrap(), "0x60005460005260206000f3".parse().unwrap())
        .await
        .unwrap();
    api.anvil_set_storage_at(from.parse().unwrap(), U256::ZERO, B256::from(U256::from(0x1234)))
        .await
        .unwrap();

    cmd.cast_fuse()
        .args([
            "call",
            to,
            "number()(uint256)",
            "--from",
            from,
            "--delegate",
            "--trace",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [..] 0x00000000000000000000000000000000000000d7::number()
    └─ ← [Return] 0x0000000000000000000000000000000000000000000000000000000000001234


Transaction successfully executed.
[GAS]

"#]]);
});

// `--debug-trace-call` must render a reverting call as `[Revert]` (success = false), exercising
// `status_from_frame` and the failure rendering end-to-end. The overridden runtime just reverts.
casttest!(cast_call_debug_trace_call_renders_revert, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    cmd.cast_fuse()
        .args([
            "call",
            "0x00000000000000000000000000000000000000dd",
            "run()",
            "--debug-trace-call",
            // runtime: PUSH1 0 PUSH1 0 REVERT
            "--override-code",
            "0x00000000000000000000000000000000000000dd:0x60006000fd",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [21160] 0x00000000000000000000000000000000000000dd::run()
    └─ ← [Revert] execution reverted


[GAS]

"#]]);
});

// tests that `--debug-trace-call --curl` emits a `debug_traceCall` request with the
// callTracer, not a plain `eth_call`
casttest!(curl_call_debug_trace_call, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let to = "0xdead000000000000000000000000000000000000";

    let output = cmd
        .args(["call", to, "number()(uint256)", "--rpc-url", rpc, "--debug-trace-call", "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains(rpc));
    assert!(output.contains("debug_traceCall"), "expected debug_traceCall method:\n{output}");
    assert!(output.contains("callTracer"), "expected callTracer tracer param:\n{output}");
    assert!(!output.contains("eth_call"), "unexpected eth_call request:\n{output}");
});

// tests that `--debug-trace-call --curl` forwards state and block overrides in the request,
// like the non-curl path does, so the printed request traces the same state
casttest!(curl_call_debug_trace_call_forwards_overrides, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let to = "0xdead000000000000000000000000000000000000";

    let output = cmd
        .args([
            "call",
            to,
            "number()(uint256)",
            "--rpc-url",
            rpc,
            "--debug-trace-call",
            "--override-code",
            "0x00000000000000000000000000000000000000aa:0x60005460005260206000f3",
            "--block.number",
            "1234",
            "--curl",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(output.contains("debug_traceCall"), "expected debug_traceCall method:\n{output}");
    assert!(output.contains("stateOverrides"), "expected state overrides in params:\n{output}");
    assert!(
        output.contains("0x60005460005260206000f3"),
        "expected the override code in params:\n{output}"
    );
    assert!(output.contains("blockOverrides"), "expected block overrides in params:\n{output}");
});

// tests that `--curl` forwards the scalar transaction fields into the call object, so the
// printed request runs the same call as the non-curl command
casttest!(curl_call_debug_trace_call_forwards_tx_fields, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let to = "0xdead000000000000000000000000000000000000";

    let output = cmd
        .args([
            "call",
            to,
            "number()(uint256)",
            "--rpc-url",
            rpc,
            "--debug-trace-call",
            "--from",
            "0x000000000000000000000000000000000000beef",
            "--value",
            "1ether",
            "--gas-limit",
            "12345",
            "--nonce",
            "7",
            "--curl",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(output.contains("debug_traceCall"), "expected debug_traceCall method:\n{output}");
    assert!(
        output.contains("0x000000000000000000000000000000000000beef"),
        "expected the from address in params:\n{output}"
    );
    assert!(
        output.contains("0xde0b6b3a7640000"),
        "expected the value (1 ether) in params:\n{output}"
    );
    assert!(output.contains("0x3039"), "expected the gas limit (12345) in params:\n{output}");
    assert!(output.contains("nonce"), "expected the nonce in params:\n{output}");
});

// tests that `--labels` / `--disable-labels` are accepted with `--debug-trace-call`, which
// forwards them to the trace renderer like `--trace` does
casttest!(call_labels_accepted_with_debug_trace_call, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let to = "0xdead000000000000000000000000000000000000";

    cmd.args([
        "call",
        to,
        "number()(uint256)",
        "--rpc-url",
        rpc,
        "--debug-trace-call",
        "--labels",
        "0xdead000000000000000000000000000000000000:Counter",
        "--disable-labels",
        "--curl",
    ])
    .assert_success();
});

// tests that `--labels` still requires one of the trace modes
casttest!(call_labels_rejected_without_trace_mode, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let to = "0xdead000000000000000000000000000000000000";

    cmd.args([
        "call",
        to,
        "number()(uint256)",
        "--rpc-url",
        rpc,
        "--labels",
        "0xdead000000000000000000000000000000000000:Counter",
        "--curl",
    ])
    .assert_failure();
});

// `--debug-trace-call` must render a multi-node trace (a call that emits a log AND makes a
// sub-call), exercising the log/sub-call interleaving and nesting through the real pipeline, not
// just in the unit tests. The overridden runtime emits a LOG0 then STATICCALLs the identity
// precompile, so the trace has a child call ordered after the log.
casttest!(cast_call_debug_trace_call_renders_nested_call_and_log, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    cmd.cast_fuse()
        .args([
            "call",
            "0x00000000000000000000000000000000000000cc",
            "run()",
            "--debug-trace-call",
            // runtime: LOG0(0,0); STATICCALL(gas, 0x4, 0,0,0,0); POP; STOP
            "--override-code",
            "0x00000000000000000000000000000000000000cc:0x60006000a060006000600060007300000000000000000000000000000000000000045afa5000",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
Traces:
  [21579] 0x00000000000000000000000000000000000000cc::run()
    ├─           data: 0x
    ├─ [15] PRECOMPILES::identity(0x) [staticcall]
    │   └─ ← [Return] 0x
    └─ ← [Return]


Transaction successfully executed.
[GAS]

"#]]);
});
