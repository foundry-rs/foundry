//! CLI tests for rpc commands.

use super::*;

// test for cast_rpc without arguments
casttest!(rpc_no_args, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast rpc eth_chainId`
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_chainId"]).assert_success().stdout_eq(
        str![[r#"
"0x1"

"#]],
    );
});

// test for cast_rpc without arguments using websocket
casttest!(ws_rpc_no_args, |_prj, cmd| {
    let eth_rpc_url = next_ws_rpc_endpoint();

    // Call `cast rpc eth_chainId`
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_chainId"]).assert_success().stdout_eq(
        str![[r#"
"0x1"

"#]],
    );
});

// test for cast_rpc with arguments
casttest!(rpc_with_args, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast rpc eth_getBlockByNumber 0x123 false`
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_getBlockByNumber", "0x123", "false"])
    .assert_json_stdout(str![[r#"
{"number":"0x123","hash":"0xc5dab4e189004a1312e9db43a40abb2de91ad7dd25e75880bf36016d8e9df524","transactions":[],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","extraData":"0x476574682f4c5649562f76312e302e302f6c696e75782f676f312e342e32","nonce":"0x29d6547c196e00e0","miner":"0xbb7b8287f3f0a933474a79eae42cbca977791171","difficulty":"0x494433b31","gasLimit":"0x1388","gasUsed":"0x0","uncles":[],"sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","size":"0x220","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","stateRoot":"0x3fe6bd17aa85376c7d566df97d9f2e536f37f7a87abb3a6f9e2891cf9442f2e4","mixHash":"0x943056aa305aa6d22a3c06110942980342d1f4d4b11c17711961436a0f963ea0","parentHash":"0x7abfd11e862ccde76d6ea8ee20978aac26f4bcb55de1188cc0335be13e817017","timestamp":"0x55ba4564"}

"#]]);
});

// test for cast_rpc with arguments
casttest!(rpc_format_as_json, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast rpc eth_getBlockByNumber 0x123 false`
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_getBlockByNumber", "0x123", "false", "--json"])
    .assert_json_stdout(str![[r#"
{
  "schema_version": 1,
  "success": true,
  "data": {
    "hash": "0xc5dab4e189004a1312e9db43a40abb2de91ad7dd25e75880bf36016d8e9df524",
    "parentHash": "0x7abfd11e862ccde76d6ea8ee20978aac26f4bcb55de1188cc0335be13e817017",
    "sha3Uncles": "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
    "miner": "0xbb7b8287f3f0a933474a79eae42cbca977791171",
    "stateRoot": "0x3fe6bd17aa85376c7d566df97d9f2e536f37f7a87abb3a6f9e2891cf9442f2e4",
    "transactionsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
    "receiptsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421",
    "logsBloom": "0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    "difficulty": "0x494433b31",
    "number": "0x123",
    "gasLimit": "0x1388",
    "gasUsed": "0x0",
    "timestamp": "0x55ba4564",
    "extraData": "0x476574682f4c5649562f76312e302e302f6c696e75782f676f312e342e32",
    "mixHash": "0x943056aa305aa6d22a3c06110942980342d1f4d4b11c17711961436a0f963ea0",
    "nonce": "0x29d6547c196e00e0",
    "size": "0x220",
    "uncles": [],
    "transactions": []
  },
  "errors": [],
  "warnings": []
}

"#]]);
});

// test for cast_rpc with raw params
casttest!(rpc_raw_params, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast rpc eth_getBlockByNumber --raw '["0x123", false]'`
    cmd.args([
        "rpc",
        "--rpc-url",
        eth_rpc_url.as_str(),
        "eth_getBlockByNumber",
        "--raw",
        r#"["0x123", false]"#,
    ])
    .assert_json_stdout(str![[r#"
{"number":"0x123","hash":"0xc5dab4e189004a1312e9db43a40abb2de91ad7dd25e75880bf36016d8e9df524","transactions":[],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","extraData":"0x476574682f4c5649562f76312e302e302f6c696e75782f676f312e342e32","nonce":"0x29d6547c196e00e0","miner":"0xbb7b8287f3f0a933474a79eae42cbca977791171","difficulty":"0x494433b31","gasLimit":"0x1388","gasUsed":"0x0","uncles":[],"sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","size":"0x220","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","stateRoot":"0x3fe6bd17aa85376c7d566df97d9f2e536f37f7a87abb3a6f9e2891cf9442f2e4","mixHash":"0x943056aa305aa6d22a3c06110942980342d1f4d4b11c17711961436a0f963ea0","parentHash":"0x7abfd11e862ccde76d6ea8ee20978aac26f4bcb55de1188cc0335be13e817017","timestamp":"0x55ba4564"}

"#]]);
});

// test for cast_rpc with direct params
casttest!(rpc_raw_params_stdin, |_prj, cmd| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `echo "\n[\n\"0x123\",\nfalse\n]\n" | cast rpc eth_getBlockByNumber --raw
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_getBlockByNumber", "--raw"]).stdin(
        b"\n[\n\"0x123\",\nfalse\n]\n"
    )
    .assert_json_stdout(str![[r#"
{"number":"0x123","hash":"0xc5dab4e189004a1312e9db43a40abb2de91ad7dd25e75880bf36016d8e9df524","transactions":[],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","receiptsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","extraData":"0x476574682f4c5649562f76312e302e302f6c696e75782f676f312e342e32","nonce":"0x29d6547c196e00e0","miner":"0xbb7b8287f3f0a933474a79eae42cbca977791171","difficulty":"0x494433b31","gasLimit":"0x1388","gasUsed":"0x0","uncles":[],"sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","size":"0x220","transactionsRoot":"0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421","stateRoot":"0x3fe6bd17aa85376c7d566df97d9f2e536f37f7a87abb3a6f9e2891cf9442f2e4","mixHash":"0x943056aa305aa6d22a3c06110942980342d1f4d4b11c17711961436a0f963ea0","parentHash":"0x7abfd11e862ccde76d6ea8ee20978aac26f4bcb55de1188cc0335be13e817017","timestamp":"0x55ba4564"}

"#]]);
});

// tests that the --curl flag outputs a valid curl command for cast rpc
casttest!(curl_rpc, |_prj, cmd| {
    let rpc = "https://eth.example.com";

    let output = cmd
        .args(["rpc", "eth_blockNumber", "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("-H 'Content-Type: application/json'"));
    assert!(output.contains("eth_blockNumber"));
    assert!(output.contains("jsonrpc"));
    assert!(output.contains(rpc));
});

// tests that the --jwt-secret flag outputs a valid curl command with Authorization header
casttest!(curl_rpc_with_jwt, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let jwt_secret = "cabee703106087906e50f3e75a6ddbab60809f980511d1d1548d449d52220795";

    let output = cmd
        .args(["rpc", "eth_blockNumber", "--rpc-url", rpc, "--jwt-secret", jwt_secret, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("-H 'Content-Type: application/json'"));
    assert!(output.contains("eth_blockNumber"));
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
