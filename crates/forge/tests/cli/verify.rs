//! Contains various tests for checking forge commands related to verifying contracts on Etherscan
//! and Sourcify.

use crate::utils::{self, EnvExternalities};
use alloy_primitives::hex;
use anvil::{NodeConfig, spawn};
use axum::{Router, extract::Query};
use foundry_common::retry::Retry;
use foundry_test_utils::{
    forgetest, forgetest_async, str,
    util::{OutputExt, TestCommand, TestProject},
};
use std::{collections::HashMap, time::Duration};
use tokio::net::TcpListener;

/// Adds a `Unique` contract to the source directory of the project that can be imported as
/// `import {Unique} from "./unique.sol";`
fn add_unique(prj: &TestProject) {
    let timestamp = utils::millis_since_epoch();
    prj.add_source(
        "unique",
        &format!(
            r#"
contract Unique {{
    uint public _timestamp = {timestamp};
}}
"#
        ),
    );
}

fn add_verify_target(prj: &TestProject) {
    prj.add_source(
        "Verify.sol",
        r#"
import {Unique} from "./unique.sol";
contract Verify is Unique {
function doStuff() external {}
}
"#,
    );
}

fn add_single_verify_target_file(prj: &TestProject) {
    let timestamp = utils::millis_since_epoch();
    let contract = format!(
        r#"
contract Unique {{
    uint public _timestamp = {timestamp};
}}
contract Verify is Unique {{
function doStuff() external {{}}
}}
"#
    );

    prj.add_source("Verify.sol", &contract);
}

fn add_verify_target_with_constructor(prj: &TestProject) {
    prj.add_source(
        "Verify.sol",
        r#"
import {Unique} from "./unique.sol";
contract Verify is Unique {
    struct SomeStruct {
        uint256 a;
        string str;
    }

    constructor(SomeStruct memory st, address owner) {}
}
"#,
    );
}

fn parse_verification_result(cmd: &mut TestCommand, retries: u32) -> eyre::Result<()> {
    // Give Etherscan some time to verify the contract.
    Retry::new(retries, Duration::from_secs(30)).run(|| -> eyre::Result<()> {
        let output = cmd.execute();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        test_debug!("stdout: {stdout}\nstderr: {stderr}");
        if stderr.contains("Contract successfully verified") {
            return Ok(());
        }
        eyre::bail!("Failed to get verification, stdout: {stdout}, stderr: {stderr}")
    })
}

fn verify_check(
    guid: String,
    chain: String,
    etherscan_api_key: Option<String>,
    verifier: Option<String>,
    mut cmd: TestCommand,
) {
    let mut args = vec!["verify-check", &guid, "--chain-id", &chain];

    if let Some(etherscan_api_key) = &etherscan_api_key {
        args.push("--etherscan-api-key");
        args.push(etherscan_api_key);
    }

    if let Some(verifier) = &verifier {
        args.push("--verifier");
        args.push(verifier);
    }
    cmd.forge_fuse().args(args);

    parse_verification_result(&mut cmd, 6).expect("Failed to verify check")
}

fn await_verification_response(info: EnvExternalities, mut cmd: TestCommand) {
    let guid = {
        // Give Etherscan some time to detect the transaction.
        Retry::new(5, Duration::from_secs(60))
            .run(|| -> eyre::Result<String> {
                let output = cmd.execute();
                let out = String::from_utf8_lossy(&output.stdout);
                utils::parse_verification_guid(&out).ok_or_else(|| {
                    eyre::eyre!(
                        "Failed to get guid, stdout: {}, stderr: {}",
                        out,
                        String::from_utf8_lossy(&output.stderr)
                    )
                })
            })
            .expect("Failed to get verify guid")
    };

    // verify-check
    let etherscan = (!info.etherscan.is_empty()).then_some(info.etherscan.clone());
    let verifier = (!info.verifier.is_empty()).then_some(info.verifier.clone());
    verify_check(guid, info.chain.to_string(), etherscan, verifier, cmd);
}

