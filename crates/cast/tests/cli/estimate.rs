//! CLI tests for estimate commands.

use super::*;

// tests that `cast estimate` is working correctly.
casttest!(estimate_function_gas, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // ensure we get a positive non-error value for gas estimate
    let output: u32 = cmd
        .args([
            "estimate",
            "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045", // vitalik.eth
            "--value",
            "100",
            "deposit()",
            "--rpc-url",
            eth_rpc_url.as_str(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .parse()
        .unwrap();
    assert!(output.ge(&0));
});

// tests that `cast estimate --cost` is working correctly.
casttest!(estimate_function_cost, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // ensure we get a positive non-error value for cost estimate
    let output: f64 = cmd
        .args([
            "estimate",
            "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045", // vitalik.eth
            "--value",
            "100",
            "deposit()",
            "--rpc-url",
            eth_rpc_url.as_str(),
            "--cost",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .parse()
        .unwrap();
    assert!(output > 0f64);
});

// tests that `cast estimate --create` is working correctly.
casttest!(estimate_contract_deploy_gas, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    // sample contract code bytecode. Wouldn't run but is valid bytecode that the estimate method
    // accepts and could be deployed.
    let output = cmd
        .args([
            "estimate",
            "--rpc-url",
            eth_rpc_url.as_str(),
            "--create",
            "0000",
            "ERC20(uint256,string,string)",
            "100",
            "Test",
            "TST",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // ensure we get a positive non-error value for gas estimate
    let output: u32 = output.trim().parse().unwrap();
    assert!(output > 0);
});

casttest!(estimate_eip7702_auth_disclosure_declined, |prj, cmd| {
    prj.update_config(|config| config.chain = Some(31337.into()));

    cmd.args([
        "estimate",
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

casttest!(estimate_eip7702_auth_disclosure_requires_signer, |prj, cmd| {
    prj.update_config(|config| config.chain = Some(31337.into()));

    cmd.args([
        "estimate",
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

casttest!(estimate_eip7702_auth_disclosure_accepted_and_forced, async |_prj, cmd| {
    let (api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;
    let endpoint = handle.http_endpoint();
    api.anvil_set_code(
        address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8"),
        "0x602a5f5260205ff3".parse().unwrap(),
    )
    .await
    .unwrap();
    let args = [
        "estimate",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
        "--auth",
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--rpc-url",
        &endpoint,
    ];

    let output = cmd
        .args(args)
        .arg("--json")
        .stdin("y\n")
        .assert_success()
        .stderr_eq(str![[r#"
Warning: This command will send a signed EIP-7702 authorization to the RPC endpoint. The authorization can be submitted on-chain by anyone once its nonce is valid.

Continue anyway? [y/N] "#]])
        .get_output()
        .stdout_lossy();
    let output: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert!(output["data"].as_u64().unwrap() > 21_000);

    let output = cmd
        .cast_fuse()
        .args(args)
        .arg("--force")
        .assert_success()
        .stderr_eq(str![""])
        .get_output()
        .stdout_lossy();
    assert!(output.trim().parse::<u64>().unwrap() > 21_000);

    let output = cmd
        .cast_fuse()
        .args(args)
        .args(["--quiet", "--force"])
        .assert_success()
        .stderr_eq(str![""])
        .get_output()
        .stdout_lossy();
    assert!(output.trim().parse::<u64>().unwrap() > 21_000);
});

// <https://basescan.org/block/30558838>
casttest!(
    #[ignore = "public Base RPC endpoint used in CI does not reliably serve this block"]
    flaky_estimate_base_da,
    |_prj, cmd| {
        cmd.args(["da-estimate", "30558838", "-r", "https://mainnet.base.org/"])
            .assert_success()
            .stdout_eq(str![[r#"
52916546100

"#]])
            .stderr_eq(str![[r#"
Estimated data availability size for block 30558838 with 225 transactions:

"#]]);
    }
);

// Test that cast estimate --create works correctly with constructor arguments
// <https://github.com/foundry-rs/foundry/issues/10947>
casttest!(cast_estimate_create_with_constructor_args, |prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Add a simple contract with constructor arguments
    prj.add_source(
        "EstimateContract",
        r#"
contract EstimateContract {
    uint256 public value;
    string public name;

    constructor(uint256 _value, string memory _name) {
        value = _value;
        name = _name;
    }
}
"#,
    );

    // Compile to get bytecode
    cmd.forge_fuse().args(["build"]).assert_success();

    // Get the compiled bytecode
    let bytecode_path = prj.root().join("out/EstimateContract.sol/EstimateContract.json");
    let contract_json = std::fs::read_to_string(bytecode_path).unwrap();
    let contract_data: serde_json::Value = serde_json::from_str(&contract_json).unwrap();
    let bytecode = contract_data["bytecode"]["object"].as_str().unwrap();

    let output = cmd
        .cast_fuse()
        .args([
            "estimate",
            "--rpc-url",
            eth_rpc_url.as_str(),
            "--create",
            bytecode,
            "constructor(uint256,string)",
            "100",
            "TestContract",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Parse the gas estimate
    let gas_estimate = output.trim().parse::<u64>().expect("Failed to parse gas estimate");

    // Gas estimate should be positive and reasonable for contract deployment
    assert!(gas_estimate > 50000, "Gas estimate too low for contract deployment");
    assert!(gas_estimate < 5000000, "Gas estimate unreasonably high");
});

// Tests for negative number argument parsing
// Ensures that negative numbers in function arguments are properly parsed
// instead of being treated as command flags

// Test cast estimate with negative numbers
casttest!(cast_estimate_negative_numbers, |_prj, cmd| {
    let rpc = next_rpc_endpoint(NamedChain::Sepolia);
    cmd.args([
        "estimate",
        "0xBbBbBbBbBbBbBbBbBbBbBbBbBbBbBbBbBbBbBbBb",
        "rebalance(int64)",
        "-8888",
        "--rpc-url",
        rpc.as_str(),
    ])
    .assert_success();
});
