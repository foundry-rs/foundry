//! Contains various tests for checking forge's commands
use foundry_cli_test_utils::{
    ethers_solc::PathStyle,
    forgetest,
    util::{TestCommand, TestProject},
};
use std::env;

// tests `--help` is printed to std out
forgetest!(print_help, |_: TestProject, mut cmd: TestCommand| {
    cmd.arg("--help");
    cmd.assert_non_empty_stdout();
});

// checks that `clean` can be invoked even if out and cache don't exist
forgetest!(can_clean_non_existing, |prj: TestProject, mut cmd: TestCommand| {
    cmd.arg("clean");
    cmd.assert_empty_stdout();
    prj.assert_cleaned();
});

// checks that `clean` removes dapptools style paths
forgetest!(can_clean, |prj: TestProject, mut cmd: TestCommand| {
    prj.assert_create_dirs_exists();
    prj.assert_style_paths_exist(PathStyle::Dapptools);
    cmd.arg("clean");
    cmd.assert_empty_stdout();
    prj.assert_cleaned();
});

// checks that `clean` removes hardhat style paths
forgetest!(can_clean_hardhat, PathStyle::HardHat, |prj: TestProject, mut cmd: TestCommand| {
    prj.assert_create_dirs_exists();
    prj.assert_style_paths_exist(PathStyle::HardHat);
    cmd.arg("clean");
    cmd.assert_empty_stdout();
    prj.assert_cleaned();
});

// test against a local checkout, useful to debug with local ethers-rs patch
forgetest!(can_compile_local_spells, |_: TestProject, mut cmd: TestCommand| {
    let current_dir = std::env::current_dir().unwrap();
    let root = current_dir
        .join("../../foundry-integration-tests/testdata/spells-mainnet")
        .to_string_lossy()
        .to_string();
    println!("project root: \"{}\"", root);

    let eth_rpc_url = env::var("ETH_RPC_URL").unwrap();
    let dss_exec_lib = "src/DssSpell.sol:DssExecLib:0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4";

    cmd.args([
        "build",
        "--root",
        root.as_str(),
        // "--fork-url",
        // eth_rpc_url.as_str(),
        "--libraries",
        dss_exec_lib,
        // "-vvv",
        "--force",
    ]);
    cmd.print_output();
});

#[test]
fn find_bin_diff() {
    let a = "0x608060405260016000806101000a81548160ff02191690831515021790555034801561002a57600080fd5b5060cd806100396000396000f3fe6080604052348015600f57600080fd5b506004361060325760003560e01c8063ba414fa6146037578063fa7626d4146055575b600080fd5b603d6073565b60405180821515815260200191505060405180910390f35b605b6086565b60405180821515815260200191505060405180910390f35b600060019054906101000a900460ff1681565b60008054906101000a900460ff168156fea2646970667358221220\
    f205d5ae7885750ab20a24d6e7556df112a28020037d3332b2bc32acdc5633a2\
    \
    64736f6c634300060c0033";
    let b = "0x608060405260016000806101000a81548160ff02191690831515021790555034801561002a57600080fd5b5060cd806100396000396000f3fe6080604052348015600f57600080fd5b506004361060325760003560e01c8063ba414fa6146037578063fa7626d4146055575b600080fd5b603d6073565b60405180821515815260200191505060405180910390f35b605b6086565b60405180821515815260200191505060405180910390f35b600060019054906101000a900460ff1681565b60008054906101000a900460ff168156fea2646970667358221220\
    6363f82f4f9257abaf49bd09c1fc0c655e002ae464a00255e63a14b9d6f906b1\
    64736f6c634300060c0033";

    for (idx, (l, r)) in a.chars().zip(b.chars()).enumerate() {
        if l != r {
            println!("{}", &a[..idx]);
            break
        }
    }
}
