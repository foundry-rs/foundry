//! CLI tests for chain commands.

use super::*;

// tests that the `cast block` command works correctly
casttest!(latest_block, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast find-block`
    cmd.args(["block", "latest", "--rpc-url", eth_rpc_url.as_str()]);
    cmd.assert_success().stdout_eq(str![[r#"


baseFeePerGas        [..]
difficulty           [..]
extraData            [..]
gasLimit             [..]
gasUsed              [..]
hash                 [..]
logsBloom            [..]
miner                [..]
mixHash              [..]
nonce                [..]
number               [..]
parentHash           [..]
parentBeaconRoot     [..]
transactionsRoot     [..]
receiptsRoot         [..]
sha3Uncles           [..]
size                 [..]
stateRoot            [..]
timestamp            [..]
withdrawalsRoot      [..]
totalDifficulty      [..]
blobGasUsed          [..]
excessBlobGas        [..]
requestsHash         [..]
transactions:        [
...
]

"#]]);

    // <https://etherscan.io/block/15007840>
    cmd.cast_fuse().args([
        "block",
        "15007840",
        "-f",
        "hash,timestamp",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ]);
    cmd.assert_success().stdout_eq(str![[r#"
0x950091817a57e22b6c1f3b951a15f52d41ac89b299cc8f9c89bb6d185f80c415
1655904485

"#]]);
});

casttest!(block_raw, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    let output = cmd
        .args(["block", "22934900", "--rpc-url", eth_rpc_url.as_str(), "--raw"])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    // Hash the output with keccak256
    let hash = alloy_primitives::keccak256(hex::decode(output).unwrap());

    // Verify the Mainnet's block #22934900 header hash equals the expected value
    // obtained with go-ethereum's `block.Header().Hash()` method
    assert_eq!(
        hash.to_string(),
        "0x49fd7f3b9ba5d67fa60197027f09454d4cac945e8f271edcc84c3fd5872446d3"
    );
});

casttest!(block_json_wraps_raw_and_scalar_field_outputs, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    let raw_output = cmd
        .args(["block", "22934900", "--rpc-url", eth_rpc_url.as_str(), "--raw", "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let raw_envelope: serde_json::Value = serde_json::from_str(raw_output.trim()).unwrap();
    assert_eq!(raw_envelope["schema_version"], 1);
    assert!(raw_envelope["success"].as_bool().unwrap());
    assert!(raw_envelope["data"].as_str().unwrap().starts_with("0x"));

    let field_output = cmd
        .cast_fuse()
        .args(["block", "0x123", "--field", "number", "--rpc-url", eth_rpc_url.as_str(), "--json"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let field_envelope: serde_json::Value = serde_json::from_str(field_output.trim()).unwrap();
    assert_eq!(field_envelope["schema_version"], 1);
    assert!(field_envelope["success"].as_bool().unwrap());
    assert_eq!(field_envelope["data"], 291);
});

casttest!(block_raw_tempo, |_prj, cmd| {
    // https://explore.tempo.xyz/block/8386710
    let output = cmd
        .args([
            "block",
            "8386710",
            "--rpc-url",
            "https://rpc.moderato.tempo.xyz",
            "--raw",
            "-n",
            "tempo",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    let hash = alloy_primitives::keccak256(hex::decode(output).unwrap());

    assert_eq!(
        hash.to_string(),
        "0xcd6170dc28b888bcb93ed1ad76a6bea4ad9977b678db5d462df83d35ec9b8d15"
    );
});

// tests that the `cast find-block` command works correctly
casttest!(finds_block, |_prj, cmd| {
    // Construct args
    let timestamp = "1647843609".to_string();
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast find-block`
    // <https://etherscan.io/block/14428082>
    cmd.args(["find-block", "--rpc-url", eth_rpc_url.as_str(), &timestamp])
        .assert_success()
        .stdout_eq(str![[r#"
14428082

"#]]);
});

casttest!(balance, |_prj, cmd| {
    let rpc = next_http_rpc_endpoint();
    let dai = "0x6B175474E89094C44Da98b954EedeAC495271d0F";

    let dai_result = cmd
        .args([
            "balance",
            "0x0000000000000000000000000000000000000000",
            "--erc20",
            dai,
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    let alias_result = cmd
        .cast_fuse()
        .args([
            "balance",
            "0x0000000000000000000000000000000000000000",
            "--erc721",
            dai,
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    assert_ne!(dai_result, "0");
    assert_eq!(alias_result, dai_result);
});

casttest!(block_number, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    let s = cmd
        .args(["block-number", "--rpc-url", eth_rpc_url.as_str()])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(s.trim().parse::<u64>().unwrap() > 0, "{s}")
});

casttest!(block_number_latest, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    let s = cmd
        .args(["block-number", "--rpc-url", eth_rpc_url.as_str(), "latest"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(s.trim().parse::<u64>().unwrap() > 0, "{s}")
});

casttest!(block_number_hash, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();
    let s = cmd
        .args([
            "block-number",
            "--rpc-url",
            eth_rpc_url.as_str(),
            "0x88e96d4537bea4d9c05d12549907b32561d3bf31f45aae734cdc119f13406cb6",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(s.trim().parse::<u64>().unwrap(), 1, "{s}")
});

// tests that the --curl flag outputs a valid curl command for cast block-number
casttest!(curl_block_number, |_prj, cmd| {
    let rpc = "https://eth.example.com";

    let output = cmd
        .args(["block-number", "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_blockNumber"));
    assert!(output.contains(rpc));
});

// tests that the --curl flag outputs a valid curl command for cast chain-id
casttest!(curl_chain_id, |_prj, cmd| {
    let rpc = "https://eth.example.com";

    let output = cmd
        .args(["chain-id", "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_chainId"));
    assert!(output.contains(rpc));
});

// tests that the --curl flag outputs a valid curl command for cast gas-price
casttest!(curl_gas_price, |_prj, cmd| {
    let rpc = "https://eth.example.com";

    let output = cmd
        .args(["gas-price", "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_gasPrice"));
    assert!(output.contains(rpc));
});

casttest!(chain_unknown, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args(["chain", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq("unknown\n");
});

casttest!(age, async |_prj, cmd| {
    let (_, handle) =
        anvil::spawn(NodeConfig::test().with_genesis_timestamp(Some(1_645_099_200u64))).await;
    cmd.args(["age", "0", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq("Thu Feb 17 12:00:00 2022 UTC\n");
});

casttest!(age_rejects_timestamp_overflow, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test().with_genesis_timestamp(Some(u64::MAX))).await;
    cmd.args(["age", "0", "--rpc-url", &handle.http_endpoint()])
        .assert_failure()
        .stderr_eq("Error: invalid timestamp\n");
});

casttest!(base_fee, async |_prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test().with_base_fee(Some(123_456_789))).await;
    cmd.args(["base-fee", "0", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq("123456789\n");
});
