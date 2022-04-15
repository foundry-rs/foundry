//! Contains various tests for checking cast commands
use std::env;

use foundry_cli_test_utils::{
    casttest,
    util::{TestCommand, TestProject},
};

// tests that the `cast find-block` command works correctly
casttest!(finds_block, |_: TestProject, mut cmd: TestCommand| {
    // Skip fork tests if the RPC url is not set.
    if std::env::var("ETH_RPC_URL").is_err() {
        eprintln!("Skipping test finds_block. ETH_RPC_URL is not set.");
        return
    };

    // Construct args
    let timestamp = "1647843609".to_string();
    let eth_rpc_url = env::var("ETH_RPC_URL").unwrap();

    // Call `cast find-block`
    cmd.args(["find-block", "--rpc-url", eth_rpc_url.as_str(), &timestamp]);
    let output = cmd.stdout_lossy();
    println!("{}", output);

    // Expect successful block query
    // Query: 1647843609, Mar 21 2022 06:20:09 UTC
    // Output block: https://etherscan.io/block/14428082
    // Output block time: Mar 21 2022 06:20:09 UTC
    assert!(output.contains("14428082"), "{}", output);
});