fn deploy_contract(
    info: &EnvExternalities,
    contract_path: &str,
    prj: TestProject,
    cmd: &mut TestCommand,
) -> String {
    add_unique(&prj);
    add_verify_target(&prj);
    let output = cmd
        .forge_fuse()
        .arg("create")
        .args(info.create_args())
        .arg(contract_path)
        .assert_success()
        .get_output()
        .stdout_lossy();
    utils::parse_deployed_address(output.as_str())
        .unwrap_or_else(|| panic!("Failed to parse deployer {output}"))
}

fn verify_on_chain(info: Option<EnvExternalities>, prj: TestProject, mut cmd: TestCommand) {
    // only execute if keys present
    if let Some(info) = info {
        test_debug!("verifying on {}", info.chain);

        let contract_path = "src/Verify.sol:Verify";
        let address = deploy_contract(&info, contract_path, prj, &mut cmd);

        let mut args = vec![
            "--chain-id".to_string(),
            info.chain.to_string(),
            address,
            contract_path.to_string(),
        ];

        if !info.etherscan.is_empty() {
            args.push("--etherscan-api-key".to_string());
            args.push(info.etherscan.clone());
        }

        if !info.verifier.is_empty() {
            args.push("--verifier".to_string());
            args.push(info.verifier.clone());
        }
        cmd.forge_fuse().arg("verify-contract").root_arg().args(args);

        await_verification_response(info, cmd)
    }
}

fn guess_constructor_args(info: Option<EnvExternalities>, prj: TestProject, mut cmd: TestCommand) {
    // only execute if keys present
    if let Some(info) = info {
        test_debug!("verifying on {}", info.chain);
        add_unique(&prj);
        add_verify_target_with_constructor(&prj);

        let contract_path = "src/Verify.sol:Verify";
        let output = cmd
            .arg("create")
            .args(info.create_args())
            .arg(contract_path)
            .args(vec![
                "--constructor-args",
                "(239,SomeString)",
                "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
            ])
            .assert_success()
            .get_output()
            .stdout_lossy();

        let address = utils::parse_deployed_address(output.as_str())
            .unwrap_or_else(|| panic!("Failed to parse deployer {output}"));

        cmd.forge_fuse().arg("verify-contract").root_arg().args([
            "--rpc-url".to_string(),
            info.rpc.clone(),
            address,
            contract_path.to_string(),
            "--etherscan-api-key".to_string(),
            info.etherscan.clone(),
            "--verifier".to_string(),
            info.verifier.clone(),
            "--guess-constructor-args".to_string(),
        ]);

        await_verification_response(info, cmd)
    }
}

/// Executes create --verify on the given chain
fn create_verify_on_chain(info: Option<EnvExternalities>, prj: TestProject, mut cmd: TestCommand) {
    // only execute if keys present
    if let Some(info) = info {
        test_debug!("verifying on {}", info.chain);
        add_single_verify_target_file(&prj);

        let contract_path = "src/Verify.sol:Verify";
        let assert = cmd
            .arg("create")
            .args(info.create_args())
            .args([contract_path, "--etherscan-api-key", info.etherscan.as_str(), "--verify"])
            .assert_success();
        let output = assert.get_output();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("Contract successfully verified"), "stderr: {stderr}");
    }
}

// tests `create && contract-verify && verify-check` on Fantom testnet if correct env vars are set
forgetest!(can_verify_random_contract_fantom_testnet, |prj, cmd| {
    verify_on_chain(EnvExternalities::ftm_testnet(), prj, cmd);
});

// tests `create && contract-verify && verify-check` on Optimism kovan if correct env vars are set
forgetest!(can_verify_random_contract_optimism_kovan, |prj, cmd| {
    verify_on_chain(EnvExternalities::optimism_kovan(), prj, cmd);
});

// tests `create && contract-verify && verify-check` on Sepolia testnet if correct env vars are set
forgetest!(can_verify_random_contract_sepolia, |prj, cmd| {
    // Implicitly tests `--verifier etherscan` on Sepolia testnet
    verify_on_chain(EnvExternalities::sepolia_etherscan(), prj, cmd);
});

