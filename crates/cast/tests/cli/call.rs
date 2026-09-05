//! Tests for `cast call`.

use super::*;

casttest!(call_eip7702_auth_disclosure_declined, |_prj, cmd| {
    cmd.args([
        "call",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--chain",
        "31337",
        "--rpc-url",
        "http://127.0.0.1:1",
    ])
    .stdin("n\n")
    .assert_success()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] Aborted.

"#]]);
});

casttest!(call_eip7702_auth_disclosure_requires_signer, |_prj, cmd| {
    cmd.args([
        "call",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--from",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--nonce",
        "0",
        "--chain",
        "31337",
        "--rpc-url",
        "http://127.0.0.1:1",
    ])
    .assert_failure()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Error: No signer available to sign authorization. Provide a pre-signed authorization (hex-encoded) instead.

"#]]);
});

casttest!(call_eip7702_auth_disclosure_accepted_and_forced, async |_prj, cmd| {
    let (api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;
    let endpoint = handle.http_endpoint();
    let delegate_code = "0x602a5f5260205ff3".parse().unwrap();
    api.anvil_set_code(address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8"), delegate_code)
        .await
        .unwrap();
    api.anvil_set_code(
        address!("0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        "0x602a5f5260205ff3".parse().unwrap(),
    )
    .await
    .unwrap();
    let args = [
        "call",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
    ];

    cmd.args(args)
        .arg("--json")
        .stdin("y\n")
        .assert_success()
        .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000000000002a

"#]])
        .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] "#]]);

    cmd.cast_fuse()
        .args(args)
        .arg("--force")
        .assert_success()
        .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000000000002a

"#]])
        .stderr_eq(str![""]);

    cmd.cast_fuse()
        .args(args)
        .args(["--quiet", "--force"])
        .assert_success()
        .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000000000002a

"#]])
        .stderr_eq(str![""]);

    let signer: PrivateKeySigner =
        "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d".parse().unwrap();
    let auth = Authorization {
        chain_id: U256::from(31337),
        address: address!("0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
        nonce: 0,
    };
    let signature = signer.sign_hash(&auth.signature_hash()).await.unwrap();
    let encoded_auth = hex::encode_prefixed(alloy_rlp::encode(auth.into_signed(signature)));

    cmd.cast_fuse()
        .args([
            "call",
            &signer.address().to_string(),
            "--auth",
            &encoded_auth,
            "--from",
            &signer.address().to_string(),
            "--rpc-url",
            &endpoint,
        ])
        .stdin("y\n")
        .assert_success()
        .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000000000002a

"#]])
        .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] "#]]);

    cmd.cast_fuse().args(args).arg("--trace").assert_success().stderr_eq(str![""]);
    cmd.cast_fuse()
        .args(args)
        .args(["--trace", "--access-list", "[]"])
        .assert_success()
        .stderr_eq(str![""]);
});

casttest!(call_eip7702_auth_disclosure_routing, |_prj, cmd| {
    let base_args = [
        "call",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--chain",
        "31337",
        "--rpc-url",
        "http://127.0.0.1:1",
    ];

    cmd.args(base_args)
        .arg("--debug-trace-call")
        .stdin("n\n")
        .assert_success()
        .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] Aborted.

"#]]);

    cmd.cast_fuse()
        .args(base_args)
        .args(["--trace", "--access-list"])
        .stdin("n\n")
        .assert_success()
        .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] Aborted.

"#]]);

    cmd.cast_fuse()
        .args(base_args)
        .arg("--quiet")
        .assert_failure()
        .stderr_eq(str![[r#"
Error: EIP-7702 authorization disclosure requires confirmation; pass `--force` to continue with `--quiet`

"#]]);

    cmd.cast_fuse().args(base_args).arg("--curl").assert_success().stderr_eq(str![""]);
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

// https://github.com/foundry-rs/foundry/issues/11521
// `--delegate` must run the destination's code against the sender's storage. The destination's
// runtime returns slot 0, and only the sender holds a value there, so a plain call returns zero
// and the delegated call returns the sender's value.
casttest!(cast_call_delegate_uses_sender_storage, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let from = "0x00000000000000000000000000000000000000d0";
    let to = "0x00000000000000000000000000000000000000d1";

    cmd.cast_fuse()
        .args([
            "call",
            to,
            "number()(uint256)",
            "--from",
            from,
            "--delegate",
            // runtime: PUSH1 0 SLOAD PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN
            "--override-code",
            &format!("{to}:0x60005460005260206000f3"),
            "--override-state",
            &format!("{from}:0x0:0x1234"),
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
4660

"#]]);

    // Without `--delegate` the same call runs against the destination's own empty storage.
    cmd.cast_fuse()
        .args([
            "call",
            to,
            "number()(uint256)",
            "--from",
            from,
            "--override-code",
            &format!("{to}:0x60005460005260206000f3"),
            "--override-state",
            &format!("{from}:0x0:0x1234"),
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
0

"#]]);
});

// A code override on the sender is what `--delegate` installs, so the two cannot both win.
casttest!(cast_call_delegate_rejects_sender_code_override, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let from = "0x00000000000000000000000000000000000000d2";
    let to = "0x00000000000000000000000000000000000000d3";

    cmd.cast_fuse()
        .args([
            "call",
            to,
            "--from",
            from,
            "--delegate",
            "--override-code",
            &format!("{to}:0x60005460005260206000f3,{from}:0x00"),
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: `--delegate` conflicts with `--override-code` for the sender 0x00000000000000000000000000000000000000D2

"#]]);
});

// The primary `--delegate` path reads the destination's runtime code from the node: no override
// flags are involved, the delegated code and the sender's storage both come from chain state.
casttest!(cast_call_delegate_fetches_code_from_node, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let from = "0x00000000000000000000000000000000000000d4";
    let to = "0x00000000000000000000000000000000000000d5";

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
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
4660

"#]]);

    // A delegated call that returns no data must not trip the empty-code warning: the sender has
    // no on-chain code, but the delegate override guarantees executable code.
    let void = "0x00000000000000000000000000000000000000d6";
    // runtime: STOP
    api.anvil_set_code(void.parse().unwrap(), "0x00".parse().unwrap()).await.unwrap();
    cmd.cast_fuse()
        .args(["call", void, "--from", from, "--delegate", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
0x

"#]])
        .stderr_eq(str![""]);
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

// Documents the identity semantics: the delegated code observes the sender itself as
// `msg.sender`, not the delegating contract's caller as an on-chain `delegatecall` would.
casttest!(cast_call_delegate_msg_sender_is_from, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let from = "0x00000000000000000000000000000000000000d9";
    let to = "0x00000000000000000000000000000000000000da";

    // runtime: CALLER PUSH1 0 MSTORE PUSH1 0x20 PUSH1 0 RETURN
    api.anvil_set_code(to.parse().unwrap(), "0x3360005260206000f3".parse().unwrap()).await.unwrap();

    cmd.cast_fuse()
        .args([
            "call",
            to,
            "sender()(address)",
            "--from",
            from,
            "--delegate",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
0x00000000000000000000000000000000000000D9

"#]]);
});

// `--delegate` needs runtime code at the destination; a codeless address is an explicit error.
casttest!(cast_call_delegate_no_code_destination, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    cmd.cast_fuse()
        .args([
            "call",
            "0x00000000000000000000000000000000000000db",
            "number()(uint256)",
            "--from",
            "0x00000000000000000000000000000000000000dc",
            "--delegate",
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: `--delegate` destination 0x00000000000000000000000000000000000000db has no code to delegate to

"#]]);
});

// `--curl` builds the request offline, but the delegate override needs the destination's code
// from the node.
casttest!(cast_call_delegate_rejects_curl, |_prj, cmd| {
    cmd.args([
        "call",
        "0x00000000000000000000000000000000000000dd",
        "--from",
        "0x00000000000000000000000000000000000000de",
        "--delegate",
        "--curl",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
Error: --delegate cannot be combined with --curl

"#]]);
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

// `dirs::home_dir()` ignores `HOME` on Windows, so the signature cache cannot be isolated there.
#[cfg(not(windows))]
casttest!(cast_call_decodes_custom_error, async |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;

    let signature = "RequestLimitExceeded(uint256,uint256)";
    let selector = keccak256(signature);
    let mut revert_data = selector[..4].to_vec();
    revert_data.extend((U256::from(5), U256::from(3)).abi_encode());

    // Runtime bytecode that copies the appended revert payload into memory and reverts with it.
    let payload_len = u8::try_from(revert_data.len()).unwrap();
    let mut runtime =
        vec![0x60, payload_len, 0x60, 0x0a, 0x5f, 0x39, 0x60, payload_len, 0x5f, 0xfd];
    runtime.extend(revert_data);

    // Isolate and seed the signature cache so decoding is deterministic and offline.
    let home = prj.root().join("home");
    let cache_dir = home.join(".foundry/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let selector = format!("0x{}", hex::encode(&selector[..4]));
    let mut errors = serde_json::Map::new();
    errors.insert(selector, json!(signature));
    fs::write(
        cache_dir.join("signatures"),
        serde_json::to_vec(&json!({
            "functions": {},
            "errors": errors,
            "events": {},
        }))
        .unwrap(),
    )
    .unwrap();

    let target = "0x000000000000000000000000000000000000dead";
    let code_override = format!("{target}:0x{}", hex::encode(runtime));
    let endpoint = handle.http_endpoint();

    cmd.env("HOME", &home);
    cmd.env("FOUNDRY_OFFLINE", "true");
    cmd.args([
        "call",
        target,
        "--data",
        "0x",
        "--override-code",
        &code_override,
        "--rpc-url",
        &endpoint,
    ])
    .assert_failure()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Error: execution reverted: RequestLimitExceeded(5, 3)

Context:
- server returned an error response:[..]

"#]]);

    cmd.cast_fuse();
    cmd.env("HOME", &home);
    cmd.env("FOUNDRY_OFFLINE", "true");
    cmd.args([
            "call",
            target,
            "--data",
            "0x",
            "--override-code",
            &code_override,
            "--rpc-url",
            &endpoint,
            "--json",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
{"schema_version":1,"success":false,"data":null,"errors":[{"level":"error","code":"cast.error","message":"execution reverted: RequestLimitExceeded(5, 3)"},{"level":"error","code":"cast.error.context","message":"server returned an error response:[..]"}],"warnings":[]}

"#]])
        .stderr_eq(str![""]);
});

// <https://github.com/foundry-rs/foundry/issues/10705>
casttest!(cast_call_return_array_of_tuples, |_prj, cmd| {
    cmd.args([
        "call",
        "0x198FC70Dfe05E755C81e54bd67Bff3F729344B9b",
        "facets() returns ((address,bytes4[])[])",
        "--rpc-url",
        "https://rpc.viction.xyz",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[[..]]

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/7541>
casttest!(cast_call_on_contract_with_no_code_prints_warning, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "call",
        "0x0000000000000000000000000000000000000000",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ])
    .assert_success()
    .stderr_eq(str![[r#"
Warning: Contract code is empty

"#]])
    .stdout_eq(str![[r#"
0x

"#]]);
});

// tests that cast call properly applies state diff override
// <https://github.com/foundry-rs/foundry/issues/10930>
casttest!(cast_call_can_override_state_diff, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "call",
        "--rpc-url",
        rpc.as_str(),
        "--data",
        "0x",
        "0x1EA77b250eF79e917A5A637D5BB82D0980653F1B",
        "--override-state-diff",
        "0x1EA77b250eF79e917A5A637D5BB82D0980653F1B:1:1",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x1337

"#]]);
    cmd.args(["--trace"]).assert_success().stdout_eq(str![[r#"
Traces:
  [7281] 0x1EA77b250eF79e917A5A637D5BB82D0980653F1B::fallback()
    ├─ [2275] 0xe537cb8a46Bd179c0C36aB7E3Fdecd759C8B80fc::fallback() [delegatecall]
    │   └─ ← [Return] 0x1337
    └─ ← [Return] 0x1337


Transaction successfully executed.
[GAS]

"#]]);
});

// Test that cast call accepts negative numbers as function arguments
casttest!(cast_call_negative_numbers, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    // Test with negative int parameter - should not treat -456789 as a flag
    cmd.args([
        "call",
        "0xAbCdEf1234567890aBcDeF1234567890aBcDeF12",
        "processValue(int128)",
        "-456789",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success();
});

// Test negative numbers with multiple parameters
casttest!(cast_call_multiple_negative_numbers, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "call",
        "--rpc-url",
        rpc.as_str(),
        "0xDeaDBeeFcAfEbAbEfAcEfEeDcBaDbEeFcAfEbAbE",
        "calculateDelta(int64,int32,uint16)",
        "-987654321",
        "-42",
        "65535",
    ])
    .assert_success();
});

// Test negative numbers mixed with flags
casttest!(cast_call_negative_with_flags, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "call",
        "--trace", // flag before
        "0x9876543210FeDcBa9876543210FeDcBa98765432",
        "updateBalance(int256)",
        "-777888",
        "--rpc-url",
        rpc.as_str(), // flag after
    ])
    .assert_success();
});

// Test that actual invalid flags are still caught
casttest!(cast_call_invalid_flag_still_caught, |_prj, cmd| {
    cmd.args([
        "call",
        "--invalid-flag", // This should be caught as invalid
        "0x5555555555555555555555555555555555555555",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
error: unexpected argument '--invalid-flag' found

  tip: to pass '--invalid-flag' as a value, use '-- --invalid-flag'

Usage: cast[..] call [OPTIONS] [TO] [SIG] [ARGS]... [COMMAND]

For more information, try '--help'.

"#]]);
});

// tests that cast call properly applies multiple state diff overrides
// <https://github.com/foundry-rs/foundry/issues/11551>
casttest!(cast_call_can_override_several_state_diff, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "call",
        "--trace",
        "--from",
        "0xf6F444fD3B0088c1375671c05A7513661beFa4e6",
        "0x5EA1d9A6dDC3A0329378a327746D71A2019eC332",
        "--rpc-url",
        rpc.as_str(),
        "--block",
        "23290753",
        "--data",
        "0xe75235b8",
        "--override-state-diff",
        "0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0xf0af0268363540b847b4c07f2f9a0401c607c1b11ebca511724a71755dfd4137:1,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:4:1,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8:0,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0xb104e0b93118902c651344349b610029d694cfdec91c589c91ebafbcd0289947:0",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
  [..] 0x5EA1d9A6dDC3A0329378a327746D71A2019eC332::getThreshold()
...

"#]]);

    cmd.cast_fuse().args([
        "call",
        "--trace",
        "--from",
        "0x2066901073a33ba2500274704aB04763875cF210",
        "0x5EA1d9A6dDC3A0329378a327746D71A2019eC332",
        "--rpc-url",
        rpc.as_str(),
        "--block",
        "23290753",
        "--data",
        "0x2f54bf6e0000000000000000000000002066901073a33ba2500274704ab04763875cf210",
        "--override-state-diff",
        "0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0xf0af0268363540b847b4c07f2f9a0401c607c1b11ebca511724a71755dfd4137:1,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:4:1,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0x4a204f620c8c5ccdca3fd54d003badd85ba500436a431f0cbda4f558c93c34c8:0,0x5EA1d9A6dDC3A0329378a327746D71A2019eC332:0xb104e0b93118902c651344349b610029d694cfdec91c589c91ebafbcd0289947:0",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
  [..] 0x5EA1d9A6dDC3A0329378a327746D71A2019eC332::isOwner(0x2066901073a33ba2500274704aB04763875cF210)
...
"#]]);
});

// tests that the --jwt-secret flag outputs a valid curl command with Authorization header
casttest!(curl_call_with_jwt, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let jwt_secret = "cabee703106087906e50f3e75a6ddbab60809f980511d1d1548d449d52220795";
    let to = "0xdead000000000000000000000000000000000000";

    let output = cmd
        .args([
            "call",
            to,
            "balanceOf(address)(uint256)",
            to,
            "--rpc-url",
            rpc,
            "--jwt-secret",
            jwt_secret,
            "--curl",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("-H 'Content-Type: application/json'"));
    assert!(output.contains("eth_call"));
    assert!(output.contains("jsonrpc"));
    assert!(output.contains(rpc));

    let jwt = output
        .split("Authorization: Bearer ")
        .nth(1)
        .expect("missing Authorization header")
        .split('\'')
        .next()
        .expect("malformed Authorization header");
    let secret = JwtSecret::from_hex(jwt_secret).unwrap();
    secret.validate(jwt).unwrap();
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

casttest!(curl_call_rejects_browser_wallet, |_prj, cmd| {
    let stderr = cmd
        .args(["call", "0xdead000000000000000000000000000000000000", "--browser", "--curl"])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(
        stderr.contains("--browser cannot be combined with --curl; use --from <ADDRESS>"),
        "unexpected stderr:\n{stderr}"
    );
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

// tests that the --curl flag outputs a valid curl command for cast call
casttest!(curl_call, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let to = "0xdead000000000000000000000000000000000000";

    let output = cmd
        .args(["call", to, "balanceOf(address)(uint256)", to, "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_call"));
    assert!(output.contains(rpc));
});

// https://github.com/foundry-rs/foundry/issues/11584
// Tests that invalid hex with uppercase 0X prefix also produces clear error
casttest!(cast_call_invalid_hex_uppercase_prefix, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Mainnet);
    cmd.args([
        "call",
        "0xdead000000000000000000000000000000000000",
        "--data",
        "0X1", // Invalid: odd length hex with uppercase prefix
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
Error: Invalid hex calldata '0X1': odd number of digits

"#]]);
});
