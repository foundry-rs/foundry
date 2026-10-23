use crate::utils;
use alloy_chains::Chain;
use alloy_network::ReceiptResponse;
use alloy_primitives::{Address, B256, Bytes, hex};
use alloy_provider::Provider;
use axum::{Json, Router, extract::Query};
use foundry_compilers::artifacts::{BytecodeHash, EvmVersion};
use foundry_config::Config;
use foundry_evm_networks::NetworkConfigs;
use foundry_test_utils::{
    TestCommand, TestProject,
    etherscan::fetch_etherscan_source_flattened,
    rpc::{next_etherscan_api_key, next_http_archive_rpc_url},
    util::OutputExt,
};
use std::{collections::HashMap, fs};
use tokio::net::TcpListener;

#[forgetest_init]
async fn can_verify_bytecode_with_local_creation_data_fork(prj: _, cmd: _) {
    prj.initialize_default_contracts();
    cmd.forge_fuse().arg("build").assert_success();

    let artifact: serde_json::Value = serde_json::from_slice(
        &fs::read(prj.paths().artifacts.join("Counter.sol/Counter.json")).unwrap(),
    )
    .unwrap();
    let bytecode =
        Bytes::from(hex::decode(artifact["bytecode"]["object"].as_str().unwrap()).unwrap());

    let (api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let accounts = handle.dev_accounts().take(3).collect::<Vec<_>>();
    api.anvil_set_auto_mine(false).await.unwrap();
    let gas_price = provider.get_gas_price().await.unwrap();
    let _: B256 = provider
        .client()
        .request(
            "eth_sendTransaction",
            [serde_json::json!({
                "from": accounts[1],
                "to": accounts[2],
                "value": "0x1",
                "gasPrice": format!("0x{:x}", gas_price + 1),
            })],
        )
        .await
        .unwrap();
    let transaction_hash: B256 = provider
        .client()
        .request(
            "eth_sendTransaction",
            [serde_json::json!({
                "from": accounts[0],
                "data": bytecode,
                "gasPrice": format!("0x{gas_price:x}"),
            })],
        )
        .await
        .unwrap();
    api.mine_one().await.unwrap();
    let receipt = provider.get_transaction_receipt(transaction_hash).await.unwrap().unwrap();
    assert_eq!(receipt.transaction_index(), Some(1));
    let address = receipt.contract_address().unwrap().to_string();

    // Local explorer data forces a nonempty creation-block replay and shared completion path.
    let creation_data = serde_json::json!({"status":"1", "message":"OK", "result":[{
        "contractAddress": address,
        "contractCreator": accounts[0],
        "txHash": transaction_hash,
    }]});
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api", listener.local_addr().unwrap());
    let app = Router::new().fallback(move |Query(query): Query<HashMap<String, String>>| {
        let response = if query.get("action").is_some_and(|action| action == "getcontractcreation")
        {
            creation_data.clone()
        } else {
            serde_json::json!({"status":"1", "message":"OK", "result":[]})
        };
        async move { Json(response) }
    });
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    cmd.forge_fuse();
    cmd.args([
        "verify-bytecode",
        &address,
        "src/Counter.sol:Counter",
        "--rpc-url",
        &rpc,
        "--verifier",
        "etherscan",
        "--verifier-url",
        &url,
        "--etherscan-api-key",
        "test",
        "--json",
    ])
    .assert_json_stdout(
        r#"[
        {"bytecode_type":"creation", "match_type":"full"},
        {"bytecode_type":"runtime", "match_type":"full"}
    ]"#,
    );
    server.abort();
}

