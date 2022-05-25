//! Contains various tests for checking cast commands

use foundry_cli_test_utils::{
    casttest,
    util::{TestCommand, TestProject},
};
use foundry_utils::rpc::next_http_rpc_endpoint;
use std::path::PathBuf;

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
