//! Contains various tests for checking cast commands

use clap::CommandFactory;
use foundry_cli::opts::cast::Opts;
use foundry_cli_test_utils::{
    casttest,
    util::{TestCommand, TestProject},
};
use foundry_utils::rpc::next_http_rpc_endpoint;
use std::{io::Write, path::PathBuf};

// tests `--help` is printed to std out
casttest!(print_help, |_: TestProject, mut cmd: TestCommand| {
    cmd.arg("--help");
    cmd.assert_non_empty_stdout();
});

// tests `--help` for all subcommand
casttest!(print_cast_subcommand_help, |_: TestProject, mut cmd: TestCommand| {
    let cast = Opts::command();
    for sub_command in cast.get_subcommands() {
        cmd.cast_fuse().args([sub_command.get_name(), "--help"]);
        cmd.assert_non_empty_stdout();
    }
});

// tests that the `cast block` command works correctly
casttest!(latest_block, |_: TestProject, mut cmd: TestCommand| {
    let eth_rpc_url = next_http_rpc_endpoint();

    // Call `cast find-block`
    cmd.args(["block", "latest", "--rpc-url", eth_rpc_url.as_str()]);
    let output = cmd.stdout_lossy();
    assert!(output.contains("transactions:"));
    assert!(output.contains("gasUsed"));

    // <https://etherscan.io/block/15007840>
    cmd.cast_fuse().args(["block", "15007840", "-f", "hash", "--rpc-url", eth_rpc_url.as_str()]);
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
    assert!(out.contains("Address"));
});

// tests that we can get the address of a keystore file
casttest!(wallet_address_keystore_with_password_file, |_: TestProject, mut cmd: TestCommand| {
    let keystore_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keystore");

    cmd.args([
        "wallet",
        "address",
        "--keystore",
        keystore_dir
            .join("UTC--2022-12-20T10-30-43.591916000Z--ec554aeafe75601aaab43bd4621a22284db566c2")
            .to_str()
            .unwrap(),
        "--password-file",
        keystore_dir.join("password-ec554").to_str().unwrap(),
    ]);
    let out = cmd.stdout_lossy();
    assert!(out.contains("0xeC554aeAFE75601AaAb43Bd4621A22284dB566C2"));
});

// tests that `cast wallet sign message` outputs the expected signature
casttest!(cast_wallet_sign_message_utf8_data, |_: TestProject, mut cmd: TestCommand| {
    cmd.args([
        "wallet",
        "sign",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "message",
        "test",
    ]);
    let output = cmd.stdout_lossy();
    assert_eq!(output.trim(), "0xfe28833983d6faa0715c7e8c3873c725ddab6fa5bf84d40e780676e463e6bea20fc6aea97dc273a98eb26b0914e224c8dd5c615ceaab69ddddcf9b0ae3de0e371c");
});

// tests that `cast wallet sign message` outputs the expected signature, given a 0x-prefixed data
casttest!(cast_wallet_sign_message_hex_data, |_: TestProject, mut cmd: TestCommand| {
    cmd.args([
        "wallet",
        "sign",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "message",
        "0x0000000000000000000000000000000000000000000000000000000000000000",
    ]);
    let output = cmd.stdout_lossy();
    assert_eq!(output.trim(), "0x23a42ca5616ee730ff3735890c32fc7b9491a9f633faca9434797f2c845f5abf4d9ba23bd7edb8577acebaa3644dc5a4995296db420522bb40060f1693c33c9b1c");
});

// tests that `cast wallet sign typed-data` outputs the expected signature, given a JSON string
casttest!(cast_wallet_sign_typed_data_string, |_: TestProject, mut cmd: TestCommand| {
    cmd.args([
        "wallet",
        "sign",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "typed-data",
        "{\"types\": {\"EIP712Domain\": [{\"name\": \"name\",\"type\": \"string\"},{\"name\": \"version\",\"type\": \"string\"},{\"name\": \"chainId\",\"type\": \"uint256\"},{\"name\": \"verifyingContract\",\"type\": \"address\"}],\"Message\": [{\"name\": \"data\",\"type\": \"string\"}]},\"primaryType\": \"Message\",\"domain\": {\"name\": \"example.metamask.io\",\"version\": \"1\",\"chainId\": \"1\",\"verifyingContract\": \"0x0000000000000000000000000000000000000000\"},\"message\": {\"data\": \"Hello!\"}}",
    ]);
    let output = cmd.stdout_lossy();
    assert_eq!(output.trim(), "0x06c18bdc8163219fddc9afaf5a0550e381326474bb757c86dc32317040cf384e07a2c72ce66c1a0626b6750ca9b6c035bf6f03e7ed67ae2d1134171e9085c0b51b");
});