#[expect(clippy::too_many_arguments)]
async fn test_verify_bytecode(
    prj: TestProject,
    mut cmd: TestCommand,
    addr: &str,
    contract_name: &str,
    constructor_args: Option<Vec<&str>>,
    config: Config,
    verifier: &str,
    verifier_url: &str,
    expected_matches: (&str, &str),
    chain: Chain,
) {
    let etherscan_key = next_etherscan_api_key();
    let rpc_url = next_http_archive_rpc_url();

    // fetch and flatten source code using the library directly
    let source_code = fetch_etherscan_source_flattened(addr, &etherscan_key, chain)
        .await
        .expect("failed to fetch source code from etherscan");

    prj.add_source(contract_name, &source_code);
    prj.write_config(config);

    let etherscan_key = next_etherscan_api_key();
    let mut args = vec![
        "verify-bytecode",
        addr,
        contract_name,
        "--etherscan-api-key",
        &etherscan_key,
        "--verifier",
        verifier,
        "--verifier-url",
        verifier_url,
        "--rpc-url",
        &rpc_url,
    ];

    if let Some(constructor_args) = constructor_args {
        args.push("--constructor-args");
        args.extend(constructor_args.iter());
    }

    let output = cmd.forge_fuse().args(args).assert_success().get_output().stdout_lossy();

    assert!(
        output
            .contains(format!("Creation code matched with status {}", expected_matches.0).as_str())
    );
    assert!(
        output
            .contains(format!("Runtime code matched with status {}", expected_matches.1).as_str())
    );
}

