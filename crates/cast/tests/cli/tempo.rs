//! Tempo-specific CLI tests for cast commands
//!
//! These tests verify Tempo transaction support in cast commands including:
//! - `--tempo.fee-token` for custom fee token transactions
//! - `--tempo.seq` for parallelizable nonces
//! - mktx support for Tempo transaction types

use alloy_eips::eip2718::Decodable2718;
use alloy_primitives::{Address, TxKind, U256, hex};
use anvil::NodeConfig;
use foundry_evm_networks::NetworkConfigs;
use foundry_test_utils::util::OutputExt;
use tempo_primitives::AASigned;

/// Tempo testnet chain ID
const TEMPO_TESTNET_CHAIN_ID: u64 = 42429;

mod anvil_const {
    /// First Anvil account
    pub const PK1: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    pub const ADDR1: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    /// Second Anvil account
    pub const ADDR2: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
}

/// Helper to spawn a Tempo-enabled Anvil node
async fn spawn_tempo_node() -> anvil::NodeHandle {
    let (_, handle) = anvil::spawn(
        NodeConfig::test()
            .with_chain_id(Some(TEMPO_TESTNET_CHAIN_ID))
            .with_networks(NetworkConfigs::with_tempo()),
    )
    .await;
    handle
}

// NOTE: The ERC20 transfer/approve tests with --tempo.fee-token are disabled because they require
// full Tempo EVM support in Anvil (the `tempo` feature). The mktx tests below verify that Tempo
// transactions can be built and signed correctly. Full execution tests should use Anvil with the
// `tempo` feature enabled.

// The following helpers and tests are commented out until Tempo EVM execution is available in CI:
//
// fn get_u256_from_cmd(cmd: &mut foundry_test_utils::TestCommand, args: &[&str]) -> U256 { ... }
// fn get_balance(...) -> U256 { ... }
// fn deploy_test_token(...) -> String { ... }
// async fn setup_tempo_token_test(...) -> (anvil::NodeHandle, String, String) { ... }
// forgetest_async!(tempo_erc20_transfer_with_fee_token, ...)
// forgetest_async!(tempo_erc20_approve_with_fee_token, ...)