// tests that `cast wallet sign typed-data` outputs the expected signature, given a JSON file
casttest!(cast_wallet_sign_typed_data_file, |_: TestProject, mut cmd: TestCommand| {
    cmd.args([
        "wallet",
        "sign",
        "--private-key",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "typed-data",
        "-f",
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sign_typed_data.json")
            .into_os_string()
            .into_string()
            .unwrap()
            .as_str(),
    ]);
    let output = cmd.stdout_lossy();
    assert_eq!(output.trim(), "0x06c18bdc8163219fddc9afaf5a0550e381326474bb757c86dc32317040cf384e07a2c72ce66c1a0626b6750ca9b6c035bf6f03e7ed67ae2d1134171e9085c0b51b");
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

// tests that the `cast storage` command works correctly
casttest!(test_live_cast_storage_succeeds, |_: TestProject, mut cmd: TestCommand| {
    // ignore if ETHERSCAN_API_KEY not set
    if std::env::var("ETHERSCAN_API_KEY").is_err() {
        eprintln!("ETHERSCAN_API_KEY not set");
        return
    }

    let eth_rpc_url = next_http_rpc_endpoint();

    // WETH
    // version < min, so empty storage layout
    let address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
    cmd.cast_fuse().args(["storage", "--rpc-url", eth_rpc_url.as_str(), address]);
    let output = cmd.stderr_lossy();
    assert!(output.contains("Storage layout is empty"), "{}", output);
    // first slot is the name, always is "Wrapped Ether"
    cmd.cast_fuse().args(["storage", "--rpc-url", eth_rpc_url.as_str(), address, "0"]);
    let output = cmd.stdout_lossy();
    assert!(
        output.contains("0x577261707065642045746865720000000000000000000000000000000000001a"),
        "{output}",
    );

    // Polygon bridge proxy
    let address = "0xA0c68C638235ee32657e8f720a23ceC1bFc77C77";
    cmd.cast_fuse().args(["storage", "--rpc-url", eth_rpc_url.as_str(), address]);
    let output = cmd.stdout_lossy();
    assert!(
        output.contains("RootChainManager") &&
            output.contains("_roles") &&
            output.contains("mapping(bytes32 => struct AccessControl.RoleData)"),
        "{output}",
    );
    // first slot is `inited`, always is 1
    cmd.cast_fuse().args(["storage", "--rpc-url", eth_rpc_url.as_str(), address, "0"]);
    let output = cmd.stdout_lossy();
    assert!(
        output.contains("0x0000000000000000000000000000000000000000000000000000000000000001"),
        "{output}",
    );
});

// tests that `cast --to-base` commands are working correctly.
casttest!(cast_to_base, |_: TestProject, mut cmd: TestCommand| {
    let values = [
        "1",
        "100",
        "100000",
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
        "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "-1",
        "-100",
        "-100000",
        "-57896044618658097711785492504343953926634992332820282019728792003956564819968",
    ];
    for value in values {
        for subcmd in ["--to-base", "--to-hex", "--to-dec"] {
            if subcmd == "--to-base" {
                for base in ["bin", "oct", "dec", "hex"] {
                    cmd.cast_fuse().args([subcmd, value, base]);
                    assert!(!cmd.stdout_lossy().trim().is_empty());
                }
            } else {
                cmd.cast_fuse().args([subcmd, value]);
                assert!(!cmd.stdout_lossy().trim().is_empty());
            }
        }
    }
});

// tests that revert reason is only present if transaction has reverted.
casttest!(cast_receipt_revert_reason, |_: TestProject, mut cmd: TestCommand| {
    let rpc = next_http_rpc_endpoint();

    // <https://etherscan.io/tx/0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e>
    cmd.cast_fuse().args([
        "receipt",
        "0x44f2aaa351460c074f2cb1e5a9e28cbc7d83f33e425101d2de14331c7b7ec31e",
        "--rpc-url",
        rpc.as_str(),
    ]);
    let output = cmd.stdout_lossy();
    assert!(!output.contains("revertReason"));

    // <https://etherscan.io/tx/0x0e07d8b53ed3d91314c80e53cf25bcde02084939395845cbb625b029d568135c>
    cmd.cast_fuse().args([
        "receipt",
        "0x0e07d8b53ed3d91314c80e53cf25bcde02084939395845cbb625b029d568135c",
        "--rpc-url",
        rpc.as_str(),
    ]);
    let output = cmd.stdout_lossy();
    assert!(output.contains("revertReason"));
    assert!(output.contains("Transaction too old"));
});

casttest!(cast_access_list, |_: TestProject, mut cmd: TestCommand| {
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
    ]);

    let output = cmd.stdout_lossy();
    assert!(output.contains("address: 0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"));
    assert!(output.contains("0x0d2a19d3ac39dc6cc6fd07423195495e18679bd8c7dd610aa1db7cd784a683a8"));
    assert!(output.contains("0x7fba2702a7d6e85ac783a88eacdc48e51310443458071f6db9ac66f8ca7068b8"));
});