// tests that `verify-contract --verifier etherscan` also submits to Sourcify on Sepolia
forgetest!(can_verify_contract_sepolia_etherscan_also_runs_sourcify, |prj, cmd| {
    if let Some(info) = EnvExternalities::sepolia_etherscan() {
        test_debug!("verifying on {}", info.chain);
        add_unique(&prj);
        add_verify_target(&prj);
        let contract_path = "src/Verify.sol:Verify";

        let deploy_output = cmd
            .forge_fuse()
            .arg("create")
            .args(info.create_args())
            .args([contract_path, "--broadcast"])
            .assert_success()
            .get_output()
            .stdout_lossy();
        let address = utils::parse_deployed_address(deploy_output.as_str())
            .unwrap_or_else(|| panic!("Failed to parse deployer {deploy_output}"));

        let output = cmd
            .forge_fuse()
            .arg("verify-contract")
            .root_arg()
            .args([
                "--chain-id",
                info.chain.as_ref(),
                &address,
                contract_path,
                "--etherscan-api-key",
                info.etherscan.as_str(),
                "--verifier",
                info.verifier.as_str(),
            ])
            .assert_success()
            .get_output()
            .stderr_lossy();

        assert!(output.contains("Verifying on etherscan"), "Etherscan run missing: {output}");
        assert!(
            output.contains("Verification Job ID")
                || output.contains("Contract source code already fully verified"),
            "Sourcify submission did not succeed: {output}"
        );
        assert!(
            !output.contains("sourcify verification failed"),
            "Sourcify failure warning logged: {output}"
        );
    }
});

// tests `create --verify on Sepolia testnet if correct env vars are set
// SEPOLIA_RPC_URL=https://rpc.sepolia.org
// TEST_PRIVATE_KEY=0x...
// ETHERSCAN_API_KEY=<API_KEY>
forgetest!(can_create_verify_random_contract_sepolia_etherscan, |prj, cmd| {
    // Implicitly tests `--verifier etherscan` on Sepolia testnet
    create_verify_on_chain(EnvExternalities::sepolia_etherscan(), prj, cmd);
});

// tests that `create --verify --verifier etherscan` also submits to Sourcify on Sepolia
forgetest!(can_create_verify_sepolia_etherscan_also_runs_sourcify, |prj, cmd| {
    if let Some(info) = EnvExternalities::sepolia_etherscan() {
        test_debug!("verifying on {}", info.chain);
        add_single_verify_target_file(&prj);

        let contract_path = "src/Verify.sol:Verify";
        let output = cmd
            .arg("create")
            .args(info.create_args())
            .args([
                contract_path,
                "--etherscan-api-key",
                info.etherscan.as_str(),
                "--verify",
                "--broadcast",
            ])
            .assert_success()
            .get_output()
            .stderr_lossy();

        assert!(output.contains("Verifying on etherscan"), "Etherscan run missing: {output}");
        assert!(
            output.contains("Verification Job ID")
                || output.contains("Contract source code already fully verified"),
            "Sourcify submission did not succeed: {output}"
        );
        assert!(
            !output.contains("sourcify verification failed"),
            "Sourcify failure warning logged: {output}"
        );
    }
});

// tests `create --verify --verifier sourcify` on Sepolia testnet
forgetest!(can_create_verify_random_contract_sepolia_sourcify, |prj, cmd| {
    verify_on_chain(EnvExternalities::sepolia_sourcify(), prj, cmd);
});

// tests `create --verify --verifier sourcify` with etherscan api key set
// <https://github.com/foundry-rs/foundry/issues/10000>
forgetest!(
    can_create_verify_random_contract_sepolia_sourcify_with_etherscan_api_key_set,
    |prj, cmd| {
        verify_on_chain(EnvExternalities::sepolia_sourcify_with_etherscan_api_key_set(), prj, cmd);
    }
);

// tests `create --verify --verifier blockscout` on Sepolia testnet
forgetest!(can_create_verify_random_contract_sepolia_blockscout, |prj, cmd| {
    verify_on_chain(EnvExternalities::sepolia_blockscout(), prj, cmd);
});

// tests `create --verify --verifier blockscout` on Sepolia testnet with etherscan api key set
forgetest!(
    can_create_verify_random_contract_sepolia_blockscout_with_etherscan_api_key_set,
    |prj, cmd| {
        verify_on_chain(
            EnvExternalities::sepolia_blockscout_with_etherscan_api_key_set(),
            prj,
            cmd,
        );
    }
);

// tests `create && contract-verify --guess-constructor-args && verify-check` on Goerli testnet if
// correct env vars are set
forgetest!(can_guess_constructor_args, |prj, cmd| {
    guess_constructor_args(EnvExternalities::goerli(), prj, cmd);
});

