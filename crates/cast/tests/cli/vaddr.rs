//! Integration tests for `cast vaddr` subcommands against the Tempo Moderato testnet.
//!
//! Each test is self-contained: it generates fresh wallets, funds them via
//! `tempo_fundAddress`, and exercises a distinct flow through the virtual address
//! registry.

use alloy_primitives::{Address, U256, hex};
use alloy_signer_local::PrivateKeySigner;
use foundry_test_utils::{str, util::OutputExt};
use std::str::FromStr;
use tempo_contracts::precompiles::DEFAULT_FEE_TOKEN;

const RPC: &str = "https://rpc.moderato.tempo.xyz";

/// Fund `addr` with gas and TIP-20 tokens via the Moderato faucet RPC.
fn fund(cmd: &mut foundry_test_utils::TestCommand, addr: &str) {
    cmd.cast_fuse().args(["rpc", "--rpc-url", RPC, "tempo_fundAddress", addr]).assert_success();
}

/// Return the DEFAULT_FEE_TOKEN (PathUSD) balance of `addr`.
fn path_usd_balance(cmd: &mut foundry_test_utils::TestCommand, addr: &str) -> U256 {
    let out = cmd
        .cast_fuse()
        .args(["erc20", "balance", &DEFAULT_FEE_TOKEN.to_checksum(None), addr, "--rpc-url", RPC])
        .assert_success()
        .get_output()
        .stdout_lossy();
    out.split_whitespace().next().unwrap().parse().unwrap()
}

/// Mine and register a virtual master for `owner`, validating the `cast vaddr create`
/// output against the expected template, and return the first virtual address.
fn create_first_vaddr(cmd: &mut foundry_test_utils::TestCommand, owner: &str, pk: &str) -> Address {
    let stdout = cmd
        .cast_fuse()
        .args(["vaddr", "create", "--owner", owner, "--private-key", pk, "--rpc-url", RPC])
        .assert_success()
        .stdout_eq(str![[r#"
Mining TIP-1022 salt for [..] with [..] threads...
Found salt [ELAPSED]
Salt:              [..]
Registration hash: [..]
Master ID:         [..]

Virtual addresses:
  tag=0x000000000000  [..]
Submitting registerVirtualMaster([..])...
...
"#]])
        .get_output()
        .stdout_lossy();

    stdout
        .lines()
        .find_map(|l| l.strip_prefix("  tag=0x000000000000  "))
        .and_then(|s| Address::from_str(s.trim()).ok())
        .expect("virtual address not found in create output")
}

// Tests that `cast vaddr create` mines a salt, registers it on-chain, and that
// `cast vaddr resolve` returns the correct master address.
casttest!(vaddr_create_and_resolve, async |_prj, cmd| {
    let signer = PrivateKeySigner::random();
    let pk = hex::encode(signer.credential().to_bytes());
    let master = format!("{}", signer.address());

    // Fund the master wallet (gas + DEFAULT_FEE_TOKEN fee token).
    fund(&mut cmd, &master);

    // Mine and register; --no-random starts from salt 0 for test determinism.
    let vaddr = create_first_vaddr(&mut cmd, &master, &pk);

    // Resolve must return this master.
    cmd.cast_fuse()
        .args(["vaddr", "resolve", &format!("{vaddr}"), "--rpc-url", RPC])
        .assert_success()
        .stdout_eq(format!(
            r#"Virtual address: {vaddr}
Master ID:       0x[..]
User tag:        0x000000000000
Master address:  {master}

"#
        ));
});

// Tests that transferring PathUSD to a virtual address auto-forwards the tokens
// to the registered master wallet.
casttest!(vaddr_transfer_auto_forwards_to_master, async |_prj, cmd| {
    let master_signer = PrivateKeySigner::random();
    let master_pk = hex::encode(master_signer.credential().to_bytes());
    let master = format!("{}", master_signer.address());

    let sender_signer = PrivateKeySigner::random();
    let sender_pk = hex::encode(sender_signer.credential().to_bytes());
    let sender = format!("{}", sender_signer.address());

    // Fund both wallets.
    fund(&mut cmd, &master);
    fund(&mut cmd, &sender);

    // Register master and derive the first virtual address.
    let vaddr = create_first_vaddr(&mut cmd, &master, &master_pk);

    // Snapshot master balance before the transfer.
    let balance_before = path_usd_balance(&mut cmd, &master);

    // Send 1 PathUSD (6 decimals) from sender to the virtual address.
    let amount = U256::from(1_000_000u64);
    cmd.cast_fuse()
        .args([
            "send",
            &DEFAULT_FEE_TOKEN.to_checksum(None),
            "transfer(address,uint256)",
            &format!("{vaddr}"),
            &amount.to_string(),
            "--private-key",
            &sender_pk,
            "--rpc-url",
            RPC,
            "--tempo.fee-token",
            "0",
        ])
        .assert_success();

    // Master balance must have grown by exactly the transferred amount.
    let balance_after = path_usd_balance(&mut cmd, &master);
    assert_eq!(
        balance_after,
        balance_before + amount,
        "master balance did not increase by the transferred amount"
    );
});
