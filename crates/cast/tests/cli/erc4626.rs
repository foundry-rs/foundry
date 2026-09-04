//! End-to-end tests for `cast erc4626`.

use alloy_primitives::U256;
use anvil::{NodeConfig, NodeHandle};
use foundry_test_utils::{rpc::next_http_archive_rpc_url, str, util::OutputExt};

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

fn assert_inspection_surface(
    cmd: &mut foundry_test_utils::TestCommand,
    vault: &ProductionVault,
    rpc: &str,
) {
    for (command, args) in
        [("info", Vec::new()), ("position", vec![anvil_const::ADDR1]), ("check", Vec::new())]
    {
        let output = cmd
            .cast_fuse()
            .args(["erc4626", command, vault.address])
            .args(args)
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
    deploy_test_contract(cmd, rpc, private_key, "TestVault");
}

fn deploy_test_contract(
    cmd: &mut foundry_test_utils::TestCommand,
    rpc: &str,
    private_key: &str,
    contract: &str,
) {
    cmd.args([
        "create",
        "--private-key",
        private_key,
        "--rpc-url",
        rpc,
        "--broadcast",
        &format!("src/TestVault.sol:{contract}"),
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

    cmd.cast_fuse()
        .args(["erc4626", "info", anvil_const::VAULT, "--human", "--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(format!(
            "Vault                {}\n\
             Name                 Test Vault\n\
             Symbol               TV\n\
             Decimals             18\n\
             Asset                {asset}\n\
             Asset name           Test Vault Asset\n\
             Asset symbol         TVA\n\
             Asset decimals       18\n\
             Total assets         0.000000000000000100 TVA\n\
             Total supply         0.000000000000000100 TV\n\
             Assets per share     1 TVA\n\
             Shares per asset     1 TV\n",
            anvil_const::VAULT
        ));

    cmd.cast_fuse()
        .args(["erc4626", "info", anvil_const::VAULT, "--json", "--rpc-url", &rpc])
        .assert_json_stdout(format!(
            r#"{{
                "schema_version": 1,
                "success": true,
                "data": {{
                    "vault": "{}",
                    "name": "Test Vault",
                    "symbol": "TV",
                    "decimals": 18,
                    "asset": "{asset}",
                    "asset_name": "Test Vault Asset",
                    "asset_symbol": "TVA",
                    "asset_decimals": 18,
                    "total_assets": {{
                        "raw": "100",
                        "formatted": "0.000000000000000100"
                    }},
                    "total_supply": {{
                        "raw": "100",
                        "formatted": "0.000000000000000100"
                    }},
                    "assets_per_share": {{ "raw": "1000000000000000000", "formatted": "1" }},
                    "shares_per_asset": {{ "raw": "1000000000000000000", "formatted": "1" }}
                }},
                "errors": [],
                "warnings": []
            }}"#,
            anvil_const::VAULT
        ));

    cmd.cast_fuse()
        .args([
            "erc4626",
            "position",
            anvil_const::VAULT,
            anvil_const::ADDR1,
            "--json",
            "--rpc-url",
            &rpc,
        ])
        .assert_json_stdout(format!(
            r#"{{
                "schema_version": 1,
                "success": true,
                "data": {{
                    "vault": "{}",
                    "owner": "{}",
                    "asset": "{asset}",
                    "share_symbol": "TV",
                    "share_decimals": 18,
                    "asset_symbol": "TVA",
                    "asset_decimals": 18,
                    "share_balance": {{
                        "raw": "100",
                        "formatted": "0.000000000000000100"
                    }},
                    "assets_equivalent": {{
                        "raw": "100",
                        "formatted": "0.000000000000000100"
                    }},
                    "max_withdraw": {{ "raw": "0", "formatted": "0" }},
                    "max_redeem": {{ "raw": "0", "formatted": "0" }}
                }},
                "errors": [],
                "warnings": [
                    {{
                        "level": "warning",
                        "code": "erc4626_zero_max_withdraw",
                        "message": "Vault reported zero from maxWithdraw even though the owner has shares; liquidity, gates, withdrawal queues, or a conservative implementation may prevent the base ERC-4626 exit."
                    }},
                    {{
                        "level": "warning",
                        "code": "erc4626_zero_max_redeem",
                        "message": "Vault reported zero from maxRedeem even though the owner has shares; liquidity, gates, withdrawal queues, or a conservative implementation may prevent the base ERC-4626 exit."
                    }}
                ]
            }}"#,
            anvil_const::VAULT,
            anvil_const::ADDR1
        ));

    cmd.cast_fuse()
        .args(["erc4626", "check", anvil_const::VAULT, "--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(format!(
            "Vault                {}\n\
             Account              0x0000000000000000000000000000000000000000\n\
             Note: This probes read-call behavior only; it does not prove state-changing selector coverage or semantic ERC-4626 compliance.\n\
             PASS contract code            contract bytecode is present\n\
             PASS asset()                  returned {asset}\n\
             PASS asset contract           underlying asset bytecode is present\n\
             PASS asset balanceOf(address) call succeeded\n\
             PASS totalAssets()            call succeeded\n\
             PASS totalSupply()            call succeeded\n\
             PASS balanceOf(address)       call succeeded\n\
             PASS allowance(address,address) call succeeded\n\
             PASS convertToShares(0)       returned zero\n\
             PASS convertToAssets(0)       returned zero\n\
             PASS maxDeposit(address)      call succeeded\n\
             PASS previewDeposit(0)        returned zero\n\
             PASS maxMint(address)         call succeeded\n\
             PASS previewMint(0)           returned zero\n\
             PASS maxWithdraw(address)     call succeeded\n\
             PASS previewWithdraw(0)       returned zero\n\
             PASS maxRedeem(address)       call succeeded\n\
             PASS previewRedeem(0)         returned zero\n\
             PASS name()                   call succeeded\n\
             PASS symbol()                 call succeeded\n\
             PASS decimals()               call succeeded\n\
             Summary: 21 passed, 0 warnings, 0 failed\n",
            anvil_const::VAULT
        ));
});

