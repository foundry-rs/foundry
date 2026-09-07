//! CLI tests for logs commands.

use super::*;

casttest!(logs_topics, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "logs",
        "--rpc-url",
        rpc.as_str(),
        "--from-block",
        "12421181",
        "--to-block",
        "12421182",
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
        "0x000000000000000000000000ab5801a7d398351b8be11c439e05c5b3259aec9b",
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/cast_logs.stdout"]);
});

casttest!(logs_topic_2, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "logs",
        "--rpc-url",
        rpc.as_str(),
        "--from-block",
        "12421181",
        "--to-block",
        "12421182",
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
        "",
        "0x00000000000000000000000068a99f89e475a078645f4bac491360afe255dff1", /* Filter on the
                                                                               * `to` address */
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/cast_logs.stdout"]);
});

casttest!(logs_sig, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "logs",
        "--rpc-url",
        rpc.as_str(),
        "--from-block",
        "12421181",
        "--to-block",
        "12421182",
        "Transfer(address indexed from, address indexed to, uint256 value)",
        "0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B",
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/cast_logs.stdout"]);
});

casttest!(logs_sig_2, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    cmd.args([
        "logs",
        "--rpc-url",
        rpc.as_str(),
        "--from-block",
        "12421181",
        "--to-block",
        "12421182",
        "Transfer(address indexed from, address indexed to, uint256 value)",
        "",
        "0x68A99f89E475a078645f4BAC491360aFe255Dff1",
    ])
    .assert_success()
    .stdout_eq(file!["../fixtures/cast_logs.stdout"]);
});

// Queries a 60k-block range (which `--query-size` splits into multiple chunks) and asserts the
// chunked result is byte-for-byte identical to a single unchunked request. This proves chunking
// collects logs from every chunk without gaps, duplicates, or reordering, and that the inclusive
// `to` block is covered.
casttest!(logs_chunked, |_prj, cmd| {
    let rpc = next_http_archive_rpc_url();
    let args = [
        "logs",
        "--rpc-url",
        rpc.as_str(),
        "--from-block",
        "12400000",
        "--to-block",
        "12460000",
        "Transfer(address indexed from, address indexed to, uint256 value)",
        "0xAb5801a7D398351b8bE11C439e05C5B3259aeC9B",
    ];

    // Baseline: single request over the whole range.
    let unchunked = cmd.args(args).assert_success().get_output().stdout_lossy();

    // Same query split into 10k-block chunks (6 chunks).
    cmd.cast_fuse();
    let chunked =
        cmd.args(args).args(["--query-size", "10000"]).assert_success().get_output().stdout_lossy();

    assert_eq!(chunked, unchunked, "chunked logs must match the unchunked result");
    // Sanity check: results actually span the first and last chunk of the range.
    assert!(chunked.contains("12400314"), "missing log from the first chunk");
    assert!(chunked.contains("12454418"), "missing log from the last chunk");
});

forgetest_async!(events_quiet_preserves_output, |prj, cmd| {
    let (_api, handle) = anvil::spawn(NodeConfig::test()).await;
    let endpoint = handle.http_endpoint();
    let private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    cmd.set_current_dir(prj.root());
    prj.update_config(|config| {
        config.cbor_metadata = false;
        config.bytecode_hash = "none".parse().unwrap();
    });

    prj.add_source(
        "EventEmitter",
        r#"
contract EventEmitter {
    event Constructed(address indexed owner, uint256 value);
    event Transfer(address indexed from, address indexed to, uint256 value);

    constructor() {
        emit Constructed(msg.sender, 42);
    }

    function emitTransfer() external {
        emit Transfer(msg.sender, address(this), 42);
    }
}
"#,
    );
    cmd.forge_fuse().args(["build"]).assert_success();

    let artifact = prj.root().join("out/EventEmitter.sol/EventEmitter.json");
    let contract: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(artifact).unwrap()).unwrap();
    let bytecode = contract["bytecode"]["object"].as_str().unwrap();
    let deployment = cmd
        .cast_fuse()
        .args([
            "send",
            "--json",
            "--private-key",
            private_key,
            "--rpc-url",
            &endpoint,
            "--create",
            bytecode,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let deployment: serde_json::Value = serde_json::from_str(&deployment).unwrap();
    let address = deployment["contractAddress"].as_str().unwrap();
    let deployment_tx_hash = deployment["transactionHash"].as_str().unwrap();

    cmd.cast_fuse()
        .args([
            "--quiet",
            "events",
            deployment_tx_hash,
            "--with-local-artifacts",
            "--rpc-url",
            &endpoint,
        ])
        .assert_success()
        .stdout_eq(str![[r#"
[block 1, tx 0x[..], log 0] 0x5FbDB2315678afecb367f032d93F642f64180aa3::Constructed(address,uint256) { owner: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, value: 42 }

"#]]);

    let receipt = cmd
        .cast_fuse()
        .args([
            "send",
            "--json",
            "--private-key",
            private_key,
            "--rpc-url",
            &endpoint,
            address,
            "emitTransfer()",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let receipt: serde_json::Value = serde_json::from_str(&receipt).unwrap();
    let tx_hash = receipt["transactionHash"].as_str().unwrap();

    cmd.cast_fuse()
        .args(["--quiet", "events", tx_hash, "--rpc-url", &endpoint])
        .assert_success()
        .stdout_eq(str![[r#"
[block 2, tx 0x[..], log 0] 0x5FbDB2315678afecb367f032d93F642f64180aa3
  topic 0: 0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef
  topic 1: 0x000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb92266
  topic 2: 0x0000000000000000000000005fbdb2315678afecb367f032d93f642f64180aa3
  data: 0x000000000000000000000000000000000000000000000000000000000000002a

"#]]);

    cmd.cast_fuse()
        .args([
            "--quiet",
            "events",
            tx_hash,
            "--with-local-artifacts",
            "--rpc-url",
            &endpoint,
        ])
        .assert_success()
        .stdout_eq(str![[r#"
[block 2, tx 0x[..], log 0] 0x5FbDB2315678afecb367f032d93F642f64180aa3::Transfer(address,address,uint256) { from: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, to: 0x5FbDB2315678afecb367f032d93F642f64180aa3, value: 42 }

"#]]);

    prj.add_source(
        "AmbiguousEventEmitter",
        r#"
import {EventEmitter} from "./EventEmitter.sol";

contract AmbiguousEventEmitter is EventEmitter {
    event Ambiguous(uint256 value);
}
"#,
    );
    cmd.cast_fuse()
        .args(["--quiet", "events", tx_hash, "--with-local-artifacts", "--rpc-url", &endpoint])
        .assert_success()
        .stdout_eq(str![[r#"
[block 2, tx 0x[..], log 0] 0x5FbDB2315678afecb367f032d93F642f64180aa3
  topic 0: 0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef
  topic 1: 0x000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb92266
  topic 2: 0x0000000000000000000000005fbdb2315678afecb367f032d93f642f64180aa3
  data: 0x000000000000000000000000000000000000000000000000000000000000002a

"#]]);
});
