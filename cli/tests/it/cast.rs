//! Contains various tests for checking cast commands

use foundry_cli_test_utils::{
    casttest,
    util::{TestCommand, TestProject},
};
use foundry_utils::rpc::next_http_rpc_endpoint;
use std::{io::Write, path::PathBuf};

// tests that the `cast block` command works correctly
casttest!(latest_block, |_: TestProject, mut cmd: TestCommand| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast find-block`
    cmd.args(["block", "latest", "--rpc-url", eth_rpc_url.as_str()]);
    let output = cmd.stdout_lossy();
    assert!(output.contains("transactions:"));
    assert!(output.contains("gasUsed"));

    // <https://etherscan.io/block/15007840>
    cmd.cast_fuse().args(["block", "15007840", "hash", "--rpc-url", eth_rpc_url.as_str()]);
    let output = cmd.stdout_lossy();
    assert_eq!(output.trim(), "0x950091817a57e22b6c1f3b951a15f52d41ac89b299cc8f9c89bb6d185f80c415")
});

// tests that the `cast find-block` command works correctly
casttest!(finds_block, |_: TestProject, mut cmd: TestCommand| {
    // Construct args
    let timestamp = "1647843609".to_string();
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast find-block`
    cmd.args(["find-block", "--rpc-url", eth_rpc_url.as_str(), &timestamp]);
    let output = cmd.stdout_lossy();
    println!("{output}");

    // Expect successful block query
    // Query: 1647843609, Mar 21 2022 06:20:09 UTC
    // Output block: https://etherscan.io/block/14428082
    // Output block time: Mar 21 2022 06:20:09 UTC
    assert!(output.contains("14428082"), "{}", output);
});

// tests that we can create a new wallet with keystore
casttest!(new_wallet_keystore_with_password, |_: TestProject, mut cmd: TestCommand| {
    cmd.args(["wallet", "new", ".", "--unsafe-password", "test"]);
    let out = cmd.stdout_lossy();
    assert!(out.contains("Created new encrypted keystore file"));
    assert!(out.contains("Public Address of the key"));
});

// tests that `cast estimate` is working correctly.
casttest!(estimate_function_gas, |_: TestProject, mut cmd: TestCommand| {
    let eth_rpc_url = next_http_rpc_endpoint();
    cmd.args([
        "estimate",
        "vitalik.eth",
        "--value",
        "100",
        "deposit()",
        "--rpc-url",
        eth_rpc_url.as_str(),
    ]);
    let out: u32 = cmd.stdout_lossy().trim().parse().unwrap();
    // ensure we get a positive non-error value for gas estimate
    assert!(out.ge(&0));
});

// tests that `cast estimate --create` is working correctly.
casttest!(estimate_contract_deploy_gas, |_: TestProject, mut cmd: TestCommand| {
    let eth_rpc_url = next_http_rpc_endpoint();
    // sample contract code bytecode. Wouldn't run but is valid bytecode that the estimate method
    // accepts and could be deployed.
    cmd.args([
        "estimate",
        "--rpc-url",
        eth_rpc_url.as_str(),
        "--create",
        "0000",
        "ERC20(uint256,string,string)",
        "100",
        "Test",
        "TST",
    ]);

    let gas: u32 = cmd.stdout_lossy().trim().parse().unwrap();
    // ensure we get a positive non-error value for gas estimate
    assert!(gas > 0);
});

// tests that the `cast upload-signatures` command works correctly
casttest!(upload_signatures, |_: TestProject, mut cmd: TestCommand| {
    // test no prefix is accepted as function
    cmd.args(["upload-signature", "transfer(address,uint256)"]);
    let output = cmd.stdout_lossy();

    assert!(output.contains("Function transfer(address,uint256): 0xa9059cbb"), "{}", output);

    // test event prefix
    cmd.args(["upload-signature", "event Transfer(address,uint256)"]);
    let output = cmd.stdout_lossy();

    assert!(output.contains("Event Transfer(address,uint256): 0x69ca02dd4edd7bf0a4abb9ed3b7af3f14778db5d61921c7dc7cd545266326de2"), "{}", output);

    // test multiple sigs
    cmd.args([
        "upload-signature",
        "event Transfer(address,uint256)",
        "transfer(address,uint256)",
        "approve(address,uint256)",
    ]);
    let output = cmd.stdout_lossy();

    assert!(output.contains("Event Transfer(address,uint256): 0x69ca02dd4edd7bf0a4abb9ed3b7af3f14778db5d61921c7dc7cd545266326de2"), "{}", output);
    assert!(output.contains("Function transfer(address,uint256): 0xa9059cbb"), "{}", output);
    assert!(output.contains("Function approve(address,uint256): 0x095ea7b3"), "{}", output);

    // test abi
    cmd.args([
        "upload-signature",
        "event Transfer(address,uint256)",
        "transfer(address,uint256)",
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/ERC20Artifact.json")
            .into_os_string()
            .into_string()
            .unwrap()
            .as_str(),
    ]);
    let output = cmd.stdout_lossy();

    assert!(output.contains("Event Transfer(address,uint256): 0x69ca02dd4edd7bf0a4abb9ed3b7af3f14778db5d61921c7dc7cd545266326de2"), "{}", output);
    assert!(output.contains("Function transfer(address,uint256): 0xa9059cbb"), "{}", output);
    assert!(output.contains("Function approve(address,uint256): 0x095ea7b3"), "{}", output);
    assert!(output.contains("Function decimals(): 0x313ce567"), "{}", output);
    assert!(output.contains("Function allowance(address,address): 0xdd62ed3e"), "{}", output);
});

