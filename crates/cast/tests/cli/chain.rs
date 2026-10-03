//! CLI tests for chain commands.

use super::*;

// tests that the `cast block` command works correctly
#[casttest]
fn latest_block(cmd: _) {
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
}

#[casttest]
fn block_raw(cmd: _) {
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
}

#[casttest]
fn block_json_wraps_raw_and_scalar_field_outputs(cmd: _) {
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
}

#[casttest]
fn block_raw_tempo(cmd: _) {
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
}

// tests that the `cast find-block` command works correctly
#[casttest]
fn finds_block(cmd: _) {
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
}

#[casttest]
fn balance(cmd: _) {
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
}

#[casttest]
fn block_number(cmd: _) {
    let eth_rpc_url = next_http_rpc_endpoint();
    let s = cmd
        .args(["block-number", "--rpc-url", eth_rpc_url.as_str()])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(s.trim().parse::<u64>().unwrap() > 0, "{s}")
}

#[casttest]
fn block_number_latest(cmd: _) {
    let eth_rpc_url = next_http_rpc_endpoint();
    let s = cmd
        .args(["block-number", "--rpc-url", eth_rpc_url.as_str(), "latest"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(s.trim().parse::<u64>().unwrap() > 0, "{s}")
}

#[casttest]
fn block_number_hash(cmd: _) {
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
}

// tests that the --curl flag outputs a valid curl command for cast block-number
#[casttest]
fn curl_block_number(cmd: _) {
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
}

// tests that the --curl flag outputs a valid curl command for cast chain-id
#[casttest]
fn curl_chain_id(cmd: _) {
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
}

// tests that the --curl flag outputs a valid curl command for cast gas-price
#[casttest]
fn curl_gas_price(cmd: _) {
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
}

#[casttest]
async fn chain_unknown(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    cmd.args(["chain", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq("unknown\n");
}

#[casttest]
async fn age(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test().with_genesis_timestamp(Some(1_645_099_200u64))).await;
    cmd.args(["age", "0", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq("Thu Feb 17 12:00:00 2022 UTC\n");
}

#[casttest]
async fn age_rejects_timestamp_overflow(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test().with_genesis_timestamp(Some(u64::MAX))).await;
    cmd.args(["age", "0", "--rpc-url", &handle.http_endpoint()])
        .assert_failure()
        .stderr_eq("Error: invalid timestamp\n");
}

#[casttest]
async fn base_fee(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test().with_base_fee(Some(123_456_789))).await;
    cmd.args(["base-fee", "0", "--rpc-url", &handle.http_endpoint()])
        .assert_success()
        .stdout_eq("123456789\n");
}

#[casttest]
fn cast_tx_curl_skips_network_probe(cmd: _) {
    cmd.args([
        "tx",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "--rpc-url",
        "http://127.0.0.1:1",
        "--curl",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
curl -X POST -H 'Content-Type: application/json' --data-raw '{"method":"eth_getTransactionByHash","params":["0x0000000000000000000000000000000000000000000000000000000000000001"],"id":0,"jsonrpc":"2.0"}' 'http://127.0.0.1:1/'

"#]])
    .stderr_eq(str![""]);
}

#[casttest]
fn cast_raw_block_curl_skips_network_probe(prj: _, cmd: _) {
    for raw in ["--raw", "--field=raw"] {
        cmd.cast_fuse().current_dir(prj.root())
            .args(["block", "latest"])
            .arg(raw)
            .args(["--rpc-url", "http://127.0.0.1:1", "--curl"])
            .assert_success()
            .stdout_eq(str![[r#"
curl -X POST -H 'Content-Type: application/json' --data-raw '{"method":"eth_getBlockByNumber","params":["latest",false],"id":0,"jsonrpc":"2.0"}' 'http://127.0.0.1:1/'

"#]])
            .stderr_eq(str![""]);
    }
}
