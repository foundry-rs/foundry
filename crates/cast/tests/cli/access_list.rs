//! CLI tests for access list commands.

use super::*;

casttest!(access_list, |_prj, cmd| {
    let rpc = next_http_rpc_endpoint();
    cmd.args([
        "access-list",
        "0xbb2b8038a1640196fbe3e38816f3e67cba72d940",
        "skim(address)",
        "0xbb2b8038a1640196fbe3e38816f3e67cba72d940",
        "--rpc-url",
        rpc.as_str(),
        "--gas-limit", // need to set this for alchemy.io to avoid "intrinsic gas too low" error
        "100000",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[GAS]
access list:
- address: [..]
  keys:
...
- address: [..]
  keys:
...
- address: [..]
  keys:
...

"#]]);
});

casttest!(access_list_eip7702_auth_disclosure_declined, |prj, cmd| {
    prj.update_config(|config| config.chain = Some(31337.into()));

    cmd.args([
        "access-list",
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
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

casttest!(access_list_eip7702_auth_disclosure_requires_signer, |prj, cmd| {
    prj.update_config(|config| config.chain = Some(31337.into()));

    cmd.args([
        "access-list",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--from",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--nonce",
        "0",
        "--rpc-url",
        "http://127.0.0.1:1",
    ])
    .assert_failure()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Error: No signer available to sign authorization. Provide a pre-signed authorization (hex-encoded) instead.

"#]]);
});

casttest!(access_list_eip7702_auth_disclosure_accepted_and_forced, async |_prj, cmd| {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;
    let endpoint = handle.http_endpoint();
    let args = [
        "access-list",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
    ];

    cmd.args(args)
        .stdin("y\n")
        .assert_success()
        .stdout_eq(str![[r#"
[GAS]
access list:

"#]])
        .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] "#]]);

    cmd.cast_fuse()
        .args(args)
        .arg("--force")
        .assert_success()
        .stdout_eq(str![[r#"
[GAS]
access list:

"#]])
        .stderr_eq(str![""]);
});

// Test cast access-list with negative numbers
casttest!(cast_access_list_negative_numbers, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "access-list",
        "0x9999999999999999999999999999999999999999",
        "adjustPosition(int128)",
        "-33333",
        "--gas-limit",
        "1000000",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success();
});