// tests `create && verify-contract && verify-check` on sepolia with default sourcify verifier
forgetest!(can_verify_random_contract_sepolia_default_sourcify, |prj, cmd| {
    verify_on_chain(EnvExternalities::sepolia_empty_verifier(), prj, cmd);
});

// Tests that verify properly validates verifier arguments.
// <https://github.com/foundry-rs/foundry/issues/11430>
forgetest_init!(can_validate_verifier_settings, |prj, cmd| {
    prj.initialize_default_contracts();
    // Build the project to create the cache.
    cmd.forge_fuse().arg("build").assert_success();
    // No verifier URL.
    cmd.forge_fuse()
        .args([
            "verify-contract",
            "--rpc-url",
            "https://rpc.sepolia-api.lisk.com",
            "--verifier",
            "blockscout",
            "0x19b248616E4964f43F611b5871CE1250f360E9d3",
            "src/Counter.sol:Counter",
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Start verifying contract `0x19b248616E4964f43F611b5871CE1250f360E9d3` deployed on 4202
Error: No verifier URL specified for verifier blockscout

"#]]);

    // Unknown Etherscan chain.
    cmd.forge_fuse()
        .args([
            "verify-contract",
            "--rpc-url",
            "https://rpc.sepolia-api.lisk.com",
            "--verifier",
            "etherscan",
            "0x19b248616E4964f43F611b5871CE1250f360E9d3",
            "src/Counter.sol:Counter",
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Start verifying contract `0x19b248616E4964f43F611b5871CE1250f360E9d3` deployed on 4202
Error: No known Etherscan API URL for chain `4202`. To fix this, please:
1. Specify a `url` when using Etherscan verifier
2. Verify the chain `4202` is correct

"#]]);

    cmd.forge_fuse()
        .args([
            "verify-contract",
            "--rpc-url",
            "https://rpc.sepolia-api.lisk.com",
            "--verifier",
            "blockscout",
            "--verifier-url",
            "https://sepolia-blockscout.lisk.com/api",
            "0x19b248616E4964f43F611b5871CE1250f360E9d3",
            "src/Counter.sol:Counter",
        ])
        .assert_success()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Start verifying contract `0x19b248616E4964f43F611b5871CE1250f360E9d3` deployed on 4202

Verifying on blockscout...
Contract [src/Counter.sol:Counter] "0x19b248616E4964f43F611b5871CE1250f360E9d3" is already verified. Skipping verification.

"#]]);
});

// Tests that `forge script --broadcast --verify` fails before broadcasting when
// the verifier rejects the API key (credential preflight check).
forgetest_async!(script_fails_early_on_bad_verifier_credentials, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());
    prj.add_script(
        "Deploy.s.sol",
        r#"
import "forge-std/Script.sol";
contract Noop {}
contract Deploy is Script {
    function run() external {
        vm.startBroadcast();
        new Noop();
        vm.stopBroadcast();
    }
}
"#,
    );

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    let (verifier_url, _server) =
        spawn_mock_verifier(r#"{"status":"0","message":"NOTOK","result":"Invalid API Key"}"#).await;

    let output = cmd
        .forge_fuse()
        .args([
            "script",
            "script/Deploy.s.sol:Deploy",
            "--rpc-url",
            handle.http_endpoint().as_str(),
            "--private-key",
            pk.as_str(),
            "--broadcast",
            "--verify",
            "--verifier",
            "custom",
            "--verifier-url",
            verifier_url.as_str(),
            "--verifier-api-key",
            "FAKE_KEY_1234",
        ])
        .execute();

    assert!(!output.status.success(), "expected command to fail");
    let stderr = output.stderr_lossy();
    assert!(
        stderr.contains("Verification preflight check failed"),
        "expected preflight error in stderr, got: {stderr}"
    );
    // The broadcast phase prints "ONCHAIN EXECUTION COMPLETE" and "Sending transactions";
    // neither must appear if the preflight check stopped execution before broadcasting.
    let stdout = output.stdout_lossy();
    assert!(
        !stdout.contains("ONCHAIN EXECUTION COMPLETE") && !stdout.contains("Sending transactions"),
        "transactions were broadcast but preflight check should have prevented it: {stdout}"
    );
});

/// Spawns a local HTTP server that returns the given body for Etherscan-style ABI requests.
async fn spawn_mock_verifier(body: &'static str) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app =
        Router::new().fallback(move |Query(query): Query<HashMap<String, String>>| async move {
            if query.get("module").is_some_and(|value| value == "contract")
                && query.get("action").is_some_and(|value| value == "getabi")
                && query.contains_key("address")
                && query.contains_key("apikey")
            {
                body
            } else {
                r#"{"status":"0","message":"NOTOK","result":"Contract source code not verified"}"#
            }
        });
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), handle)
}

/// Spawns a local HTTP server mocking the full Etherscan verification flow: the `getabi` preflight
/// returns "not verified", `verifysourcecode` returns a submission GUID, and `checkverifystatus`
/// reports success. Returns the server URL and the GUID it emits.
async fn spawn_full_mock_verifier() -> (String, &'static str, tokio::task::JoinHandle<()>) {
    const GUID: &str = "mockguid1976000000000000000000000000000000000000000";
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(move |uri: axum::http::Uri, body: String| async move {
        let combined = format!("{}&{body}", uri.query().unwrap_or_default());
        if combined.contains("verifysourcecode") {
            format!(r#"{{"status":"1","message":"OK","result":"{GUID}"}}"#)
        } else if combined.contains("checkverifystatus") {
            r#"{"status":"1","message":"OK","result":"Pass - Verified"}"#.to_string()
        } else {
            r#"{"status":"0","message":"NOTOK","result":"Contract source code not verified"}"#
                .to_string()
        }
    });
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), GUID, handle)
}

