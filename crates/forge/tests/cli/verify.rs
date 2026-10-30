//! Contains various tests for checking forge commands related to verifying contracts on Etherscan
//! and Sourcify.

use crate::utils::{self, EnvExternalities};
use alloy_chains::NamedChain;
use alloy_network::Ethereum;
use alloy_primitives::{Address, U256, address, hex};
use anvil::{NodeConfig, spawn};
use axum::{
    Form, Router,
    extract::Query,
    http::{StatusCode, header},
    response::IntoResponse,
    routing::post,
};
use forge_script_sequence::ScriptSequence;
use foundry_cli::utils::parse_json;
use foundry_common::retry::Retry;
use foundry_compilers::PathStyle;
use foundry_evm::traces::CallKind;
use foundry_test_utils::{
    forgetest,
    rpc::spawn_rpc_proxy_recording_method,
    str,
    util::{OutputExt, SOLC_VERSION, TestCommand, TestProject},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::net::TcpListener;

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

fn parse_verification_result(cmd: &mut TestCommand, retries: u32) -> eyre::Result<()> {
    // Give Etherscan some time to verify the contract.
    Retry::new(retries, Duration::from_secs(30)).run(|| -> eyre::Result<()> {
        let output = cmd.execute();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        test_debug!("stdout: {stdout}\nstderr: {stderr}");
        // A retried attempt can land on a submission that already succeeded, which reports the
        // contract as verified rather than verifying it again.
        if stderr.contains("Contract successfully verified") || stderr.contains("already verified")
        {
            return Ok(());
        }
        eyre::bail!("Failed to get verification, stdout: {stdout}, stderr: {stderr}");
    })
}

/// Deploys a freshly stamped contract and verifies it, as two separate commands.
///
/// Deploy and verify are kept apart on purpose: Sourcify indexes from its own node, and submitting
/// straight after the broadcast fails the job outright with `contract_not_deployed` rather than
/// leaving it pending, so the retry has to re-run the whole `verify-contract` command.
fn deploy_and_verify_on_chain(
    info: Option<EnvExternalities>,
    prj: TestProject,
    mut cmd: TestCommand,
) {
    // only execute if the network is configured
    let Some(info) = info else { return };

    test_debug!("deploying on {} for verification via {}", info.chain, info.verifier);
    add_single_verify_target_file(&prj);

    let contract_path = "src/Verify.sol:Verify";
    let output = cmd
        .forge_fuse()
        .arg("create")
        .args(info.create_args())
        .args([contract_path, "--broadcast"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let address = utils::parse_deployed_address(output.as_str())
        .unwrap_or_else(|| panic!("Failed to parse deployer {output}"));

    let mut args = vec![
        "--chain-id".to_string(),
        (info.chain as u64).to_string(),
        address,
        contract_path.to_string(),
        "--watch".to_string(),
    ];

    if !info.etherscan.is_empty() {
        args.push("--etherscan-api-key".to_string());
        args.push(info.etherscan.clone());
    }

    if !info.verifier.is_empty() {
        args.push("--verifier".to_string());
        args.push(info.verifier.clone());
    }

    // Blockscout instances share no registry, so the URL has to be passed explicitly.
    if let Some(verifier_url) = &info.verifier_url {
        args.push("--verifier-url".to_string());
        args.push(verifier_url.clone());
    }

    cmd.forge_fuse().arg("verify-contract").root_arg().args(args);

    parse_verification_result(&mut cmd, 6)
        .unwrap_or_else(|err| panic!("Failed to verify on {}: {err}", info.chain));
}

// Blockscout instances are per-chain deployments with no shared registry, so `--verifier
// blockscout` has to be given the URL explicitly.
const SEPOLIA_BLOCKSCOUT_URL: &str = "https://eth-sepolia.blockscout.com/api";
const ROBINHOOD_TESTNET_BLOCKSCOUT_URL: &str = "https://explorer.testnet.chain.robinhood.com/api";

/// End-to-end `create` + `verify-contract` coverage across the testnets we support.
///
/// Each test is inert unless `<NETWORK>_RPC_URL` and `TESTNET_DEPLOYER_PRIVATE_KEY` are set, and
/// they are wired up by the nightly `test-deploy-verify` workflow rather than by PR CI: they spend
/// testnet funds and depend on three external verifier services.
///
/// Etherscan v2 covers Hoodi, Sepolia, Base Sepolia, Arbitrum Sepolia and Monad testnet. Robinhood
/// testnet is not on the v2 chainlist, so it is covered by Sourcify and its own Blockscout instance
/// instead. HyperEVM testnet (998) is covered by Sourcify.
macro_rules! deploy_verify_tests {
    ($($name:ident: $chain:expr, $network:literal, $verifier:literal, $url:expr;)*) => {$(
        #[forgetest]
        fn $name(prj: _, cmd: _) {
            deploy_and_verify_on_chain(
                EnvExternalities::deploy_verify($chain, $network, $verifier, $url),
                prj,
                cmd,
            );
        }
    )*};
}

deploy_verify_tests! {
    deploy_verify_hyperevm_testnet_sourcify: NamedChain::HyperliquidTestnet, "hyperevm-testnet", "sourcify", None;

    deploy_verify_hoodi_etherscan: NamedChain::Hoodi, "hoodi", "etherscan", None;
    deploy_verify_hoodi_sourcify: NamedChain::Hoodi, "hoodi", "sourcify", None;

    deploy_verify_sepolia_etherscan: NamedChain::Sepolia, "sepolia", "etherscan", None;
    deploy_verify_sepolia_sourcify: NamedChain::Sepolia, "sepolia", "sourcify", None;
    deploy_verify_sepolia_blockscout:
        NamedChain::Sepolia, "sepolia", "blockscout", Some(SEPOLIA_BLOCKSCOUT_URL);

    deploy_verify_base_sepolia_etherscan:
        NamedChain::BaseSepolia, "base-sepolia", "etherscan", None;
    deploy_verify_base_sepolia_sourcify:
        NamedChain::BaseSepolia, "base-sepolia", "sourcify", None;

    deploy_verify_arbitrum_sepolia_etherscan:
        NamedChain::ArbitrumSepolia, "arbitrum-sepolia", "etherscan", None;
    deploy_verify_arbitrum_sepolia_sourcify:
        NamedChain::ArbitrumSepolia, "arbitrum-sepolia", "sourcify", None;

    deploy_verify_monad_testnet_etherscan:
        NamedChain::MonadTestnet, "monad-testnet", "etherscan", None;
    deploy_verify_monad_testnet_sourcify:
        NamedChain::MonadTestnet, "monad-testnet", "sourcify", None;

    deploy_verify_robinhood_testnet_sourcify:
        NamedChain::RobinhoodTestnet, "robinhood-testnet", "sourcify", None;
    deploy_verify_robinhood_testnet_blockscout:
        NamedChain::RobinhoodTestnet, "robinhood-testnet", "blockscout",
        Some(ROBINHOOD_TESTNET_BLOCKSCOUT_URL);
}

// Tests that verify properly validates verifier arguments.
// <https://github.com/foundry-rs/foundry/issues/11430>
#[forgetest_init]
async fn can_validate_verifier_settings(prj: _, cmd: _) {
    prj.initialize_default_contracts();
    // Build the project to create the cache.
    cmd.forge_fuse().arg("build").assert_success();
    // Argument validation should not depend on a public block explorer being available.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let verifier_url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(
            "/api",
            post(|Form(form): Form<HashMap<String, String>>| async move {
                if form.get("apikey").map(String::as_str) == Some("verifier-key") {
                    r#"{"status":"1","message":"OK","result":"Pass - Verified"}"#
                } else {
                    r#"{"status":"0","message":"NOTOK","result":"Invalid API Key"}"#
                }
            }),
        )
        .fallback(|Query(query): Query<HashMap<String, String>>| async move {
            assert_eq!(query.get("module").map(String::as_str), Some("contract"));
            assert_eq!(query.get("action").map(String::as_str), Some("getabi"));
            assert_eq!(
                query["address"].parse::<Address>().unwrap(),
                address!("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")
            );
            r#"{"status":"1","message":"OK","result":"[]"}"#
        });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // Use the explicit chain ID so validation does not depend on a public RPC endpoint.
    // No verifier URL.
    cmd.forge_fuse()
        .args([
            "verify-contract",
            "--chain-id",
            "4202",
            "--verifier",
            "blockscout",
            "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
            "src/Counter.sol:Counter",
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Start verifying contract `0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2` deployed on 4202
Error: No verifier URL specified for verifier blockscout

"#]]);

    // Unknown Etherscan chain.
    cmd.forge_fuse()
        .args([
            "verify-contract",
            "--chain-id",
            "4202",
            "--verifier",
            "etherscan",
            "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
            "src/Counter.sol:Counter",
        ])
        .assert_failure()
        .stderr_eq(str![[r#"
Start verifying contract `0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2` deployed on 4202
Error: No known Etherscan API URL for chain `4202`. To fix this, please:
1. Specify a `url` when using Etherscan verifier
2. Verify the chain `4202` is correct

"#]]);

    cmd.forge_fuse()
        .args([
            "verify-contract",
            "--chain-id",
            "4202",
            "--verifier",
            "blockscout",
            "--verifier-url",
            verifier_url.as_str(),
            "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
            "src/Counter.sol:Counter",
        ])
        .assert_success()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Start verifying contract `0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2` deployed on 4202

Verifying on blockscout...
Contract [src/Counter.sol:Counter] "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2" is already verified. Skipping verification.

"#]]);

    // Unknown Etherscan chain, explicit Blockscout verifier, and ETHERSCAN_API_KEY set.
    // This should still use the explicit verifier URL instead of trying to resolve an
    // Etherscan API URL for the chain.
    let cmd = cmd.forge_fuse();
    cmd.env("ETHERSCAN_API_KEY", "dummy");
    cmd.args([
        "verify-contract",
        "--chain-id",
        "4202",
        "--verifier",
        "blockscout",
        "--verifier-url",
        verifier_url.as_str(),
        "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
        "src/Counter.sol:Counter",
    ])
    .assert_success()
    .stdout_eq(str![""])
    .stderr_eq(str![[r#"
Start verifying contract `0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2` deployed on 4202

Verifying on blockscout...
Contract [src/Counter.sol:Counter] "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2" is already verified. Skipping verification.

"#]]);
    // Status checks prefer the verifier key over the Etherscan key.
    cmd.forge_fuse()
        .args(["verify-check", "job-id", "--verifier", "custom"])
        .args(["--verifier-url", &format!("{verifier_url}/api")])
        .args(["--verifier-api-key", "verifier-key", "--etherscan-api-key", "fallback-key"])
        .args(["--chain", "1", "--retries", "1", "--delay", "0"])
        .assert_success()
        .stdout_eq(str![""])
        .stderr_eq(str![[r#"
Checking verification status on mainnet
Contract verification status:
Response: `OK`
Details: `Pass - Verified`
Contract successfully verified

"#]]);
    server.abort();
}

// Tests that `forge script --broadcast --verify` fails before broadcasting when
// the verifier rejects the API key (credential preflight check).
#[forgetest_init]
async fn script_fails_early_on_bad_verifier_credentials(prj: _, cmd: _) {
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
}

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
#[forgetest]
async fn create_verify_json_keeps_stdout_clean(prj: _, cmd: _) {
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
}

// Tests that `forge script --broadcast --verify --json` keeps stdout clean (valid JSON Lines only)
// and does not leak the verification submission GUID/URL into stdout, while still reporting it on
// stderr. <https://github.com/foundry-rs/foundry/issues/1976>
#[forgetest_init]
async fn script_verify_json_keeps_stdout_clean(prj: _, cmd: _) {
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
}

// Regression test for <https://github.com/foundry-rs/foundry/issues/10164>. The contract created
// by the script is compiled in a different project and can only be identified from its factory's
// verified Standard JSON input.
#[forgetest_init]
async fn script_verifies_external_create2_contract(prj: _, cmd: _) {
    const SUBMISSION_KEY: &str = "submission-key";
    const GUID: &str = "external-create2-guid";

    let external = TestProject::new("external-create2", PathStyle::Dapptools);
    foundry_test_utils::util::initialize(external.root());
    external.add_source(
        "External.sol",
        r#"
contract Child { uint256 public immutable value; constructor(uint256 value_) { value = value_; } }
contract Executor {
    function deploy(bytes memory initCode) external returns (address deployed) {
        assembly { deployed := create2(0, add(initCode, 32), mload(initCode), 0) }
        require(deployed != address(0));
    }
}
contract ExternalFactory {
    Executor public immutable executor;
    constructor() { executor = new Executor(); }
    function deployChild(uint256 value) external returns (address) {
        return executor.deploy(abi.encodePacked(type(Child).creationCode, abi.encode(value)));
    }
}
"#,
    );
    external.write_config(foundry_config::Config {
        solc: Some(foundry_config::SolcReq::Version(SOLC_VERSION.parse().unwrap())),
        optimizer: Some(true),
        optimizer_runs: Some(777),
        ..Default::default()
    });

    let (_api, anvil) = spawn(NodeConfig::test()).await;
    let wallet = anvil.dev_wallets().next().unwrap();
    let private_key = hex::encode(wallet.credential().to_bytes());
    let rpc = anvil.http_endpoint();
    let create = external
        .forge_command()
        .forge_fuse()
        .args([
            "create",
            "src/External.sol:ExternalFactory",
            "--broadcast",
            "--json",
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            private_key.as_str(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let create: serde_json::Value = serde_json::from_str(&create).unwrap();
    let factory = create["deployedTo"].as_str().unwrap().to_string();
    let selector = &alloy_primitives::keccak256("executor()")[..4];
    let call: serde_json::Value = reqwest::Client::new()
        .post(rpc.as_str())
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "eth_call",
            "params": [{"to": factory, "data": hex::encode_prefixed(selector)}, "latest"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let executor_result = call["result"].as_str().unwrap();
    let executor = format!("0x{}", &executor_result[executor_result.len() - 40..]);

    let standard_json = external
        .forge_command()
        .forge_fuse()
        .args([
            "verify-contract",
            factory.as_str(),
            "src/External.sol:ExternalFactory",
            "--show-standard-json-input",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let standard_json: serde_json::Value = serde_json::from_str(standard_json.trim()).unwrap();
    assert_eq!(standard_json["settings"]["optimizer"]["runs"], 777);

    // Use the compiler build recorded by Forge's artifact, not a guessed release suffix.
    let artifact: serde_json::Value = serde_json::from_reader(
        std::fs::File::open(external.artifacts().join("External.sol/ExternalFactory.json"))
            .unwrap(),
    )
    .unwrap();
    let metadata: serde_json::Value =
        parse_json(artifact["rawMetadata"].as_str().unwrap()).unwrap();
    let compiler_version = format!("v{}", metadata["compiler"]["version"].as_str().unwrap());

    #[derive(Default)]
    struct Requests {
        sources: Vec<HashMap<String, String>>,
        submissions: Vec<HashMap<String, String>>,
    }
    let requests = Arc::new(Mutex::new(Requests::default()));
    let state = requests.clone();
    let source_json = standard_json.clone();
    let factory_for_server = factory.to_lowercase();
    let compiler_version_for_server = compiler_version.clone();

    // Verification requests are trusted and must retain normal redirect handling. External source
    // discovery uses the same selected endpoint, but remains on the non-redirecting client.
    let redirect_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redirect_url = format!("http://{}", redirect_listener.local_addr().unwrap());
    let redirect_state = requests.clone();
    let redirect_app = Router::new().fallback(move |uri: axum::http::Uri, body: String| {
        let state = redirect_state.clone();
        async move {
            let encoded = if body.is_empty() { uri.query().unwrap_or_default() } else { &body };
            let form = url::form_urlencoded::parse(encoded.as_bytes())
                .into_owned().collect::<HashMap<_, _>>();
            match form.get("action").map(String::as_str) {
                Some("verifysourcecode") => {
                    state.lock().unwrap().submissions.push(form);
                    format!(r#"{{"status":"1","message":"OK","result":"{GUID}"}}"#)
                }
                Some("checkverifystatus") => {
                    r#"{"status":"1","message":"OK","result":"Pass - Verified"}"#.to_string()
                }
                _ => r#"{"status":"0","message":"NOTOK","result":"Contract source code not verified"}"#.to_string(),
            }
        }
    });
    let _redirect_server =
        tokio::spawn(async move { axum::serve(redirect_listener, redirect_app).await.unwrap() });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let verifier_url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(move |uri: axum::http::Uri, body: String| {
        let state = state.clone();
        let source_json = source_json.clone();
        let factory = factory_for_server.clone();
        let compiler_version = compiler_version_for_server.clone();
        let redirect_url = redirect_url.clone();
        async move {
            let encoded = if body.is_empty() { uri.query().unwrap_or_default() } else { &body };
            let form = url::form_urlencoded::parse(encoded.as_bytes())
                .into_owned().collect::<HashMap<_, _>>();
            match form.get("action").map(String::as_str) {
                Some("getsourcecode") => {
                    state.lock().unwrap().sources.push(form.clone());
                    if form.get("address").is_some_and(|address| address.to_lowercase() == factory) {
                        serde_json::json!({"status":"1","message":"OK","result":[{
                            "SourceCode": source_json.to_string(), "CompilerVersion": compiler_version,
                            "ContractName": "ExternalFactory", "ABI": "[]",
                            "OptimizationUsed": "1", "OptimizationRuns": "777",
                            "ConstructorArguments": "", "EVMVersion": "osaka", "IsProxy": "0"
                        }]}).to_string().into_response()
                    } else {
                        r#"{"status":"0","message":"NOTOK","result":"Contract source code not verified"}"#.into_response()
                    }
                }
                Some("verifysourcecode" | "checkverifystatus") => (
                    StatusCode::TEMPORARY_REDIRECT,
                    [(header::LOCATION, redirect_url)],
                )
                    .into_response(),
                _ => r#"{"status":"0","message":"NOTOK","result":"Contract source code not verified"}"#.into_response(),
            }
        }
    });
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    prj.add_source(
        "IExternalFactory.sol",
        "interface IExternalFactory { function deployChild(uint256) external returns (address); }",
    );
    prj.add_script("Deploy.s.sol", &format!(r#"
import "forge-std/Script.sol";
import {{IExternalFactory}} from "../src/IExternalFactory.sol";
contract Deploy is Script {{
    function run() external {{ vm.startBroadcast(); IExternalFactory({factory}).deployChild(42); vm.stopBroadcast(); }}
}}
"#));

    let output = cmd
        .forge_fuse()
        .args([
            "script",
            "script/Deploy.s.sol:Deploy",
            "--broadcast",
            "--verify",
            "--verify-external",
            "--verifier",
            "custom",
            "--verifier-url",
            verifier_url.as_str(),
            "--verifier-api-key",
            SUBMISSION_KEY,
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            private_key.as_str(),
        ])
        .execute();
    assert!(output.status.success(), "script failed: {}", output.stderr_lossy());

    assert!(!prj.root().join("src/External.sol").exists());
    assert!(!prj.artifacts().join("External.sol/Child.json").exists());
    let submission = {
        let requests = requests.lock().unwrap();
        assert!(requests.sources.len() >= 2, "source requests: {:?}", requests.sources);
        let source_addresses = requests
            .sources
            .iter()
            .map(|request| request["address"].to_lowercase())
            .collect::<Vec<_>>();
        let expected = [executor.to_lowercase(), factory.to_lowercase()];
        assert!(
            source_addresses.windows(2).any(|addresses| addresses == expected),
            "source request order: {source_addresses:?}"
        );
        assert!(requests.sources.iter().all(|request| request["apikey"] == SUBMISSION_KEY));
        assert_eq!(requests.submissions.len(), 1, "script stderr: {}", output.stderr_lossy());
        requests.submissions[0].clone()
    };
    assert_eq!(submission["apikey"], SUBMISSION_KEY);
    assert_eq!(submission["contractname"], "src/External.sol:Child");
    assert_eq!(submission["codeformat"], "solidity-standard-json-input");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&submission["sourceCode"]).unwrap(),
        standard_json
    );
    assert_eq!(submission["compilerversion"], compiler_version);
    assert_eq!(submission["constructorArguements"], format!("{:064x}", 42));
    let broadcast_path = prj.root().join("broadcast/Deploy.s.sol/31337/run-latest.json");
    let broadcast = std::fs::read_to_string(&broadcast_path).unwrap();
    let mut sequence: ScriptSequence<Ethereum> = serde_json::from_str(&broadcast).unwrap();
    let child_address = submission["contractaddress"].parse::<Address>().unwrap();
    let child = sequence
        .transactions
        .iter()
        .flat_map(|transaction| &transaction.additional_contracts)
        .find(|contract| contract.address == child_address)
        .expect("submitted contract is not a traced nested creation");
    assert_eq!(child.call_kind, CallKind::Create2);
    assert_eq!(
        &child.creator_code_addresses[..2],
        &[executor.parse::<Address>().unwrap(), factory.parse::<Address>().unwrap()]
    );

    let value_selector = &alloy_primitives::keccak256("value()")[..4];
    let value: serde_json::Value = reqwest::Client::new()
        .post(rpc.as_str())
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "eth_call",
            "params": [{"to": child_address, "data": hex::encode_prefixed(value_selector)}, "latest"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(value["result"].as_str().unwrap().parse::<U256>().unwrap(), U256::from(42));

    // Old broadcast logs do not contain creator provenance. An explicitly requested external
    // verification must report that skipped attempt as a failure rather than `All (0)` success.
    sequence
        .transactions
        .iter_mut()
        .flat_map(|transaction| &mut transaction.additional_contracts)
        .find(|contract| contract.address == child_address)
        .unwrap()
        .creator_code_addresses
        .clear();
    sequence.recovery_generation = None;
    std::fs::write(&broadcast_path, serde_json::to_vec_pretty(&sequence).unwrap()).unwrap();
    let cache_path = prj.root().join("cache/Deploy.s.sol/31337/run-latest.json");
    let mut cache: serde_json::Value = foundry_common::fs::read_json_file(&cache_path).unwrap();
    cache.as_object_mut().unwrap().remove("recovery_generation");
    std::fs::write(cache_path, serde_json::to_vec_pretty(&cache).unwrap()).unwrap();
    std::fs::remove_file(prj.root().join("cache/Deploy.s.sol/31337/run-latest.json.recovery.json"))
        .unwrap();

    let failed = prj
        .forge_command()
        .forge_fuse()
        .args([
            "script",
            "script/Deploy.s.sol:Deploy",
            "--broadcast",
            "--resume",
            "--verify",
            "--verify-external",
            "--verifier",
            "custom",
            "--verifier-url",
            verifier_url.as_str(),
            "--verifier-api-key",
            SUBMISSION_KEY,
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            private_key.as_str(),
        ])
        .execute();
    assert!(!failed.status.success(), "resume unexpectedly succeeded: {}", failed.stderr_lossy());
    let stderr = failed.stderr_lossy();
    assert!(stderr.contains("creator provenance is unavailable"), "{stderr}");
    assert!(stderr.contains("Not all (0 / 1) contracts were verified"), "{stderr}");
    assert!(!stderr.contains("All (0) contracts were verified"), "{stderr}");
}

// Tests that the preflight check passes (does not block deploy) when the verifier responds
// with ContractCodeNotVerified (the normal "valid key, unknown address" response).
#[forgetest]
async fn create_preflight_passes_on_contract_not_verified(prj: _, cmd: _) {
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
}

// Tests that the preflight check fails (blocks deploy) when the verifier explicitly
// rejects the API key with an InvalidApiKey response.
#[forgetest]
async fn create_preflight_fails_on_invalid_api_key(prj: _, cmd: _) {
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
}

// Tests that the preflight check does NOT block deployment when the verifier responds
// with a rate-limit error (transient, not an auth failure).
#[forgetest]
async fn create_preflight_warns_on_rate_limit(prj: _, cmd: _) {
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
}

// Tests that `--resume --verify` fails when contracts that exactly matched a project artifact at
// broadcast time, top-level and nested, no longer match one, while other unmatched contracts only
// warn.
#[forgetest_init]
async fn script_verify_fails_when_deployed_contract_no_longer_matches(prj: _, cmd: _) {
    prj.add_source(
        "Alpha.sol",
        "contract Child { uint256 public x = 1; } contract Alpha { Child public child = new Child(); }",
    );
    prj.add_source("Beta.sol", "contract Beta { uint256 public y = 1; }");
    prj.add_raw_source("Alpha.vy", "x: public(uint256)\n");
    prj.add_script(
        "Deploy.s.sol",
        r#"
import "forge-std/Script.sol";
import {Alpha} from "../src/Alpha.sol";
import {Beta} from "../src/Beta.sol";
bytes constant RAW = hex"600a600c600039600a6000f3602a60005260206000f3";
contract DeployUnmatched is Script {
    function run() external {
        bytes memory raw = RAW;
        bytes memory lookalike = type(Beta).creationCode;
        lookalike[lookalike.length - 1] ^= bytes1(0x01);
        vm.startBroadcast();
        assembly {
            pop(create(0, add(raw, 0x20), mload(raw)))
            pop(create(0, add(lookalike, 0x20), mload(lookalike)))
        }
        vm.stopBroadcast();
    }
}
contract DeployVyper is Script {
    function run() external {
        vm.startBroadcast();
        vm.deployCode("src/Alpha.vy:Alpha");
        vm.stopBroadcast();
    }
}
contract DeployAlpha is Script {
    function run() external {
        bytes memory raw = RAW;
        vm.startBroadcast();
        new Alpha();
        vm.deployCode("src/Alpha.vy:Alpha");
        assembly { pop(create(0, add(raw, 0x20), mload(raw))) }
        vm.stopBroadcast();
    }
}
"#,
    );

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());
    let rpc = handle.http_endpoint();
    let (verifier_url, _, _server) = spawn_full_mock_verifier().await;
    let script = |target: &str| {
        [
            "script".to_string(),
            format!("script/Deploy.s.sol:{target}"),
            "--rpc-url".to_string(),
            rpc.clone(),
            "--private-key".to_string(),
            pk.clone(),
            "--broadcast".to_string(),
        ]
    };
    let verify = [
        "--verify",
        "--verifier",
        "custom",
        "--verifier-url",
        verifier_url.as_str(),
        "--verifier-api-key",
        "VALID_KEY",
    ];

    // Unmatched contracts that never exactly matched a project artifact only warn and count as
    // skipped, on a fresh run and on resume: raw bytecode has no recorded name, and `Beta` with a
    // flipped metadata byte is only named by fuzzy matching.
    cmd.forge_fuse().args(script("DeployUnmatched")).args(verify).assert_success().stderr_eq(str![[r#"
Warning: We haven't found any matching bytecode for the following contracts: [0x5fbdb2315678afecb367f032d93f642f64180aa3, 0xe7f1725e7734ce288f8367e1bb143e90bb3f0512].

This may occur when resuming a verification, but the underlying source code or compiler version has changed.
Run `forge clean` to make sure builds are in sync with project files, then try again. Alternatively, use `forge verify-contract` to verify contracts that are already deployed.
##
Start verification for (0) contracts
Verified (0) contracts; skipped (2) contracts (see warnings above).

"#]]);
    cmd.forge_fuse()
        .args(script("DeployUnmatched"))
        .arg("--resume")
        .args(verify)
        .assert_success()
        .stderr_eq(str![[r#"
Warning: We haven't found any matching bytecode for the following contracts: [0x5fbdb2315678afecb367f032d93f642f64180aa3, 0xe7f1725e7734ce288f8367e1bb143e90bb3f0512].

This may occur when resuming a verification, but the underlying source code or compiler version has changed.
Run `forge clean` to make sure builds are in sync with project files, then try again. Alternatively, use `forge verify-contract` to verify contracts that are already deployed.
##
Start verification for (0) contracts
Verified (0) contracts; skipped (2) contracts (see warnings above).

"#]]);

    let broadcast: serde_json::Value = foundry_common::fs::read_json_file(
        &prj.root().join("broadcast/Deploy.s.sol/31337/run-latest.json"),
    )
    .unwrap();
    assert_eq!(broadcast["transactions"][1]["contractName"], "Beta");

    // A Vyper contract is skipped even when a Solidity contract shares its name.
    cmd.forge_fuse().args(script("DeployVyper")).args(verify).assert_success().stderr_eq(str![[r#"
Warning: We haven't found any matching bytecode for the following contracts: [0x9fe46736679d2d9a65f0992f2272de9f3c7fa6e0].

This may occur when resuming a verification, but the underlying source code or compiler version has changed.
Run `forge clean` to make sure builds are in sync with project files, then try again. Alternatively, use `forge verify-contract` to verify contracts that are already deployed.
##
Start verification for (0) contracts
Verified (0) contracts; skipped (1) contracts (see warnings above).

"#]]);

    // After both `Alpha` sources change, the Solidity `Alpha` and its nested `Child` fail, while
    // the Vyper `Alpha` and raw bytecode only warn.
    cmd.forge_fuse().args(script("DeployAlpha")).assert_success();
    prj.add_raw_source("Alpha.vy", "x: public(uint256)\nz: public(uint256)\n");
    prj.add_source(
        "Alpha.sol",
        "contract Child { uint256 public x = 2; } contract Alpha { Child public child = new Child(); }",
    );
    cmd.forge_fuse()
        .args(script("DeployAlpha"))
        .arg("--resume")
        .args(verify)
        .assert_failure()
        .stderr_eq(str![[r#"
Warning: We haven't found any matching bytecode for the following contracts: [0xcf7ed3acca5a467e9e704c703e8d87f634fb0fc9, 0xd8058efe0198ae9dd7d563e1b4938dcbc86a1f81, 0xdc64a140aa3e981100a9beca4e685f962f0cf6c9, 0x5fc8d32690cc91d4c39d9d3abcbd16989f875707].

This may occur when resuming a verification, but the underlying source code or compiler version has changed.
Run `forge clean` to make sure builds are in sync with project files, then try again. Alternatively, use `forge verify-contract` to verify contracts that are already deployed.
##
Start verification for (2) contracts
Error: Not all (0 / 2) contracts were verified; no local artifact matches the creation code of Alpha at 0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9, Child at 0xd8058efe0198ae9dD7D563e1b4938Dcbc86A1F81!

"#]]);
}

// Equal sender nonces on different chains must not make one deployment inherit the other's
// artifact identity when deciding whether verification is required.
#[forgetest_init]
async fn script_verify_fork_same_address_different_artifacts(prj: _, cmd: _) {
    prj.add_source("Alpha.sol", "contract Alpha { uint256 public x = 1; }");
    prj.add_source("Beta.sol", "contract Beta { uint256 public y = 2; }");
    let (_api1, handle1) = spawn(NodeConfig::test()).await;
    let (_api2, handle2) = spawn(NodeConfig::test().with_chain_id(Some(31338u64))).await;
    let wallet = handle1.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());
    prj.add_script(
        "Deploy.s.sol",
        &r#"
import "forge-std/Script.sol";
import {Alpha} from "../src/Alpha.sol";
import {Beta} from "../src/Beta.sol";
contract Deploy is Script {
    function run() external {
        vm.createSelectFork("<RPC1>");
        vm.startBroadcast();
        address alpha = address(new Alpha());
        vm.stopBroadcast();
        vm.createSelectFork("<RPC2>");
        vm.startBroadcast();
        address beta = address(new Beta());
        vm.stopBroadcast();
        require(alpha == beta, "deployment addresses differ");
    }
}
"#
        .replace("<RPC1>", &handle1.http_endpoint())
        .replace("<RPC2>", &handle2.http_endpoint()),
    );
    let script = ["script", "script/Deploy.s.sol:Deploy", "--broadcast", "--private-key", &pk];
    cmd.forge_fuse().args(script).assert_success();

    let (verifier_url, _, _server) = spawn_full_mock_verifier().await;
    let resume = [
        "--resume",
        "--multi",
        "--verify",
        "--verifier",
        "custom",
        "--verifier-url",
        verifier_url.as_str(),
        "--verifier-api-key",
        "VALID_KEY",
    ];
    // Both deployments can be verified against the original artifacts.
    cmd.forge_fuse().args(script).args(resume).assert_success();
    prj.add_source("Beta.sol", "contract Beta { uint256 public y = 3; }");
    cmd.forge_fuse().args(script).args(resume).assert_failure().stderr_eq(str![[r#"
...
Error: chain 31338: Not all (0 / 1) contracts were verified; no local artifact matches the creation code of Beta at 0x5FbDB2315678afecb367f032d93F642f64180aa3!

"#]]);
}

// The EVM permits ignored trailing bytes even without an ABI constructor. Such input must use
// the same artifact-prefix matching rule when recording deployments and verifying them.
#[forgetest_init]
async fn script_verify_nested_creation_with_trailing_data(prj: _, cmd: _) {
    prj.add_source("Child.sol", "contract Child { uint256 public x = 1; }");
    prj.add_source(
        "Factory.sol",
        r#"
contract Factory {
    function deploy(bytes memory code) external {
        assembly { pop(create(0, add(code, 0x20), mload(code))) }
    }
}
"#,
    );
    prj.add_script(
        "Deploy.s.sol",
        r#"
import "forge-std/Script.sol";
import {Child} from "../src/Child.sol";
import {Factory} from "../src/Factory.sol";
contract Deploy is Script {
    function run() external {
        bytes memory code = bytes.concat(type(Child).creationCode, new bytes(1024));
        vm.startBroadcast();
        Factory factory = new Factory();
        factory.deploy(code);
        vm.stopBroadcast();
    }
}
"#,
    );
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());
    let rpc = handle.http_endpoint();
    let script = [
        "script",
        "script/Deploy.s.sol:Deploy",
        "--rpc-url",
        rpc.as_str(),
        "--broadcast",
        "--private-key",
        &pk,
    ];
    cmd.forge_fuse().args(script).assert_success();
    let sequence: ScriptSequence<Ethereum> = foundry_common::fs::read_json_file(
        &prj.root().join("broadcast/Deploy.s.sol/31337/run-latest.json"),
    )
    .unwrap();
    assert_eq!(sequence.transactions[1].additional_contracts.len(), 1);

    let (verifier_url, _, _server) = spawn_full_mock_verifier().await;
    let resume = [
        "--resume",
        "--verify",
        "--verifier",
        "custom",
        "--verifier-url",
        verifier_url.as_str(),
        "--verifier-api-key",
        "VALID_KEY",
    ];
    cmd.forge_fuse().args(script).args(resume).assert_success();
    prj.add_source("Child.sol", "contract Child { uint256 public x = 2; }");
    cmd.forge_fuse().args(script).args(resume).assert_failure().stderr_eq(str![[r#"
...
Error: Not all (1 / 2) contracts were verified; no local artifact matches the creation code of Child at 0xa16E02E87b7454126E5E10d957A927A7F5B5d2be!

"#]]);
}

// A chain whose verification fails does not stop the contracts of later chains from being
// submitted, and the error names the failing chain.
#[forgetest_init]
async fn script_verifies_every_chain_after_a_failure(prj: _, cmd: _) {
    prj.add_source(
        "Contracts.sol",
        r#"
contract Alpha { function id() external pure returns (uint256) { return 1111; } }
contract Beta { function id() external pure returns (uint256) { return 2222; } }
"#,
    );
    let script = prj.add_script(
        "MultiVerify.s.sol",
        r#"
import "forge-std/Script.sol";
import {Alpha, Beta} from "../src/Contracts.sol";

contract MultiVerify is Script {
    function run(string memory rpc1, string memory rpc2) external {
        vm.createSelectFork(rpc1);
        vm.startBroadcast();
        new Alpha();
        vm.stopBroadcast();

        vm.createSelectFork(rpc2);
        vm.startBroadcast();
        new Beta();
        vm.stopBroadcast();
    }
}
"#,
    );
    let (_api1, handle1) = spawn(NodeConfig::test().with_chain_id(Some(1111u64))).await;
    let (_api2, handle2) = spawn(NodeConfig::test().with_chain_id(Some(2222u64))).await;
    let (rpc1, transactions1) =
        spawn_rpc_proxy_recording_method(handle1.http_endpoint(), "eth_sendRawTransaction").await;
    let (rpc2, transactions2) =
        spawn_rpc_proxy_recording_method(handle2.http_endpoint(), "eth_sendRawTransaction").await;

    // Initially reject only Alpha, then both contracts, then accept Alpha with Beta already
    // verified.
    let attempt = Arc::new(AtomicUsize::new(0));
    let verifier_attempt = attempt.clone();
    let submissions = Arc::new(Mutex::new(Vec::new()));
    let state = submissions.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let verifier_url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().fallback(move |uri: axum::http::Uri, body: String| {
        let state = state.clone();
        let attempt = verifier_attempt.load(Ordering::SeqCst);
        async move {
            let encoded = if body.is_empty() { uri.query().unwrap_or_default() } else { &body };
            let form = url::form_urlencoded::parse(encoded.as_bytes())
                .into_owned()
                .collect::<HashMap<_, _>>();
            match form.get("action").map(String::as_str) {
                Some("verifysourcecode") => {
                    let name = form["contractname"].clone();
                    let alpha = name.ends_with(":Alpha");
                    state.lock().unwrap().push(name);
                    if attempt == 1 || (attempt == 0 && alpha) {
                        r#"{"status":"0","message":"NOTOK","result":"mock failure"}"#
                    } else if attempt == 2 && !alpha {
                        r#"{"status":"0","message":"NOTOK","result":"Contract source code already verified"}"#
                    } else {
                        r#"{"status":"1","message":"OK","result":"guid"}"#
                    }
                }
                Some("checkverifystatus") => {
                    r#"{"status":"1","message":"OK","result":"Pass - Verified"}"#
                }
                _ => r#"{"status":"0","message":"NOTOK","result":"Contract source code not verified"}"#,
            }
        }
    });
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let args = [
        "script",
        script.to_str().unwrap(),
        "--sig",
        "run(string,string)",
        &rpc1,
        &rpc2,
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--broadcast",
        "--verify",
        "--verifier",
        "custom",
        "--verifier-url",
        &verifier_url,
        "--verifier-api-key",
        "KEY",
        "--retries",
        "1",
        "--delay",
        "0",
    ];
    cmd.forge_fuse().args(args);
    cmd.assert_failure().stderr_eq(str![[r#"
...
Error: chain 1111: Not all (0 / 1) contracts were verified!

"#]]);

    // `--retries 1` submits `Alpha` twice before chain 2222 is verified.
    assert_eq!(
        *submissions.lock().unwrap(),
        ["src/Contracts.sol:Alpha", "src/Contracts.sol:Alpha", "src/Contracts.sol:Beta"]
    );
    assert_eq!(transactions1.lock().unwrap().len(), 1);
    assert_eq!(transactions2.lock().unwrap().len(), 1);

    // A retry that fails on both chains reports both failures, in sequence order.
    attempt.store(1, Ordering::SeqCst);
    submissions.lock().unwrap().clear();
    cmd.forge_fuse().args(args).args(["--multi", "--resume"]);
    cmd.assert_failure().stderr_eq(str![[r#"
...
Error: chain 1111: Not all (0 / 1) contracts were verified!
chain 2222: Not all (0 / 1) contracts were verified!

"#]]);
    assert_eq!(
        *submissions.lock().unwrap(),
        [
            "src/Contracts.sol:Alpha",
            "src/Contracts.sol:Alpha",
            "src/Contracts.sol:Beta",
            "src/Contracts.sol:Beta",
        ]
    );
    assert_eq!(transactions1.lock().unwrap().len(), 1);
    assert_eq!(transactions2.lock().unwrap().len(), 1);

    // Resume retries verification on every chain, counting an already-verified response as success
    // without rebroadcasting completed deployments.
    attempt.store(2, Ordering::SeqCst);
    submissions.lock().unwrap().clear();
    cmd.forge_fuse().args(args).args(["--multi", "--resume"]);
    cmd.assert_success();
    assert_eq!(*submissions.lock().unwrap(), ["src/Contracts.sol:Alpha", "src/Contracts.sol:Beta"]);
    assert_eq!(transactions1.lock().unwrap().len(), 1);
    assert_eq!(transactions2.lock().unwrap().len(), 1);
}