forgetest_async!(erc4626_check_warns_for_known_extensions, |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source("TestVault.sol", include_str!("../fixtures/TestVault.sol"));
    deploy_test_contract(&mut cmd, &rpc, anvil_const::PK1, "TestAsyncVault");

    cmd.cast_fuse()
        .args(["erc4626", "info", anvil_const::VAULT, "--human", "--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(str![[r#"
Vault                0x5FbDB2315678afecb367f032d93F642f64180aa3
Name                 Test Async Vault
Symbol               TAV
Decimals             18
Asset                0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE
Asset name           <unavailable>
Asset symbol         <unavailable>
Asset decimals       18
Total assets         1
Total supply         0 TAV
Assets per share     1
Shares per asset     1 TAV

"#]]);

    let output = cmd
        .cast_fuse()
        .args([
            "erc4626",
            "position",
            anvil_const::VAULT,
            anvil_const::ADDR1,
            "--json",
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .get_output()
        .stdout
        .clone();
    let output: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(output["data"]["asset_decimals"], 18);
    assert_eq!(output["data"]["assets_equivalent"]["formatted"], "0");

    cmd.cast_fuse()
        .args(["erc4626", "check", anvil_const::VAULT, "--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(str![[r#"
Vault                0x5FbDB2315678afecb367f032d93F642f64180aa3
Account              0x0000000000000000000000000000000000000000
Note: This probes read-call behavior only; it does not prove state-changing selector coverage or semantic ERC-4626 compliance.
PASS contract code            contract bytecode is present
WARN asset()                  returned the ERC-7535 native-asset sentinel
PASS totalAssets()            call succeeded
PASS totalSupply()            call succeeded
PASS balanceOf(address)       call succeeded
PASS allowance(address,address) call succeeded
PASS convertToShares(0)       returned zero
PASS convertToAssets(0)       returned zero
PASS maxDeposit(address)      call succeeded
WARN previewDeposit(0)        reverted as required by advertised asynchronous ERC-7540 deposit support
PASS maxMint(address)         call succeeded
WARN previewMint(0)           reverted as required by advertised asynchronous ERC-7540 deposit support
PASS maxWithdraw(address)     call succeeded
WARN previewWithdraw(0)       reverted as required by advertised asynchronous ERC-7540 redeem support
PASS maxRedeem(address)       call succeeded
WARN previewRedeem(0)         reverted as required by advertised asynchronous ERC-7540 redeem support
PASS name()                   call succeeded
PASS symbol()                 call succeeded
PASS decimals()               call succeeded
Summary: 14 passed, 5 warnings, 0 failed

"#]]);
});

forgetest_async!(erc4626_check_fails_for_missing_metadata, |prj, cmd| {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();

    foundry_test_utils::util::initialize(prj.root());
    prj.add_source("TestVault.sol", include_str!("../fixtures/TestVault.sol"));
    deploy_test_contract(&mut cmd, &rpc, anvil_const::PK1, "TestMissingMetadataVault");

    cmd.cast_fuse()
        .args(["erc4626", "check", anvil_const::VAULT, "--rpc-url", &rpc])
        .assert_failure()
        .stdout_eq(str![[r#"
Vault                0x5FbDB2315678afecb367f032d93F642f64180aa3
Account              0x0000000000000000000000000000000000000000
Note: This probes read-call behavior only; it does not prove state-changing selector coverage or semantic ERC-4626 compliance.
PASS contract code            contract bytecode is present
WARN asset()                  returned the ERC-7535 native-asset sentinel
PASS totalAssets()            call succeeded
PASS totalSupply()            call succeeded
PASS balanceOf(address)       call succeeded
PASS allowance(address,address) call succeeded
PASS convertToShares(0)       returned zero
PASS convertToAssets(0)       returned zero
PASS maxDeposit(address)      call succeeded
WARN previewDeposit(0)        reverted as required by advertised asynchronous ERC-7540 deposit support
PASS maxMint(address)         call succeeded
WARN previewMint(0)           reverted as required by advertised asynchronous ERC-7540 deposit support
PASS maxWithdraw(address)     call succeeded
WARN previewWithdraw(0)       reverted as required by advertised asynchronous ERC-7540 redeem support
PASS maxRedeem(address)       call succeeded
WARN previewRedeem(0)         reverted as required by advertised asynchronous ERC-7540 redeem support
FAIL name()                   call failed or returned incompatible data
FAIL symbol()                 call failed or returned incompatible data
FAIL decimals()               call failed or returned incompatible data
Summary: 11 passed, 5 warnings, 3 failed

"#]]);

    let output = cmd
        .cast_fuse()
        .args(["erc4626", "check", anvil_const::VAULT, "--rpc-url", &rpc, "--json"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let output: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(output["success"], false);
    assert_eq!(output["data"]["read_compatible"], false);
    assert_eq!(output["data"]["failed"], 3);
    assert_eq!(output["errors"][0]["code"], "erc4626.compatibility_failed");
});

casttest!(erc4626_fork_reads_multiple_production_vaults, async |_prj, cmd| {
    let fork = NodeConfig::test()
        .with_eth_rpc_url(Some(next_http_archive_rpc_url()))
        .with_fork_block_number(Some(ETHEREUM_FORK_BLOCK));
    let (_, handle) = anvil::spawn(fork).await;
    let rpc = handle.http_endpoint();

    for vault in PRODUCTION_VAULTS {
        assert_read_surface(&mut cmd, vault, &rpc);
        assert_inspection_surface(&mut cmd, vault, &rpc);
    }
});

casttest!(flaky_erc4626_fork_reads_tempo_vault, async |_prj, cmd| {
    let fork = NodeConfig::test_tempo()
        .with_eth_rpc_url(Some(TEMPO_RPC_URL.to_string()))
        .with_fork_block_number(Some(TEMPO_FORK_BLOCK));
    let (_, handle) = anvil::spawn(fork).await;
    let rpc = handle.http_endpoint();

    assert_read_surface(&mut cmd, &TEMPO_VAULT, &rpc);
    assert_inspection_surface(&mut cmd, &TEMPO_VAULT, &rpc);
});
