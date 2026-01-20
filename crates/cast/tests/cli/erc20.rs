//! Contains various tests for checking cast erc20 subcommands

use alloy_primitives::U256;
use anvil::NodeConfig;
use foundry_test_utils::util::OutputExt;

mod anvil_const {
    /// First Anvil account
    pub const PK1: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    pub const ADDR1: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    /// Second Anvil account
    pub const _PK2: &str = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
    pub const ADDR2: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    /// Contract address deploying from ADDR1 with nonce 0
    pub const TOKEN: &str = "0x5FbDB2315678afecb367f032d93F642f64180aa3";
}

fn get_u256_from_cmd(cmd: &mut foundry_test_utils::TestCommand, args: &[&str]) -> U256 {
    let output = cmd.cast_fuse().args(args).assert_success().get_output().stdout_lossy();

    // Parse balance from output (format: "100000000000000000000 [1e20]")
    output.split_whitespace().next().unwrap().parse().unwrap()
}

fn get_balance(
    cmd: &mut foundry_test_utils::TestCommand,
    token: &str,
    address: &str,
    rpc: &str,
) -> U256 {
    get_u256_from_cmd(cmd, &["erc20", "balance", token, address, "--rpc-url", rpc])
}

fn get_allowance(
    cmd: &mut foundry_test_utils::TestCommand,
    token: &str,
    owner: &str,
    spender: &str,
    rpc: &str,
) -> U256 {
    get_u256_from_cmd(cmd, &["erc20", "allowance", token, owner, spender, "--rpc-url", rpc])
}

/// Helper function to deploy TestToken contract
fn deploy_test_token(
    cmd: &mut foundry_test_utils::TestCommand,
    rpc: &str,
    private_key: &str,
) -> String {
    cmd.args([
        "create",
        "--private-key",
        private_key,
        "--rpc-url",
        rpc,
        "--broadcast",
        "src/TestToken.sol:TestToken",
    ])
    .assert_success();

    // Return the standard deployment address (nonce 0 from first account)
    anvil_const::TOKEN.to_string()
}

/// Helper to setup anvil node and deploy test token
async fn setup_token_test(
    prj: &foundry_test_utils::TestProject,
    cmd: &mut foundry_test_utils::TestCommand,
) -> (String, String) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();

    // Deploy TestToken contract
    foundry_test_utils::util::initialize(prj.root());
    prj.add_source("TestToken.sol", include_str!("../fixtures/TestToken.sol"));
    let token = deploy_test_token(cmd, &rpc, anvil_const::PK1);

    (rpc, token)
}

// tests that `balance` and `transfer` commands works correctly
forgetest_async!(erc20_transfer_approve_success, |prj, cmd| {
    let (rpc, token) = setup_token_test(&prj, &mut cmd).await;

    // Test constants
    let transfer_amount = U256::from(100_000_000_000_000_000_000u128); // 100 tokens (18 decimals)
    let initial_supply = U256::from(1_000_000_000_000_000_000_000u128); // 1000 tokens total supply

    // Verify initial balances
    let addr1_balance_before = get_balance(&mut cmd, &token, anvil_const::ADDR1, &rpc);
    let addr2_balance_before = get_balance(&mut cmd, &token, anvil_const::ADDR2, &rpc);
    assert_eq!(addr1_balance_before, initial_supply);
    assert_eq!(addr2_balance_before, U256::ZERO);

    // Test ERC20 transfer from ADDR1 to ADDR2
    cmd.cast_fuse()
        .args([
            "erc20",
            "transfer",
            &token,
            anvil_const::ADDR2,
            &transfer_amount.to_string(),
            "--yes",
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
        ])
        .assert_success();

    // Verify balance changes after transfer
    let addr1_balance_after = get_balance(&mut cmd, &token, anvil_const::ADDR1, &rpc);
    let addr2_balance_after = get_balance(&mut cmd, &token, anvil_const::ADDR2, &rpc);
    assert_eq!(addr1_balance_after, addr1_balance_before - transfer_amount);
    assert_eq!(addr2_balance_after, addr2_balance_before + transfer_amount);
});

// tests that `approve` and `allowance` commands works correctly
forgetest_async!(erc20_approval_allowance, |prj, cmd| {
    let (rpc, token) = setup_token_test(&prj, &mut cmd).await;

    // ADDR1 approves ADDR2 to spend their tokens
    let approve_amount = U256::from(50_000_000_000_000_000_000u128); // 50 tokens
    cmd.cast_fuse()
        .args([
            "erc20",
            "approve",
            &token,
            anvil_const::ADDR2,
            &approve_amount.to_string(),
            "--yes",
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
        ])
        .assert_success();

    // Verify allowance was set
    let allowance = get_allowance(&mut cmd, &token, anvil_const::ADDR1, anvil_const::ADDR2, &rpc);
    assert_eq!(allowance, approve_amount);
});