// Tests that `forge create --broadcast --verify --json` keeps stdout clean (valid JSON only) and
// does not leak the verification submission GUID/URL into stdout, while still reporting it on
// stderr. <https://github.com/foundry-rs/foundry/issues/1976>
forgetest_async!(create_verify_json_keeps_stdout_clean, |prj, cmd| {
    prj.initialize_default_contracts();
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    let (verifier_url, guid, _server) = spawn_full_mock_verifier().await;

    let output = cmd
        .forge_fuse()
        .args([
            "create",
            "src/Counter.sol:Counter",
            "--rpc-url",
            handle.http_endpoint().as_str(),
            "--private-key",
            pk.as_str(),
            "--broadcast",
            "--verify",
            "--verifier",
            "custom",
            "--verifier-url",
            verifier_url.as_str(),
            "--verifier-api-key",
            "VALID_KEY",
            "--json",
        ])
        .execute();

    assert!(output.status.success(), "expected command to succeed");

    let stdout = output.stdout_lossy();
    // stdout must be a single valid JSON document (the deployment result).
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}): {stdout}"));
    // The verification submission GUID must not pollute stdout.
    assert!(!stdout.contains(guid), "verification GUID leaked into stdout: {stdout}");

    // The GUID/URL is still reported to the user on stderr.
    let stderr = output.stderr_lossy();
    assert!(stderr.contains(guid), "expected verification GUID on stderr, got: {stderr}");
});

// Tests that `forge script --broadcast --verify --json` keeps stdout clean (valid JSON Lines only)
// and does not leak the verification submission GUID/URL into stdout, while still reporting it on
// stderr. <https://github.com/foundry-rs/foundry/issues/1976>
forgetest_async!(script_verify_json_keeps_stdout_clean, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());
    prj.add_script(
        "Deploy.s.sol",
        r#"
import "forge-std/Script.sol";
contract Noop {}
contract Deploy is Script {
    function run() external {
        vm.startBroadcast();
        new Noop();
        vm.stopBroadcast();
    }
}
"#,
    );

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    let (verifier_url, guid, _server) = spawn_full_mock_verifier().await;

    let output = cmd
        .forge_fuse()
        .args([
            "script",
            "script/Deploy.s.sol:Deploy",
            "--rpc-url",
            handle.http_endpoint().as_str(),
            "--private-key",
            pk.as_str(),
            "--broadcast",
            "--verify",
            "--verifier",
            "custom",
            "--verifier-url",
            verifier_url.as_str(),
            "--verifier-api-key",
            "VALID_KEY",
            "--json",
        ])
        .execute();

    assert!(output.status.success(), "expected command to succeed");

    let stdout = output.stdout_lossy();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|e| panic!("stdout line is not valid JSON ({e}): {line}"));
    }
    assert!(!stdout.contains(guid), "verification GUID leaked into stdout: {stdout}");

    let stderr = output.stderr_lossy();
    assert!(stderr.contains(guid), "expected verification GUID on stderr, got: {stderr}");
});

