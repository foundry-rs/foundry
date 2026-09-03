//! End-to-end tests for `cast erc4626`.

use alloy_primitives::U256;
use anvil::{NodeConfig, NodeHandle};
use foundry_test_utils::{rpc::next_http_archive_rpc_url, util::OutputExt};

mod anvil_const {
    pub const PK1: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    pub const ADDR1: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    pub const ADDR2: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

    /// Contract address deployed by `ADDR1` at nonce zero.
    pub const VAULT: &str = "0x5FbDB2315678afecb367f032d93F642f64180aa3";
}

const ETHEREUM_FORK_BLOCK: u64 = 25_519_075;
const TEMPO_FORK_BLOCK: u64 = 37_847_799;
const TEMPO_RPC_URL: &str = "https://rpc.tempo.xyz";

struct ProductionVault {
    project: &'static str,
    address: &'static str,
}

const PRODUCTION_VAULTS: &[ProductionVault] = &[
    ProductionVault {
        project: "Morpho MetaMorpho",
        address: "0xBEEF01735c132Ada46AA9aA4c54623cAA92A64CB",
    },
    ProductionVault { project: "Yearn V3", address: "0x028eC7330ff87667b6dfb0D94b954c820195336c" },
    ProductionVault {
        project: "Maple syrupUSDC",
        address: "0x80ac24aA929eaF5013f6436cdA2a7ba190f5Cc0b",
    },
];

const TEMPO_VAULT: ProductionVault = ProductionVault {
    project: "Morpho on Tempo",
    address: "0x83a1491f3e7f8dAAB8F787a631334b9ca7a87023",
};

const READ_CALLS: &[(&str, &[&str])] = &[
    ("asset", &[]),
    ("total-assets", &[]),
    ("convert-to-shares", &["1"]),
    ("convert-to-assets", &["1"]),
    ("max-deposit", &[anvil_const::ADDR1]),
    ("preview-deposit", &["1"]),
    ("max-mint", &[anvil_const::ADDR1]),
    ("preview-mint", &["1"]),
    ("max-withdraw", &[anvil_const::ADDR1]),
    ("preview-withdraw", &["1"]),
    ("max-redeem", &[anvil_const::ADDR1]),
    ("preview-redeem", &["1"]),
];

fn assert_read_surface(
    cmd: &mut foundry_test_utils::TestCommand,
    vault: &ProductionVault,
    rpc: &str,
) {
    for (command, args) in READ_CALLS {
        let output = cmd
            .cast_fuse()
            .args(["erc4626", command, vault.address])
            .args(*args)
            .args(["--rpc-url", rpc])
            .assert_success()
            .get_output()
            .stdout_lossy();
        assert!(!output.trim().is_empty(), "{} {command} returned no output", vault.project);
    }
}

