//! CLI tests for mktx commands.

use super::*;

casttest!(mktx, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--chain",
        "1",
        "--nonce",
        "0",
        "--value",
        "100",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
    ]).assert_success().stdout_eq(str![[r#"
0x02f86b0180843b9aca008502540be4008252089400000000000000000000000000000000000000016480c001a070d55e79ed3ac9fc8f51e78eb91fd054720d943d66633f2eb1bc960f0126b0eca052eda05a792680de3181e49bab4093541f75b49d1ecbe443077b3660c836016a

"#]]);
});

casttest!(mktx_eip7702_auth_disclosure_declined, |_prj, cmd| {
    cmd.args([
        "mktx",
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

casttest!(mktx_ethsign_eip7702_auth_disclosure_declined, |_prj, cmd| {
    cmd.args([
        "mktx",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        PRESIGNED_EIP7702_AUTH,
        "--ethsign",
        "--from",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--chain",
        "31337",
        "--nonce",
        "0",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
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

casttest!(mktx_eip7702_auth_no_disclosure, |_prj, cmd| {
    cmd.args([
        "mktx",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--chain",
        "31337",
        "--nonce",
        "0",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "--rpc-url",
        "http://127.0.0.1:1",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x[..]

"#]])
    .stderr_eq(str![""]);
});

casttest!(mktx_eip7702_auth_disclosure_forced, async |_prj, cmd| {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;

    cmd.args([
        "mktx",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--force",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x[..]

"#]])
    .stderr_eq(str![""]);
});

casttest!(mktx_sponsor_hash_supports_address_auth, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test_tempo()).await;

    cmd.args([
        "mktx",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--force",
        "--tempo.print-sponsor-hash",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x[..]

"#]])
    .stderr_eq(str![""]);
});

casttest!(mktx_signature, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--signature",
        "0x70d55e79ed3ac9fc8f51e78eb91fd054720d943d66633f2eb1bc960f0126b0ec52eda05a792680de3181e49bab4093541f75b49d1ecbe443077b3660c836016a01",
        "--from",
        "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
        "--chain",
        "1",
        "--nonce",
        "0",
        "--value",
        "100",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x02f86b0180843b9aca008502540be4008252089400000000000000000000000000000000000000016480c001a070d55e79ed3ac9fc8f51e78eb91fd054720d943d66633f2eb1bc960f0126b0eca052eda05a792680de3181e49bab4093541f75b49d1ecbe443077b3660c836016a

"#]]);
});

casttest!(mktx_signature_requires_from, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--signature",
        "0x70d55e79ed3ac9fc8f51e78eb91fd054720d943d66633f2eb1bc960f0126b0ec52eda05a792680de3181e49bab4093541f75b49d1ecbe443077b3660c836016a01",
        "0x0000000000000000000000000000000000000001",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
error: the following required arguments were not provided:
  --from <ADDRESS>

Usage: cast[..] mktx --from <ADDRESS> --signature <SIGNATURE> <TO> [SIG] [ARGS]...

For more information, try '--help'.

"#]]);
});

casttest!(mktx_signature_normalizes_high_s, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--signature",
        "0x70d55e79ed3ac9fc8f51e78eb91fd054720d943d66633f2eb1bc960f0126b0ecad125fa586d97f21ce7e1b6454bf6caa9b392849907cbbf8b857282c08003fd700",
        "--from",
        "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
        "--chain",
        "1",
        "--nonce",
        "0",
        "--value",
        "100",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
0x02f86b0180843b9aca008502540be4008252089400000000000000000000000000000000000000016480c001a070d55e79ed3ac9fc8f51e78eb91fd054720d943d66633f2eb1bc960f0126b0eca052eda05a792680de3181e49bab4093541f75b49d1ecbe443077b3660c836016a

"#]]);
});

casttest!(mktx_signature_from_mismatch, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--signature",
        "0x70d55e79ed3ac9fc8f51e78eb91fd054720d943d66633f2eb1bc960f0126b0ec52eda05a792680de3181e49bab4093541f75b49d1ecbe443077b3660c836016a01",
        "--from",
        "0x0000000000000000000000000000000000000001",
        "--chain",
        "1",
        "--nonce",
        "0",
        "--value",
        "100",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
    ])
    .assert_failure()
    .stderr_eq(str![[r#"
Error: The provided signature recovers to 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf, which does not match the specified sender 0x0000000000000000000000000000000000000001

"#]]);
});

// ensure recipient or code is required
casttest!(mktx_requires_to, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--chain",
        "1",
    ]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: Must specify a recipient address or contract code to deploy

"#]]);
});

casttest!(mktx_signer_from_mismatch, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--from",
        "0x0000000000000000000000000000000000000001",
        "--chain",
        "1",
        "0x0000000000000000000000000000000000000001",
    ]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: The specified sender via CLI/env vars does not match the sender configured via
the hardware wallet's HD Path.
Please use the `--hd-path <PATH>` parameter to specify the BIP32 Path which
corresponds to the sender, or let foundry automatically detect it by not specifying any sender address.

"#]]);
});

casttest!(mktx_signer_from_match, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--from",
        "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
        "--chain",
        "1",
        "--nonce",
        "0",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
    ]).assert_success().stdout_eq(str![[r#"
0x02f86b0180843b9aca008502540be4008252089400000000000000000000000000000000000000018080c001a0cce9a61187b5d18a89ecd27ec675e3b3f10d37f165627ef89a15a7fe76395ce8a07537f5bffb358ffbef22cda84b1c92f7211723f9e09ae037e81686805d3e5505

"#]]);
});

casttest!(mktx_raw_unsigned, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--from",
        "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
        "--chain",
        "1",
        "--nonce",
        "0",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
        "--raw-unsigned",
    ])
    .assert_success()
    .stdout_eq(str![[
        r#"0x02e80180843b9aca008502540be4008252089400000000000000000000000000000000000000018080c0

"#
    ]]);
});

casttest!(mktx_raw_unsigned_curl_skips_unknown_fee_token_symbol_lookup, |_prj, cmd| {
    let output = cmd
        .args([
            "mktx",
            "0x0000000000000000000000000000000000001234",
            "--chain",
            "tempo",
            "--rpc-url",
            "https://example.invalid",
            "--curl",
            "--tempo.fee-token",
            "0x20C00000000000000000000014f22CA97301EB73",
            "--from",
            "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
            "--nonce",
            "0",
            "--gas-limit",
            "100000",
            "--gas-price",
            "1000000000",
            "--priority-gas-price",
            "1000000000",
            "--value",
            "0",
            "--raw-unsigned",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(output.starts_with("0x"), "expected raw transaction hex, got:\n{output}");
    assert!(!output.contains("eth_call"), "unexpected fee-token symbol lookup curl:\n{output}");
    assert!(!output.contains("0x95d89b41"), "unexpected symbol() calldata:\n{output}");
});

casttest!(mktx_raw_unsigned_no_from_missing_chain, async |_prj, cmd| {
    // As chain is not provided, a query is made to the provider to get the chain id, before the
    // tx is built. Anvil is configured to use chain id 1 so that the produced tx will
    // be the same as in the `mktx_raw_unsigned` test.
    let (_, handle) = anvil::spawn(NodeConfig::test().with_chain_id(Some(1u64))).await;
    cmd.args([
        "mktx",
        "--nonce",
        "0",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
        "--raw-unsigned",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[
        r#"0x02e80180843b9aca008502540be4008252089400000000000000000000000000000000000000018080c0

"#
    ]]);
});

casttest!(mktx_raw_unsigned_no_from_missing_gas_pricing, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args([
        "mktx",
        "--nonce",
        "0",
        "0x0000000000000000000000000000000000000001",
        "--raw-unsigned",
        "--rpc-url",
        &handle.http_endpoint(),
    ])
    .assert_success()
    .stdout_eq(str![[
        r#"0x02e5827a69800184773594018252089400000000000000000000000000000000000000018080c0

"#
    ]]);
});

casttest!(mktx_raw_unsigned_no_from_missing_nonce, |_prj, cmd| {
    cmd.args([
        "mktx",
        "--chain",
        "1",
        "--gas-limit",
        "21000",
        "--gas-price",
        "20000000000",
        "0x742d35Cc6634C0532925a3b8D6Ac6F67C9c2b7FD",
        "--raw-unsigned",
    ])
    .assert_failure()
    .stderr_eq(str![[
        r#"Error: Missing required parameters for raw unsigned transaction. When --from is not provided, you must specify: --nonce

"#
    ]]);
});

casttest!(mktx_ethsign, async |_prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    cmd.args([
        "mktx",
        "--from",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--chain",
        "31337",
        "--nonce",
        "0",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "0x0000000000000000000000000000000000000001",
        "--ethsign",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success()
    .stdout_eq(str![[
        r#"
0x02f86d827a6980843b9aca008502540be4008252089400000000000000000000000000000000000000018080c001a0b8eeb1ded87b085859c510c5692bed231e3ee8b068ccf71142bbf28da0e95987a07813b676a248ae8055f28495021d78dee6695479d339a6ad9d260d9eaf20674c

"#
    ]]);
});

// tests that `cast mktx --tempo.lane <name>` resolves the lane against a `tempo.lanes.toml` file at
// the project root, sets the corresponding `nonce_key` on the produced Tempo AA transaction.
casttest!(mktx_tempo_lane_resolves_nonce_key, |prj, cmd| {
    // Write a shared lanes file at the project root.
    let lanes_path = prj.root().join("tempo.lanes.toml");
    fs::write(&lanes_path, "deploy = 1\nops = 2\npayments = 42\n").unwrap();

    let output = cmd
        .current_dir(prj.root())
        .args([
            "mktx",
            "--tempo.lane",
            "payments",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--chain",
            "1",
            "--nonce",
            "0",
            "--gas-limit",
            "21000",
            "--gas-price",
            "10000000000",
            "--priority-gas-price",
            "1000000000",
            "0x0000000000000000000000000000000000000001",
        ])
        .assert_success()
        .get_output()
        .clone();

    // The resolved-lane breadcrumb is printed to stderr so it doesn't pollute stdout
    // (which carries the raw signed transaction).
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("lane: payments (nonce_key=42, nonce=0)"),
        "expected lane breadcrumb on stderr, got: {stderr}",
    );

    // Decode the produced signed Tempo AA transaction and verify it carries the
    // resolved 2D nonce key.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw_hex = stdout.trim().trim_start_matches("0x");
    let raw = hex::decode(raw_hex).expect("decode hex output");
    let envelope = TempoTxEnvelope::decode_2718(&mut raw.as_slice()).expect("decode tempo tx");
    assert!(envelope.is_aa(), "expected Tempo AA transaction, got: {envelope:?}");
    assert_eq!(envelope.nonce_key(), Some(U256::from(42_u64)));
});

casttest!(batch_mktx_eip7702_auth_disclosure, async |_prj, cmd| {
    let args = [
        "batch-mktx",
        "--call",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    ];

    cmd.args(args)
        .args(["--chain", "31337", "--rpc-url", "http://127.0.0.1:1"])
        .stdin("n\n")
        .assert_success()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Building batch transaction with 1 call(s)...
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] Aborted.

"#]]);

    let (_api, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    cmd.cast_fuse()
        .args(args)
        .args(["--force", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq(str![[r#"
0x[..]

"#]])
        .stderr_eq(str![[r#"
Building batch transaction with 1 call(s)...

"#]]);
});

casttest!(batch_mktx_ethsign_eip7702_auth_disclosure_declined, |_prj, cmd| {
    cmd.args([
        "batch-mktx",
        "--call",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        PRESIGNED_EIP7702_AUTH,
        "--ethsign",
        "--from",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--chain",
        "31337",
        "--nonce",
        "0",
        "--gas-limit",
        "21000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "--rpc-url",
        "http://127.0.0.1:1",
    ])
    .stdin("n\n")
    .assert_success()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Building batch transaction with 1 call(s)...
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] Aborted.

"#]]);
});

// Test cast mktx with negative numbers
casttest!(cast_mktx_negative_numbers, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "mktx",
        "0x1111111111111111111111111111111111111111",
        "settleDebt(int256)",
        "-15000",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80", // anvil wallet #0
        "--rpc-url",
        rpc.as_str(),
        "--gas-limit",
        "100000",
    ])
    .assert_success();
});

// Test cast mktx with EIP-4844 blob transaction (legacy format)
casttest!(cast_mktx_eip4844_blob, |prj, cmd| {
    // Create a temporary blob data file
    let blob_data = b"dummy blob data for testing";
    let blob_path = prj.root().join("blob_data.bin");
    fs::write(&blob_path, blob_data).unwrap();

    cmd.args([
        "mktx",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--chain",
        "1",
        "--nonce",
        "0",
        "--gas-limit",
        "100000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "--blob",
        "--eip4844",
        "--blob-gas-price",
        "1000000",
        "--path",
        blob_path.to_str().unwrap(),
        "0x0000000000000000000000000000000000000001",
    ])
    .assert_success();
});

// Test cast mktx with EIP-7594 blob transaction (default format)
casttest!(cast_mktx_eip7594_blob, |prj, cmd| {
    // Create a temporary blob data file
    let blob_data = b"dummy peerdas blob data for testing";
    let blob_path = prj.root().join("peerdas_blob_data.bin");
    fs::write(&blob_path, blob_data).unwrap();

    cmd.args([
        "mktx",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--chain",
        "1",
        "--nonce",
        "0",
        "--gas-limit",
        "100000",
        "--gas-price",
        "10000000000",
        "--priority-gas-price",
        "1000000000",
        "--blob",
        "--blob-gas-price",
        "1000000",
        "--path",
        blob_path.to_str().unwrap(),
        "0x0000000000000000000000000000000000000001",
    ])
    .assert_success();
});

casttest!(mktx_tempo_access_key_uses_alloy_wallet, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let output = cmd
        .args([
            "mktx",
            "0x0000000000000000000000000000000000000001",
            "--rpc-url",
            &handle.http_endpoint(),
            "--chain",
            "31337",
            "--nonce",
            "0",
            "--gas-limit",
            "100000",
            "--gas-price",
            "20000000000",
            "--priority-gas-price",
            "1000000000",
            "--tempo.fee-token",
            "0x20C0000000000000000000000000000000000000",
            "--tempo.access-key",
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
            "--tempo.root-account",
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        ])
        .assert_success()
        .get_output()
        .clone();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw = hex::decode(stdout.trim().trim_start_matches("0x")).expect("decode raw transaction");
    let TempoTxEnvelope::AA(signed) =
        TempoTxEnvelope::decode_2718(&mut raw.as_slice()).expect("decode Tempo AA transaction")
    else {
        panic!("expected a Tempo AA transaction");
    };
    let TempoSignature::Keychain(signature) = signed.signature() else {
        panic!("expected an account access-key signature");
    };
    assert_eq!(signature.user_address, address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
    assert_eq!(signature.version, KeychainVersion::V2);
    assert_eq!(
        signature.key_id(&signed.tx().signature_hash()).unwrap(),
        address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8")
    );
});