// Tests that `cast mktx` works with `--tempo.fee-token` flag.
// This test verifies that mktx can create unsigned Tempo transactions with custom fee tokens.
casttest!(tempo_mktx_with_fee_token, async |_prj, cmd| {
    let handle = spawn_tempo_node().await;
    let rpc = handle.http_endpoint();

    // Create a transaction with --tempo.fee-token flag
    let output = cmd
        .args([
            "mktx",
            "--private-key",
            anvil_const::PK1,
            "--rpc-url",
            &rpc,
            "--nonce",
            "0",
            "--gas-limit",
            "21000",
            "--gas-price",
            "1000000000",
            "--tempo.fee-token",
            "0x0000000000000000000000000000000000000000",
            anvil_const::ADDR2,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify output is a hex-encoded transaction (starts with 0x76 for Tempo type)
    let output = output.trim();
    assert!(output.starts_with("0x"), "Transaction should be hex-encoded");
    // Tempo transactions are type 0x76
    assert!(output.starts_with("0x76"), "Transaction should be Tempo type (0x76)");
});

// Tests that `cast mktx` works with `--tempo.seq` (sequence key / nonce key) flag.
// This test verifies that mktx can create transactions with parallelizable nonces.
casttest!(tempo_mktx_with_sequence_key, async |_prj, cmd| {
    let handle = spawn_tempo_node().await;
    let rpc = handle.http_endpoint();

    // Create a transaction with --tempo.seq flag (sequence key for parallelizable nonces)
    let output = cmd
        .args([
            "mktx",
            "--private-key",
            anvil_const::PK1,
            "--rpc-url",
            &rpc,
            "--nonce",
            "0",
            "--gas-limit",
            "21000",
            "--gas-price",
            "1000000000",
            "--tempo.seq",
            "1", // Use sequence key 1 instead of default 0
            anvil_const::ADDR2,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify output is a hex-encoded Tempo transaction
    let output = output.trim();
    assert!(output.starts_with("0x76"), "Transaction should be Tempo type (0x76)");
});

// Tests that `cast mktx` works with both `--tempo.fee-token` and `--tempo.seq` flags combined.
casttest!(tempo_mktx_with_fee_token_and_sequence_key, async |_prj, cmd| {
    let handle = spawn_tempo_node().await;
    let rpc = handle.http_endpoint();

    // Create a transaction with both Tempo flags
    let output = cmd
        .args([
            "mktx",
            "--private-key",
            anvil_const::PK1,
            "--rpc-url",
            &rpc,
            "--nonce",
            "0",
            "--gas-limit",
            "21000",
            "--gas-price",
            "1000000000",
            "--tempo.fee-token",
            "0x0000000000000000000000000000000000000000",
            "--tempo.seq",
            "42",
            anvil_const::ADDR2,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify output is a hex-encoded Tempo transaction
    let output = output.trim();
    assert!(output.starts_with("0x76"), "Transaction should be Tempo type (0x76)");
});

// Tests that `cast send` works with `--tempo.fee-token` flag.
casttest!(tempo_send_with_fee_token, async |_prj, cmd| {
    let handle = spawn_tempo_node().await;
    let rpc = handle.http_endpoint();

    // Send a simple value transfer with --tempo.fee-token flag
    cmd.args([
        "send",
        "--private-key",
        anvil_const::PK1,
        "--rpc-url",
        &rpc,
        "--tempo.fee-token",
        "0x0000000000000000000000000000000000000000",
        "--value",
        "1000",
        anvil_const::ADDR2,
    ])
    .assert_success();
});

// Tests that `cast send` works with `--tempo.seq` flag for parallelizable nonces.
casttest!(tempo_send_with_sequence_key, async |_prj, cmd| {
    let handle = spawn_tempo_node().await;
    let rpc = handle.http_endpoint();

    // Send a transaction with a specific sequence key
    cmd.args([
        "send",
        "--private-key",
        anvil_const::PK1,
        "--rpc-url",
        &rpc,
        "--tempo.seq",
        "5",
        "--value",
        "1000",
        anvil_const::ADDR2,
    ])
    .assert_success();
});

// Tests that the tip20 alias works for erc20 command (Tempo-specific naming).
// Just verifies the command is recognized by checking it doesn't fail with "unknown subcommand"
casttest!(tip20_alias_works, async |_prj, cmd| {
    // Check that "tip20 --help" works to verify the alias is recognized
    let output = cmd.args(["tip20", "--help"]).assert_success().get_output().stdout_lossy();

    // Verify it's the ERC20 help (contains expected subcommands)
    assert!(output.contains("balance"), "tip20 should have balance subcommand");
    assert!(output.contains("transfer"), "tip20 should have transfer subcommand");
});

// Tests that `cast mktx --raw-unsigned` with Tempo options creates proper unsigned tx.
casttest!(tempo_mktx_raw_unsigned_with_fee_token, async |_prj, cmd| {
    let handle = spawn_tempo_node().await;
    let rpc = handle.http_endpoint();

    // Create an unsigned raw transaction with Tempo fee token
    let output = cmd
        .args([
            "mktx",
            "--from",
            anvil_const::ADDR1,
            "--rpc-url",
            &rpc,
            "--nonce",
            "0",
            "--gas-limit",
            "21000",
            "--gas-price",
            "1000000000",
            "--tempo.fee-token",
            "0x0000000000000000000000000000000000000000",
            "--raw-unsigned",
            anvil_const::ADDR2,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    // Verify output is a hex-encoded transaction
    let output = output.trim();
    assert!(output.starts_with("0x"), "Transaction should be hex-encoded");
});

// Tests that mktx with Tempo options produces a decodable transaction with correct fields.
// This test decodes the produced transaction and validates the Tempo-specific fields.
casttest!(tempo_mktx_decode_and_validate, async |_prj, cmd| {
    let handle = spawn_tempo_node().await;
    let rpc = handle.http_endpoint();

    let expected_fee_token: Address = "0x1234567890123456789012345678901234567890".parse().unwrap();
    let expected_nonce_key = 42u64;

    // Create a transaction with both Tempo flags
    let output = cmd
        .args([
            "mktx",
            "--private-key",
            anvil_const::PK1,
            "--rpc-url",
            &rpc,
            "--nonce",
            "0",
            "--gas-limit",
            "21000",
            "--gas-price",
            "1000000000",
            "--tempo.fee-token",
            &format!("{expected_fee_token:?}"),
            "--tempo.seq",
            &expected_nonce_key.to_string(),
            anvil_const::ADDR2,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let output = output.trim();
    assert!(output.starts_with("0x76"), "Transaction should be Tempo type (0x76)");

    // Decode the transaction and validate Tempo fields
    let tx_bytes = hex::decode(output.trim_start_matches("0x")).expect("Should be valid hex");
    let decoded = AASigned::decode_2718(&mut tx_bytes.as_slice())
        .expect("Should decode as Tempo transaction");

    // Validate fee token
    assert_eq!(decoded.tx().fee_token, Some(expected_fee_token), "Fee token should match");

    // Validate nonce key
    assert_eq!(decoded.tx().nonce_key, U256::from(expected_nonce_key), "Nonce key should match");

    // Validate calls array contains our target
    assert!(!decoded.tx().calls.is_empty(), "Calls should not be empty");
    let first_call = &decoded.tx().calls[0];
    let expected_to: Address = anvil_const::ADDR2.parse().unwrap();
    assert_eq!(first_call.to, TxKind::Call(expected_to), "Call target should match");
});