fn read_amount(
    cmd: &mut foundry_test_utils::TestCommand,
    command: &str,
    vault: &str,
    args: &[&str],
    rpc: &str,
) -> U256 {
    let output = cmd
        .cast_fuse()
        .args(["erc4626", command, vault])
        .args(args)
        .args(["--rpc-url", rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    output.split_whitespace().next().unwrap().parse().unwrap()
}

fn read_erc20_balance(
    cmd: &mut foundry_test_utils::TestCommand,
    token: &str,
    owner: &str,
    rpc: &str,
) -> U256 {
    let output = cmd
        .cast_fuse()
        .args(["erc20", "balance", token, owner, "--rpc-url", rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    output.split_whitespace().next().unwrap().parse().unwrap()
}

fn deploy_test_vault(cmd: &mut foundry_test_utils::TestCommand, rpc: &str, private_key: &str) {
    cmd.args([
        "create",
        "--private-key",
        private_key,
        "--rpc-url",
        rpc,
        "--broadcast",
        "src/TestVault.sol:TestVault",
    ])
    .assert_success();
}

async fn setup_test_vault(
    prj: &foundry_test_utils::TestProject,
    cmd: &mut foundry_test_utils::TestCommand,
) -> (String, NodeHandle) {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source("TestVault.sol", include_str!("../fixtures/TestVault.sol"));
    deploy_test_vault(cmd, &rpc, anvil_const::PK1);

    (rpc, handle)
}

forgetest_async!(erc4626_complete_synchronous_interface, |prj, cmd| {
    let (rpc, _handle) = setup_test_vault(&prj, &mut cmd).await;

    let asset = cmd
        .cast_fuse()
        .args(["vault", "asset", anvil_const::VAULT, "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    assert_eq!(read_amount(&mut cmd, "total-assets", anvil_const::VAULT, &[], &rpc), U256::ZERO);
    assert_eq!(
        read_amount(&mut cmd, "convert-to-shares", anvil_const::VAULT, &["100"], &rpc),
        U256::from(100)
    );
    assert_eq!(
        read_amount(&mut cmd, "convert-to-assets", anvil_const::VAULT, &["100"], &rpc),
        U256::from(100)
    );

    let max_deposit = cmd
        .cast_fuse()
        .args(["erc4626", "max-deposit", anvil_const::VAULT, anvil_const::ADDR1, "--rpc-url", &rpc])
        .assert_success();
    assert!(max_deposit.get_output().stderr_lossy().contains("conservative maxima"));
    assert_eq!(
        read_amount(&mut cmd, "preview-deposit", anvil_const::VAULT, &["100"], &rpc),
        U256::from(100)
    );

    let max_mint = cmd
        .cast_fuse()
        .args(["erc4626", "max-mint", anvil_const::VAULT, anvil_const::ADDR1, "--rpc-url", &rpc])
        .assert_success();
    assert!(max_mint.get_output().stderr_lossy().contains("conservative maxima"));
    assert_eq!(
        read_amount(&mut cmd, "preview-mint", anvil_const::VAULT, &["50"], &rpc),
        U256::from(50)
    );

    cmd.cast_fuse()
        .args([
            "erc20",
            "approve",
            &asset,
            anvil_const::VAULT,
            "1000",
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
        ])
        .assert_success();

    cmd.cast_fuse()
        .args([
            "erc4626",
            "deposit",
            anvil_const::VAULT,
            "100",
            anvil_const::ADDR1,
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
        ])
        .assert_success();
    cmd.cast_fuse()
        .args([
            "erc4626",
            "mint",
            anvil_const::VAULT,
            "50",
            anvil_const::ADDR1,
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
        ])
        .assert_success();

    let max_withdraw = cmd
        .cast_fuse()
        .args([
            "erc4626",
            "max-withdraw",
            anvil_const::VAULT,
            anvil_const::ADDR1,
            "--rpc-url",
            &rpc,
        ])
        .assert_success();
    assert!(max_withdraw.get_output().stderr_lossy().contains("even though the owner has shares"));
    assert_eq!(
        read_amount(&mut cmd, "preview-withdraw", anvil_const::VAULT, &["25"], &rpc),
        U256::from(25)
    );

    let max_redeem = cmd
        .cast_fuse()
        .args(["erc4626", "max-redeem", anvil_const::VAULT, anvil_const::ADDR1, "--rpc-url", &rpc])
        .assert_success();
    assert!(max_redeem.get_output().stderr_lossy().contains("even though the owner has shares"));
    assert_eq!(
        read_amount(&mut cmd, "preview-redeem", anvil_const::VAULT, &["25"], &rpc),
        U256::from(25)
    );

    cmd.cast_fuse()
        .args([
            "erc4626",
            "withdraw",
            anvil_const::VAULT,
            "25",
            anvil_const::ADDR2,
            anvil_const::ADDR1,
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
        ])
        .assert_success();
    cmd.cast_fuse()
        .args([
            "erc4626",
            "redeem",
            anvil_const::VAULT,
            "25",
            anvil_const::ADDR2,
            anvil_const::ADDR1,
            "--rpc-url",
            &rpc,
            "--private-key",
            anvil_const::PK1,
        ])
        .assert_success();

    assert_eq!(
        read_amount(&mut cmd, "total-assets", anvil_const::VAULT, &[], &rpc),
        U256::from(100)
    );
    assert_eq!(
        read_erc20_balance(&mut cmd, anvil_const::VAULT, anvil_const::ADDR1, &rpc),
        U256::from(100)
    );
    assert_eq!(read_erc20_balance(&mut cmd, &asset, anvil_const::ADDR2, &rpc), U256::from(50));
});

casttest!(erc4626_fork_reads_multiple_production_vaults, async |_prj, cmd| {
    let fork = NodeConfig::test()
        .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
        .with_fork_block_number(Some(ETHEREUM_FORK_BLOCK));
    let (_, handle) = anvil::spawn(fork).await;
    let rpc = handle.http_endpoint();

    for vault in PRODUCTION_VAULTS {
        assert_read_surface(&mut cmd, vault, &rpc);
    }
});

casttest!(flaky_erc4626_fork_reads_tempo_vault, async |_prj, cmd| {
    let fork = NodeConfig::test_tempo()
        .with_eth_rpc_url(Some(TEMPO_RPC_URL.to_string()))
        .with_fork_block_number(Some(TEMPO_FORK_BLOCK));
    let (_, handle) = anvil::spawn(fork).await;
    let rpc = handle.http_endpoint();

    assert_read_surface(&mut cmd, &TEMPO_VAULT, &rpc);
});