#[expect(clippy::too_many_arguments)]
async fn test_verify_bytecode_with_ignore(
    prj: TestProject,
    mut cmd: TestCommand,
    addr: &str,
    contract_name: &str,
    config: Config,
    verifier: &str,
    verifier_url: &str,
    expected_matches: (&str, &str),
    ignore: &str,
    chain: Chain,
) {
    let etherscan_key = next_etherscan_api_key();
    let rpc_url = next_http_archive_rpc_url();

    // fetch and flatten source code using the library directly
    let source_code = fetch_etherscan_source_flattened(addr, &etherscan_key, chain)
        .await
        .expect("failed to fetch source code from etherscan");

    prj.add_source(contract_name, &source_code);
    prj.write_config(config);

    let output = cmd
        .forge_fuse()
        .args([
            "verify-bytecode",
            addr,
            contract_name,
            "--etherscan-api-key",
            &etherscan_key,
            "--verifier",
            verifier,
            "--verifier-url",
            verifier_url,
            "--rpc-url",
            &rpc_url,
            "--ignore",
            ignore,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    if ignore == "creation" {
        assert!(!output.contains(
            format!("Creation code matched with status {}", expected_matches.0).as_str()
        ));
    } else {
        assert!(output.contains(
            format!("Creation code matched with status {}", expected_matches.0).as_str()
        ));
    }

    if ignore == "runtime" {
        assert!(
            !output.contains(
                format!("Runtime code matched with status {}", expected_matches.1).as_str()
            )
        );
    } else {
        assert!(
            output.contains(
                format!("Runtime code matched with status {}", expected_matches.1).as_str()
            )
        );
    }
}

#[forgetest]
async fn flaky_verify_bytecode_no_metadata(prj: _, cmd: _) {
    test_verify_bytecode(
        prj,
        cmd,
        "0xba2492e52F45651B60B8B38d4Ea5E2390C64Ffb1",
        "SystemConfig",
        None,
        Config {
            evm_version: EvmVersion::London,
            optimizer_runs: Some(999999),
            optimizer: Some(true),
            cbor_metadata: false,
            bytecode_hash: BytecodeHash::None,
            ..Default::default()
        },
        "etherscan",
        "https://api.etherscan.io/v2/api?chainid=1",
        ("partial", "partial"),
        Chain::mainnet(),
    )
    .await;
}

#[forgetest]
async fn flaky_verify_bytecode_with_metadata(prj: _, cmd: _) {
    test_verify_bytecode(
        prj,
        cmd,
        "0xb8901acb165ed027e32754e0ffe830802919727f",
        "L1_ETH_Bridge",
        None,
        Config {
            evm_version: EvmVersion::Paris,
            optimizer_runs: Some(50000),
            optimizer: Some(true),
            ..Default::default()
        },
        "etherscan",
        "https://api.etherscan.io/v2/api?chainid=1",
        ("partial", "partial"),
        Chain::mainnet(),
    )
    .await;
}

// Test non-CREATE2 deployed contract with blockscout
#[forgetest]
async fn flaky_verify_bytecode_with_blockscout(prj: _, cmd: _) {
    test_verify_bytecode(
        prj,
        cmd,
        "0x70f44C13944d49a236E3cD7a94f48f5daB6C619b",
        "StrategyManager",
        None,
        Config {
            evm_version: EvmVersion::London,
            optimizer: Some(true),
            optimizer_runs: Some(200),
            ..Default::default()
        },
        "blockscout",
        "https://eth.blockscout.com/api",
        ("partial", "partial"),
        Chain::mainnet(),
    )
    .await;
}

// Test CREATE2 deployed contract with blockscout
#[forgetest]
async fn flaky_verify_bytecode_create2_with_blockscout(prj: _, cmd: _) {
    test_verify_bytecode(
        prj,
        cmd,
        "0xba2492e52F45651B60B8B38d4Ea5E2390C64Ffb1",
        "SystemConfig",
        None,
        Config {
            evm_version: EvmVersion::London,
            optimizer_runs: Some(999999),
            optimizer: Some(true),
            cbor_metadata: false,
            bytecode_hash: BytecodeHash::None,
            ..Default::default()
        },
        "blockscout",
        "https://eth.blockscout.com/api",
        ("partial", "partial"),
        Chain::mainnet(),
    )
    .await;
}

// Test `--constructor-args`
#[forgetest]
async fn flaky_verify_bytecode_with_constructor_args(prj: _, cmd: _) {
    let constructor_args = vec![
        "0x39053D51B77DC0d36036Fc1fCc8Cb819df8Ef37A",
        "0x91E677b07F7AF907ec9a428aafA9fc14a0d3A338",
        "0xD92145c07f8Ed1D392c1B88017934E301CC1c3Cd",
    ];
    test_verify_bytecode(
        prj,
        cmd,
        "0x70f44C13944d49a236E3cD7a94f48f5daB6C619b",
        "StrategyManager",
        Some(constructor_args),
        Config {
            evm_version: EvmVersion::London,
            optimizer: Some(true),
            optimizer_runs: Some(200),
            ..Default::default()
        },
        "etherscan",
        "https://api.etherscan.io/v2/api?chainid=1",
        ("partial", "partial"),
        Chain::mainnet(),
    )
    .await;
}

// Wrong `--constructor-args` used to verify clean, because supplied args that were not the tail
// of the creation code were silently replaced by the real ones.
#[forgetest]
async fn flaky_verify_bytecode_warns_on_wrong_constructor_args(prj: _, cmd: _) {
    let etherscan_key = next_etherscan_api_key();
    let rpc_url = next_http_archive_rpc_url();
    let addr = "0x70f44C13944d49a236E3cD7a94f48f5daB6C619b";

    let source_code = fetch_etherscan_source_flattened(addr, &etherscan_key, Chain::mainnet())
        .await
        .expect("failed to fetch source code from etherscan");
    prj.add_source("StrategyManager", &source_code);
    prj.write_config(Config {
        evm_version: EvmVersion::London,
        optimizer: Some(true),
        optimizer_runs: Some(200),
        ..Default::default()
    });

    let etherscan_key = next_etherscan_api_key();
    let run = cmd
        .forge_fuse()
        .args([
            "verify-bytecode",
            addr,
            "StrategyManager",
            "--etherscan-api-key",
            &etherscan_key,
            "--verifier",
            "etherscan",
            "--verifier-url",
            "https://api.etherscan.io/v2/api?chainid=1",
            "--rpc-url",
            &rpc_url,
            // Three zero addresses instead of the real constructor arguments.
            "--constructor-args",
            "0x0000000000000000000000000000000000000000",
            "0x0000000000000000000000000000000000000000",
            "0x0000000000000000000000000000000000000000",
        ])
        .assert_success();
    let output = run.get_output();

    // The warning goes to stderr, the match verdict to stdout.
    let stderr = output.stderr_lossy();
    let stdout = output.stdout_lossy();

    assert!(
        stderr.contains(
            "Provided constructor args could not be validated against deployment creation code"
        ),
        "expected a warning that the supplied args do not match the deployment, got:\n{stderr}"
    );
    assert!(
        !stdout.contains("Creation code matched"),
        "wrong constructor args must not produce a creation match, got:\n{stdout}"
    );

    // Ignoring creation verification must still compare the runtime produced by the supplied
    // arguments. StrategyManager embeds its constructor arguments as immutables, so they produce
    // a runtime mismatch.
    let etherscan_key = next_etherscan_api_key();
    cmd.forge_fuse()
        .args([
            "verify-bytecode",
            addr,
            "StrategyManager",
            "--etherscan-api-key",
            &etherscan_key,
            "--verifier",
            "etherscan",
            "--verifier-url",
            "https://api.etherscan.io/v2/api?chainid=1",
            "--rpc-url",
            &rpc_url,
            "--constructor-args",
            "0x0000000000000000000000000000000000000000",
            "0x0000000000000000000000000000000000000000",
            "0x0000000000000000000000000000000000000000",
            "--ignore",
            "creation",
            "--json",
        ])
        .assert_json_stdout(
            r#"[{"bytecode_type":"runtime","match_type":null,"message":"Runtime code did not match - this may be due to varying compiler settings"}]"#,
        );
}

// `--ignore` tests
#[forgetest]
async fn flaky_verify_bytecode_can_ignore_creation(prj: _, cmd: _) {
    test_verify_bytecode_with_ignore(
        prj,
        cmd,
        "0xba2492e52F45651B60B8B38d4Ea5E2390C64Ffb1",
        "SystemConfig",
        Config {
            evm_version: EvmVersion::London,
            optimizer_runs: Some(999999),
            optimizer: Some(true),
            cbor_metadata: false,
            bytecode_hash: BytecodeHash::None,
            ..Default::default()
        },
        "etherscan",
        "https://api.etherscan.io/v2/api?chainid=1",
        ("ignored", "partial"),
        "creation",
        Chain::mainnet(),
    )
    .await;
}

#[forgetest]
async fn flaky_verify_bytecode_can_ignore_runtime(prj: _, cmd: _) {
    test_verify_bytecode_with_ignore(
        prj,
        cmd,
        "0xba2492e52F45651B60B8B38d4Ea5E2390C64Ffb1",
        "SystemConfig",
        Config {
            evm_version: EvmVersion::London,
            optimizer_runs: Some(999999),
            optimizer: Some(true),
            cbor_metadata: false,
            bytecode_hash: BytecodeHash::None,
            ..Default::default()
        },
        "etherscan",
        "https://api.etherscan.io/v2/api?chainid=1",
        ("partial", "ignored"),
        "runtime",
        Chain::mainnet(),
    )
    .await;
}

// Test that verification fails when source code doesn't match deployed bytecode
#[forgetest]
async fn flaky_can_verify_bytecode_fails_on_source_mismatch(prj: _, cmd: _) {
    let etherscan_key = next_etherscan_api_key();
    let rpc_url = next_http_archive_rpc_url();

    // Fetch real source code using the library directly
    let real_source = fetch_etherscan_source_flattened(
        "0xba2492e52F45651B60B8B38d4Ea5E2390C64Ffb1",
        &etherscan_key,
        Chain::mainnet(),
    )
    .await
    .expect("failed to fetch source code from etherscan");

    prj.add_source("SystemConfig", &real_source);
    prj.write_config(Config {
        evm_version: EvmVersion::London,
        optimizer_runs: Some(999999),
        optimizer: Some(true),
        cbor_metadata: false,
        bytecode_hash: BytecodeHash::None,
        ..Default::default()
    });
    // Build once with correct source (creates cache). Linting is unrelated to bytecode
    // verification here and can dominate runtime on the flattened Etherscan source.
    cmd.forge_fuse().args(["build", "--no-lint"]).assert_success();

    let source_code = r#"
    contract SystemConfig {
        uint256 public constant MODIFIED_VALUE = 999;

        function someFunction() public pure returns (uint256) {
            return MODIFIED_VALUE;
        }
    }
    "#;

    // Now replace with different incorrect source code
    prj.add_source("SystemConfig", source_code);
    let etherscan_key = next_etherscan_api_key();
    let args = vec![
        "verify-bytecode",
        "0xba2492e52F45651B60B8B38d4Ea5E2390C64Ffb1",
        "SystemConfig",
        "--etherscan-api-key",
        &etherscan_key,
        "--verifier",
        "etherscan",
        "--verifier-url",
        "https://api.etherscan.io/v2/api?chainid=1",
        "--rpc-url",
        &rpc_url,
    ];
    let output = cmd.forge_fuse().args(args).assert_success().get_output().stderr_lossy();

    // Verify that bytecode does NOT match (recompiled with incorrect source)
    assert!(output.contains("Error: Creation code did not match".to_string().as_str()));
    assert!(output.contains("Error: Runtime code did not match".to_string().as_str()));
}

// Tests that `verify-bytecode` works without any external block explorer, relying only on the
// local project and an RPC endpoint.
// <https://github.com/foundry-rs/foundry/issues/13479>
#[forgetest_init]
async fn can_verify_bytecode_without_explorer(prj: _, cmd: _) {
    prj.initialize_default_contracts();

    let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = alloy_primitives::hex::encode(wallet.credential().to_bytes());

    // Deploy the template contract; first tx of the default dev account.
    cmd.forge_fuse();
    cmd.unset_env("ETHERSCAN_API_KEY");
    cmd.unset_env("VERIFIER_API_KEY");
    cmd.unset_env("VERIFIER_URL");
    let output = cmd
        .args([
            "create",
            "./src/Counter.sol:Counter",
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            pk.as_str(),
            "--broadcast",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let address = output
        .lines()
        .find_map(|line| line.strip_prefix("Deployed to: "))
        .expect("contract address in `forge create` output")
        .to_string();

    // Bare contract names should compile only their uniquely resolved source.
    prj.add_source("Broken", "contract Broken { uint256 public value = doesNotExist; }");

    // The local anvil chain has no block explorer: the command must still verify the runtime
    // bytecode and only warn about the unavailable explorer data.
    cmd.forge_fuse();
    cmd.unset_env("ETHERSCAN_API_KEY");
    cmd.unset_env("VERIFIER_API_KEY");
    cmd.unset_env("VERIFIER_URL");
    let assert = cmd
        .args(["verify-bytecode", &address, "Counter", "--rpc-url", rpc.as_str()])
        .assert_success();
    let output = assert.get_output();
    let stdout = output.stdout_lossy();
    let stderr = output.stderr_lossy();

    assert!(stdout.contains("Runtime code matched"), "{stdout}");
    assert!(stderr.contains("Creation data is unavailable"), "{stderr}");

    #[cfg(feature = "base")]
    {
        cmd.forge_fuse();
        cmd.unset_env("ETHERSCAN_API_KEY");
        cmd.unset_env("VERIFIER_API_KEY");
        cmd.unset_env("VERIFIER_URL");
        let output = cmd
            .args([
                "verify-bytecode",
                &address,
                "Counter",
                "--rpc-url",
                rpc.as_str(),
                "--network",
                "base",
            ])
            .assert_success()
            .get_output()
            .stdout_lossy();
        assert!(output.contains("Runtime code matched"), "{output}");
    }

    // Dependencies and projects with Vyper sources retain full-project compilation. The unrelated
    // invalid source therefore makes both builds fail.
    prj.create_file("lib/Dependency.sol", "contract Dependency {}");
    prj.add_source("UsesDependency", "import '../lib/Dependency.sol'; contract UsesDependency {}");
    cmd.forge_fuse()
        .args(["verify-bytecode", &address, "Dependency", "--rpc-url", rpc.as_str()])
        .assert_failure();
    let vyper_source = prj.add_raw_source("Counter.vy", "invalid Vyper source");
    cmd.forge_fuse()
        .args(["verify-bytecode", &address, "Counter", "--rpc-url", rpc.as_str()])
        .assert_failure();
    fs::remove_file(vyper_source).unwrap();

    // `--ignore runtime` must skip the runtime fallback as well: with no creation data either,
    // there is nothing left to verify.
    cmd.forge_fuse();
    cmd.unset_env("ETHERSCAN_API_KEY");
    cmd.unset_env("VERIFIER_API_KEY");
    cmd.unset_env("VERIFIER_URL");
    let assert = cmd
        .args([
            "verify-bytecode",
            &address,
            "Counter",
            "--rpc-url",
            rpc.as_str(),
            "--ignore",
            "runtime",
        ])
        .assert_success();
    let output = assert.get_output();
    let stdout = output.stdout_lossy();
    let stderr = output.stderr_lossy();

    assert!(!stdout.contains("Runtime code matched"), "{stdout}");
    assert!(stderr.contains("Creation data is unavailable"), "{stderr}");

    // An explicitly configured but broken verifier must surface an error instead of being
    // silently treated as "no explorer".
    cmd.forge_fuse();
    cmd.unset_env("ETHERSCAN_API_KEY");
    cmd.unset_env("VERIFIER_API_KEY");
    cmd.unset_env("VERIFIER_URL");
    cmd.args([
        "verify-bytecode",
        &address,
        "Counter",
        "--rpc-url",
        rpc.as_str(),
        "--verifier-url",
        "this-is-not-a-url",
    ])
    .assert_failure();
}

#[forgetest_init]
async fn can_verify_bytecode_with_libraries(prj: _, cmd: _) {
    prj.update_config(|config| config.libraries.clear());
    prj.add_source(
        "Libraries",
        r#"
library FirstLib {
    function compute(uint256 value) external pure returns (uint256) {
        return value + 1;
    }
}

library SecondLib {
    function compute(uint256 value) external pure returns (uint256) {
        return value * 2;
    }
}
"#,
    );
    prj.add_source(
        "LinkedContract",
        r#"
import {FirstLib, SecondLib} from "./Libraries.sol";

contract LinkedContract {
    uint256 public immutable initial;

    constructor() {
        initial = SecondLib.compute(FirstLib.compute(20));
    }

    function compute(uint256 value) external view returns (uint256) {
        return SecondLib.compute(FirstLib.compute(value));
    }
}
"#,
    );

    let (_api, handle) = anvil::spawn(anvil::NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode(wallet.credential().to_bytes());

    cmd.forge_fuse();
    cmd.unset_env("DAPP_LIBRARIES");
    cmd.unset_env("FOUNDRY_LIBRARIES");
    cmd.unset_env("FOUNDRY_CONFIG");
    let output = cmd
        .args([
            "create",
            "src/Libraries.sol:FirstLib",
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            pk.as_str(),
            "--broadcast",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let first_lib = utils::parse_deployed_address(&output)
        .unwrap_or_else(|| panic!("Failed to parse deployed library address: {output}"));

    cmd.forge_fuse();
    cmd.unset_env("DAPP_LIBRARIES");
    cmd.unset_env("FOUNDRY_LIBRARIES");
    cmd.unset_env("FOUNDRY_CONFIG");
    let output = cmd
        .args([
            "create",
            "src/Libraries.sol:SecondLib",
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            pk.as_str(),
            "--broadcast",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let second_lib = utils::parse_deployed_address(&output)
        .unwrap_or_else(|| panic!("Failed to parse deployed library address: {output}"));

    let first_lib_spec = format!("src/Libraries.sol:FirstLib:{first_lib}");
    let second_lib_spec = format!("src/Libraries.sol:SecondLib:{second_lib}");

    cmd.forge_fuse();
    cmd.unset_env("DAPP_LIBRARIES");
    cmd.unset_env("FOUNDRY_LIBRARIES");
    cmd.unset_env("FOUNDRY_CONFIG");
    let output = cmd
        .args([
            "create",
            "src/LinkedContract.sol:LinkedContract",
            "--rpc-url",
            rpc.as_str(),
            "--private-key",
            pk.as_str(),
            "--broadcast",
            "--libraries",
            first_lib_spec.as_str(),
            "--libraries",
            second_lib_spec.as_str(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let contract = utils::parse_deployed_address(&output)
        .unwrap_or_else(|| panic!("Failed to parse deployed contract address: {output}"));

    // Explicit CLI values must take precedence over configured addresses for the same libraries.
    prj.update_config(|config| {
        config.libraries = vec![
            "src/Libraries.sol:FirstLib:0x1111111111111111111111111111111111111111".to_string(),
            "src/Libraries.sol:SecondLib:0x2222222222222222222222222222222222222222".to_string(),
        ];
    });

    // Ensure verification recompiles with its own linker arguments rather than reusing the
    // artifacts produced by `forge create`.
    prj.clear();

    cmd.forge_fuse();
    cmd.unset_env("DAPP_LIBRARIES");
    cmd.unset_env("FOUNDRY_LIBRARIES");
    cmd.unset_env("FOUNDRY_CONFIG");
    cmd.unset_env("ETHERSCAN_API_KEY");
    cmd.unset_env("VERIFIER_API_KEY");
    cmd.unset_env("VERIFIER_URL");
    let assert = cmd
        .args([
            "verify-bytecode",
            contract.as_str(),
            "src/LinkedContract.sol:LinkedContract",
            "--rpc-url",
            rpc.as_str(),
            "--libraries",
            first_lib_spec.as_str(),
            "--libraries",
            second_lib_spec.as_str(),
        ])
        .assert_success();
    let output = assert.get_output();
    let stdout = output.stdout_lossy();
    let stderr = output.stderr_lossy();

    assert!(stdout.contains("Runtime code matched with status full"), "{stdout}");
    assert!(stderr.contains("Creation data is unavailable"), "{stderr}");

    // Multi-library environment values continue to be parsed by the configuration provider.
    prj.update_config(|config| config.libraries.clear());
    prj.clear();

    cmd.forge_fuse();
    cmd.env("DAPP_LIBRARIES", format!("{first_lib_spec},{second_lib_spec}"));
    cmd.unset_env("FOUNDRY_LIBRARIES");
    cmd.unset_env("FOUNDRY_CONFIG");
    cmd.unset_env("ETHERSCAN_API_KEY");
    cmd.unset_env("VERIFIER_API_KEY");
    cmd.unset_env("VERIFIER_URL");
    let assert = cmd
        .args([
            "verify-bytecode",
            contract.as_str(),
            "src/LinkedContract.sol:LinkedContract",
            "--rpc-url",
            rpc.as_str(),
        ])
        .assert_success();
    let output = assert.get_output();
    let stdout = output.stdout_lossy();
    let stderr = output.stderr_lossy();

    assert!(stdout.contains("Runtime code matched with status full"), "{stdout}");
    assert!(stderr.contains("Creation data is unavailable"), "{stderr}");
}

#[forgetest_init]
async fn can_verify_bytecode_tempo_aa_deployments(prj: _, cmd: _) {
    prj.initialize_default_contracts();
    // Constructor gas depends on the intrinsic gas of the whole batch, not only the creation call.
    prj.add_source("GasLeft.sol", "contract GasLeft { uint256 public immutable gas = gasleft(); }");

    let (api, handle) =
        anvil::spawn(anvil::NodeConfig::test_tempo().with_chain_id(Some(31337u64))).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let wallet = handle.dev_wallets().next().unwrap();
    let deployer = wallet.address().to_string();
    let pk = hex::encode(wallet.credential().to_bytes());

    // Advance the protocol nonce so it differs from the nonce of each nonce lane below.
    let mut deployments = Vec::new();
    let lanes: [&[&str]; 3] =
        [&[], &["--tempo.nonce-key", "5", "--nonce", "0"], &["--tempo.expires", "30"]];
    for (index, lane) in lanes.into_iter().enumerate() {
        cmd.forge_fuse()
            .args([
                "create",
                "./src/Counter.sol:Counter",
                "--rpc-url",
                rpc.as_str(),
                "--private-key",
                pk.as_str(),
                "--broadcast",
            ])
            .args(lane);
        let output = cmd.assert_success().get_output().stdout_lossy();
        if index == 0 {
            continue;
        }
        let field =
            |prefix| output.lines().find_map(|line| line.strip_prefix(prefix)).unwrap().to_string();
        deployments.push((
            "src/Counter.sol:Counter",
            field("Deployed to: "),
            field("Transaction hash: "),
        ));
    }

    // Batch a creation with a follow-up call.
    cmd.forge_fuse().arg("build").assert_success();
    let artifact: serde_json::Value = serde_json::from_slice(
        &fs::read(prj.paths().artifacts.join("GasLeft.sol/GasLeft.json")).unwrap(),
    )
    .unwrap();
    api.anvil_set_auto_mine(false).await.unwrap();
    let tx_hash: B256 = provider
        .client()
        .request(
            "eth_sendTransaction",
            [serde_json::json!({
                "from": deployer,
                "type": "0x76",
                "gas": "0x1e8480",
                "calls": [
                    {"to": null, "value": "0x0", "input": artifact["bytecode"]["object"]},
                    {"to": Address::with_last_byte(0x22), "value": "0x0", "input": "0x"},
                ],
            })],
        )
        .await
        .unwrap();
    api.mine_one().await.unwrap();
    let receipt: serde_json::Value =
        provider.client().request("eth_getTransactionReceipt", [tx_hash]).await.unwrap();
    assert_eq!(receipt["status"], "0x1");
    deployments.push((
        "src/GasLeft.sol:GasLeft",
        receipt["contractAddress"].as_str().unwrap().to_string(),
        tx_hash.to_string(),
    ));

    for (contract, address, tx_hash) in deployments {
        let creation_data = serde_json::json!({"status":"1", "message":"OK", "result":[{
            "contractAddress": address,
            "contractCreator": deployer,
            "txHash": tx_hash,
        }]});
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/api", listener.local_addr().unwrap());
        let app = Router::new().fallback(move |Query(query): Query<HashMap<String, String>>| {
            let response =
                if query.get("action").is_some_and(|action| action == "getcontractcreation") {
                    creation_data.clone()
                } else {
                    serde_json::json!({"status":"1", "message":"OK", "result":[]})
                };
            async move { Json(response) }
        });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        // The runtime replay must keep the lane's own nonce and the full batch. AA creation code
        // is not read from the batched calls yet, so only runtime is compared.
        for (networks, network_args) in [
            (NetworkConfigs::default(), &[][..]),
            (NetworkConfigs::with_tempo(), &[][..]),
            (NetworkConfigs::with_ethereum(), &["--network", "tempo"][..]),
        ] {
            prj.update_config(|config| config.networks = networks);
            cmd.forge_fuse()
                .args([
                    "verify-bytecode",
                    &address,
                    contract,
                    "--rpc-url",
                    &rpc,
                    "--verifier",
                    "etherscan",
                    "--verifier-url",
                    &url,
                    "--etherscan-api-key",
                    "test",
                    "--ignore",
                    "creation",
                    "--json",
                ])
                .args(network_args)
                .assert_json_stdout(r#"[{"bytecode_type":"runtime", "match_type":"full"}]"#);
        }
        server.abort();
    }
}