// tests that the `cast to-rlp` and `cast from-rlp` commands work correctly
casttest!(cast_rlp, |_: TestProject, mut cmd: TestCommand| {
    cmd.args(["--to-rlp", "[\"0xaa\", [[\"bb\"]], \"0xcc\"]"]);
    let out = cmd.stdout_lossy();
    assert!(out.contains("0xc881aac3c281bb81cc"), "{}", out);

    cmd.cast_fuse();
    cmd.args(["--from-rlp", "0xcbc58455556666c0c0c2c1c0"]);
    let out = cmd.stdout_lossy();
    assert!(out.contains("[[\"0x55556666\"],[],[],[[[]]]]"), "{}", out);
});

// test for cast_rpc without arguments
casttest!(cast_rpc_no_args, |_: TestProject, mut cmd: TestCommand| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast rpc eth_chainId`
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_chainId"]);
    let output = cmd.stdout_lossy();
    assert_eq!(output.trim_end(), r#""0x1""#);
});

// test for cast_rpc with arguments
casttest!(cast_rpc_with_args, |_: TestProject, mut cmd: TestCommand| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast rpc eth_getBlockByNumber 0x123 false`
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_getBlockByNumber", "0x123", "false"]);
    let output = cmd.stdout_lossy();
    assert!(output.contains(r#""number":"0x123""#), "{}", output);
});

// test for cast_rpc with raw params
casttest!(cast_rpc_raw_params, |_: TestProject, mut cmd: TestCommand| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast rpc eth_getBlockByNumber --raw '["0x123", false]'`
    cmd.args([
        "rpc",
        "--rpc-url",
        eth_rpc_url.as_str(),
        "eth_getBlockByNumber",
        "--raw",
        r#"["0x123", false]"#,
    ]);
    let output = cmd.stdout_lossy();
    assert!(output.contains(r#""number":"0x123""#), "{}", output);
});

// test for cast_rpc with direct params
casttest!(cast_rpc_raw_params_stdin, |_: TestProject, mut cmd: TestCommand| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `echo "\n[\n\"0x123\",\nfalse\n]\n" | cast rpc  eth_getBlockByNumber --raw
    cmd.args(["rpc", "--rpc-url", eth_rpc_url.as_str(), "eth_getBlockByNumber", "--raw"]).stdin(
        |mut stdin| {
            stdin.write_all(b"\n[\n\"0x123\",\nfalse\n]\n").unwrap();
        },
    );
    let output = cmd.stdout_lossy();
    assert!(output.contains(r#""number":"0x123""#), "{}", output);
});

// checks `cast calldata` can handle arrays
casttest!(calldata_array, |_: TestProject, mut cmd: TestCommand| {
    cmd.args(["calldata", "propose(string[])", "[\"\"]"]);
    let out = cmd.stdout_lossy();
    assert_eq!(out.trim(),"0xcde2baba0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000"
    );
});

// <https://github.com/foundry-rs/foundry/issues/2705>
casttest!(cast_run_succeeds, |_: TestProject, mut cmd: TestCommand| {
    let rpc = next_http_rpc_endpoint();
    cmd.args([
        "run",
        "-v",
        "0x2d951c5c95d374263ca99ad9c20c9797fc714330a8037429a3aa4c83d456f845",
        "--quick",
        "--rpc-url",
        rpc.as_str(),
    ]);
    let output = cmd.stdout_lossy();
    assert!(output.contains("Transaction successfully executed"));
    assert!(!output.contains("Revert"));
});