// Tests that the preflight check passes (does not block deploy) when the verifier responds
// with ContractCodeNotVerified (the normal "valid key, unknown address" response).
forgetest_async!(create_preflight_passes_on_contract_not_verified, |prj, cmd| {
    prj.initialize_default_contracts();
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    // Server returns a well-formed "source code not verified" Etherscan response.
    let (verifier_url, _server) = spawn_mock_verifier(
        r#"{"status":"0","message":"NOTOK","result":"Contract source code not verified"}"#,
    )
    .await;

    let output = cmd
        .forge_fuse()
        .args([
            "create",
            "src/Counter.sol:Counter",
            "--rpc-url",
            handle.http_endpoint().as_str(),
            "--private-key",
            pk.as_str(),
            "--verify",
            "--verifier",
            "custom",
            "--verifier-url",
            verifier_url.as_str(),
            "--verifier-api-key",
            "VALID_KEY",
        ])
        .execute();

    // Preflight must pass — the command may fail for other reasons (e.g. post-deploy
    // verification), but it must NOT fail with the preflight error.
    let stderr = output.stderr_lossy();
    assert!(
        !stderr.contains("Verification preflight check failed"),
        "preflight should not block on ContractCodeNotVerified, got: {stderr}"
    );
});

// Tests that the preflight check fails (blocks deploy) when the verifier explicitly
// rejects the API key with an InvalidApiKey response.
forgetest_async!(create_preflight_fails_on_invalid_api_key, |prj, cmd| {
    prj.initialize_default_contracts();
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    // Server returns a well-formed "invalid API key" Etherscan response.
    let (verifier_url, _server) =
        spawn_mock_verifier(r#"{"status":"0","message":"NOTOK","result":"Invalid API Key"}"#).await;

    let output = cmd
        .forge_fuse()
        .args([
            "create",
            "src/Counter.sol:Counter",
            "--rpc-url",
            handle.http_endpoint().as_str(),
            "--private-key",
            pk.as_str(),
            "--verify",
            "--verifier",
            "custom",
            "--verifier-url",
            verifier_url.as_str(),
            "--verifier-api-key",
            "BAD_KEY",
        ])
        .execute();

    assert!(!output.status.success(), "expected command to fail");
    let stderr = output.stderr_lossy();
    assert!(
        stderr.contains("Verification preflight check failed"),
        "expected preflight error in stderr, got: {stderr}"
    );
    let stdout = output.stdout_lossy();
    assert!(
        !stdout.contains("Contract Address"),
        "contract was deployed but preflight check should have prevented it"
    );
});

// Tests that the preflight check does NOT block deployment when the verifier responds
// with a rate-limit error (transient, not an auth failure).
forgetest_async!(create_preflight_warns_on_rate_limit, |prj, cmd| {
    prj.initialize_default_contracts();
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    // Server returns a well-formed "rate limit exceeded" Etherscan response.
    let (verifier_url, _server) = spawn_mock_verifier(
        r#"{"status":"0","message":"NOTOK","result":"Max rate limit reached"}"#,
    )
    .await;

    let output = cmd
        .forge_fuse()
        .args([
            "create",
            "src/Counter.sol:Counter",
            "--rpc-url",
            handle.http_endpoint().as_str(),
            "--private-key",
            pk.as_str(),
            "--verify",
            "--verifier",
            "blockscout",
            "--verifier-url",
            verifier_url.as_str(),
            "--verifier-api-key",
            "VALID_KEY",
        ])
        .execute();

    // Rate limit must not block the deploy.
    let stderr = output.stderr_lossy();
    assert!(
        !stderr.contains("Verification preflight check failed"),
        "preflight should not block on rate limit, got: {stderr}"
    );
    assert!(
        stderr.contains("verifier credential check inconclusive"),
        "preflight should warn on rate limit, got: {stderr}"
    );
});