// tests that `name`, `symbol`, `decimals`, and `totalSupply` commands work correctly
forgetest_async!(erc20_metadata_success, |prj, cmd| {
    let (rpc, token) = setup_token_test(&prj, &mut cmd).await;

    // Test name
    let output = cmd
        .cast_fuse()
        .args(["erc20", "name", &token, "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(output.trim(), "Test Token");

    // Test symbol
    let output = cmd
        .cast_fuse()
        .args(["erc20", "symbol", &token, "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(output.trim(), "TEST");

    // Test decimals
    let output = cmd
        .cast_fuse()
        .args(["erc20", "decimals", &token, "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(output.trim(), "18");

    // Test totalSupply
    let output = cmd
        .cast_fuse()
        .args(["erc20", "total-supply", &token, "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let total_supply: U256 = output.split_whitespace().next().unwrap().parse().unwrap();
    assert_eq!(total_supply, U256::from(1_000_000_000_000_000_000_000u128));
});

// tests that `mint` command works correctly
forgetest_async!(erc20_mint_success, |prj, cmd| {
    let (rpc, token) = setup_token_test(&prj, &mut cmd).await;

    let mint_amount = U256::from(500_000_000_000_000_000_000u128); // 500 tokens
    let initial_supply = U256::from(1_000_000_000_000_000_000_000u128); // 1000 tokens

    // Get initial balance and supply
    let addr2_balance_before = get_balance(&mut cmd, &token, anvil_const::ADDR2, &rpc);
    assert_eq!(addr2_balance_before, U256::ZERO);

    // Mint tokens to ADDR2 (only owner can mint)
    cmd.cast_fuse()
        .args([
            "erc20",
            "mint",
            &token,
            anvil_const::ADDR2,
            &mint_amount.to_string(),
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1, // PK1 is the owner/deployer
        ])
        .assert_success();

    // Verify balance increased
    let addr2_balance_after = get_balance(&mut cmd, &token, anvil_const::ADDR2, &rpc);
    assert_eq!(addr2_balance_after, mint_amount);

    // Verify totalSupply increased
    let output = cmd
        .cast_fuse()
        .args(["erc20", "total-supply", &token, "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let total_supply: U256 = output.split_whitespace().next().unwrap().parse().unwrap();
    assert_eq!(total_supply, initial_supply + mint_amount);
});

// tests that `burn` command works correctly
forgetest_async!(erc20_burn_success, |prj, cmd| {
    let (rpc, token) = setup_token_test(&prj, &mut cmd).await;

    let burn_amount = U256::from(200_000_000_000_000_000_000u128); // 200 tokens
    let initial_supply = U256::from(1_000_000_000_000_000_000_000u128); // 1000 tokens

    // Get initial balance
    let addr1_balance_before = get_balance(&mut cmd, &token, anvil_const::ADDR1, &rpc);
    assert_eq!(addr1_balance_before, initial_supply);

    // Burn tokens from ADDR1
    cmd.cast_fuse()
        .args([
            "erc20",
            "burn",
            &token,
            &burn_amount.to_string(),
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
        ])
        .assert_success();

    // Verify balance decreased
    let addr1_balance_after = get_balance(&mut cmd, &token, anvil_const::ADDR1, &rpc);
    assert_eq!(addr1_balance_after, addr1_balance_before - burn_amount);

    // Verify totalSupply decreased
    let output = cmd
        .cast_fuse()
        .args(["erc20", "total-supply", &token, "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let total_supply: U256 = output.split_whitespace().next().unwrap().parse().unwrap();
    assert_eq!(total_supply, initial_supply - burn_amount);
});

// tests that transfer with --yes flag skips confirmation prompt
forgetest_async!(erc20_transfer_with_yes_flag, |prj, cmd| {
    let (rpc, token) = setup_token_test(&prj, &mut cmd).await;

    let transfer_amount = U256::from(50_000_000_000_000_000_000u128); // 50 tokens

    // Transfer with --yes flag should succeed without prompting
    cmd.cast_fuse()
        .args([
            "erc20",
            "transfer",
            &token,
            anvil_const::ADDR2,
            &transfer_amount.to_string(),
            "--yes",
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
        ])
        .assert_success();

    // Verify the transfer actually happened
    let addr2_balance = get_balance(&mut cmd, &token, anvil_const::ADDR2, &rpc);
    assert_eq!(addr2_balance, transfer_amount);
});

// tests that approve with --yes flag skips confirmation prompt
forgetest_async!(erc20_approve_with_yes_flag, |prj, cmd| {
    let (rpc, token) = setup_token_test(&prj, &mut cmd).await;

    let approve_amount = U256::from(75_000_000_000_000_000_000u128); // 75 tokens

    // Approve with --yes flag should succeed without prompting
    cmd.cast_fuse()
        .args([
            "erc20",
            "approve",
            &token,
            anvil_const::ADDR2,
            &approve_amount.to_string(),
            "--yes",
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
        ])
        .assert_success();

    // Verify the approval actually happened
    let allowance = get_allowance(&mut cmd, &token, anvil_const::ADDR1, anvil_const::ADDR2, &rpc);
    assert_eq!(allowance, approve_amount);
});

// tests that `transfer` command works with gas options
forgetest_async!(erc20_transfer_with_gas_opts, |prj, cmd| {
    let (rpc, token) = setup_token_test(&prj, &mut cmd).await;

    let transfer_amount = U256::from(50_000_000_000_000_000_000u128); // 50 tokens

    cmd.cast_fuse()
        .args([
            "erc20",
            "transfer",
            &token,
            anvil_const::ADDR2,
            &transfer_amount.to_string(),
            "--yes",
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
            "--gas-limit",
            "100000",
            "--gas-price",
            "2000000000",
        ])
        .assert_success();

    // Verify transfer succeeded
    let balance = get_balance(&mut cmd, &token, anvil_const::ADDR2, &rpc);
    assert_eq!(balance, transfer_amount);
});

// tests that `transfer` command fails with insufficient gas limit
forgetest_async!(erc20_transfer_insufficient_gas, |prj, cmd| {
    let (rpc, token) = setup_token_test(&prj, &mut cmd).await;

    let transfer_amount = U256::from(50_000_000_000_000_000_000u128); // 50 tokens

    // Transfer with insufficient gas limit (ERC20 transfer needs ~50k gas)
    cmd.cast_fuse()
        .args([
            "erc20",
            "transfer",
            &token,
            anvil_const::ADDR2,
            &transfer_amount.to_string(),
            "--yes",
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
            "--gas-limit",
            "1000", // Way too low for ERC20 transfer
        ])
        .assert_failure();

    // Verify transfer did NOT occur
    let balance = get_balance(&mut cmd, &token, anvil_const::ADDR2, &rpc);
    assert_eq!(balance, U256::ZERO);
});

// tests that `transfer` command fails with incorrect nonce
forgetest_async!(erc20_transfer_incorrect_nonce, |prj, cmd| {
    let (rpc, token) = setup_token_test(&prj, &mut cmd).await;

    let transfer_amount = U256::from(50_000_000_000_000_000_000u128); // 50 tokens

    cmd.cast_fuse()
        .args([
            "erc20",
            "transfer",
            &token,
            anvil_const::ADDR2,
            &transfer_amount.to_string(),
            "--yes",
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
        ])
        .assert_success();

    // Transfer with nonce too low
    cmd.cast_fuse()
        .args([
            "erc20",
            "transfer",
            &token,
            anvil_const::ADDR2,
            &transfer_amount.to_string(),
            "--yes",
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
            "--nonce",
            "0", // Too low nonce
        ])
        .assert_failure();

    // Verify transfer did NOT occur
    let balance = get_balance(&mut cmd, &token, anvil_const::ADDR2, &rpc);
    assert_eq!(balance, transfer_amount); // 2nd transfer failed
});

// tests that the --curl flag outputs a valid curl command for cast erc20 balance
casttest!(curl_erc20_balance, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let token = "0xdead000000000000000000000000000000000000";
    let owner = "0xbeef000000000000000000000000000000000000";

    let output = cmd
        .args(["erc20", "balance", token, owner, "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_call"));
    assert!(output.contains(rpc));
});

// tests that the --curl flag outputs a valid curl command for cast erc20 name
casttest!(curl_erc20_name, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let token = "0xdead000000000000000000000000000000000000";

    let output = cmd
        .args(["erc20", "name", token, "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_call"));
    assert!(output.contains(rpc));
});

// tests that the --curl flag outputs a valid curl command for cast erc20 decimals
casttest!(curl_erc20_decimals, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let token = "0xdead000000000000000000000000000000000000";

    let output = cmd
        .args(["erc20", "decimals", token, "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_call"));
    assert!(output.contains(rpc));
});

// tests that the --curl flag outputs a valid curl command for cast erc20 total-supply
casttest!(curl_erc20_total_supply, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let token = "0xdead000000000000000000000000000000000000";

    let output = cmd
        .args(["erc20", "total-supply", token, "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_call"));
    assert!(output.contains(rpc));
});

// tests that the --curl flag outputs a valid curl command for erc20 balance
casttest!(erc20_curl_balance, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let token = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"; // USDC
    let owner = "0xdead000000000000000000000000000000000000";

    let output = cmd
        .args(["erc20", "balance", token, owner, "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_call"));
    assert!(output.contains(rpc));
});

// tests that the --curl flag outputs a valid curl command for erc20 name
casttest!(erc20_curl_name, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let token = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"; // USDC

    let output = cmd
        .args(["erc20", "name", token, "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_call"));
    assert!(output.contains(rpc));
});

// tests that the --curl flag outputs a valid curl command for erc20 decimals
casttest!(erc20_curl_decimals, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let token = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"; // USDC

    let output = cmd
        .args(["erc20", "decimals", token, "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_call"));
    assert!(output.contains(rpc));
});

// tests that the --curl flag outputs a valid curl command for erc20 total-supply
casttest!(erc20_curl_total_supply, |_prj, cmd| {
    let rpc = "https://eth.example.com";
    let token = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"; // USDC

    let output = cmd
        .args(["erc20", "total-supply", token, "--rpc-url", rpc, "--curl"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify curl command structure
    assert!(output.contains("curl -X POST"));
    assert!(output.contains("eth_call"));
    assert!(output.contains(rpc));
});
