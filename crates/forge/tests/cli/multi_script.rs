//! Contains various tests related to forge script

use crate::utils::KillOnDrop;
use alloy_primitives::{Address, B256, address, hex, keccak256};
use alloy_provider::Provider;
use anvil::{NodeConfig, NodeHandle, spawn};
use foundry_config::Config;
use foundry_test_utils::{
    ScriptOutcome, ScriptTester,
    rpc::{
        spawn_rpc_proxy_blocking_first_submission, spawn_rpc_proxy_recording_method,
        spawn_rpc_proxy_rejecting_method_when_enabled,
    },
    util::TestProject,
};
use serde_json::Value;
use std::{collections::HashSet, sync::atomic::Ordering, time::Duration};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

#[forgetest]
async fn can_deploy_multi_chain_script_without_lib(prj: _, cmd: _) {
    let (api1, handle1) = spawn(NodeConfig::test()).await;
    let (api2, handle2) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());

    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("MultiChainBroadcastNoLink", "deploy(string memory,string memory)")
        .args(&[&handle1.http_endpoint(), &handle2.http_endpoint()])
        .broadcast(ScriptOutcome::OkBroadcast);

    assert_eq!(api1.transaction_count(tester.accounts_pub[0], None).await.unwrap().to::<u32>(), 1);
    assert_eq!(api1.transaction_count(tester.accounts_pub[1], None).await.unwrap().to::<u32>(), 1);

    assert_eq!(api2.transaction_count(tester.accounts_pub[0], None).await.unwrap().to::<u32>(), 2);
    assert_eq!(api2.transaction_count(tester.accounts_pub[1], None).await.unwrap().to::<u32>(), 3);
}

#[forgetest]
async fn can_deploy_multi_chain_script_with_lib(prj: _, cmd: _) {
    let (api1, handle1) = spawn(NodeConfig::test()).await;
    let (api2, handle2) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());

    tester
        .load_private_keys(&[0, 1])
        .await
        .add_deployer(0)
        .add_sig("MultiChainBroadcastLink", "deploy(string memory,string memory)")
        .args(&[&handle1.http_endpoint(), &handle2.http_endpoint()])
        .broadcast(ScriptOutcome::OkBroadcast);

    let deployer = tester.accounts_pub[1];
    for (api, handle) in [(&api1, &handle1), (&api2, &handle2)] {
        assert_eq!(api.transaction_count(deployer, None).await.unwrap().to::<u32>(), 2);
        assert!(!handle.http_provider().get_code_at(deployer.create(1)).await.unwrap().is_empty());
    }
}

#[forgetest]
async fn can_not_deploy_multi_chain_script_with_lib_without_create2_deployer(prj: _, cmd: _) {
    let (_, handle1) = spawn(NodeConfig::test().with_disable_default_create2_deployer(true)).await;
    let (_, handle2) = spawn(NodeConfig::test().with_disable_default_create2_deployer(true)).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle1.http_endpoint(), prj.root());

    tester
        .load_private_keys(&[0, 1])
        .await
        .add_deployer(0)
        .add_sig("MultiChainBroadcastLink", "deploy(string memory,string memory)")
        .args(&[&handle1.http_endpoint(), &handle2.http_endpoint()])
        .broadcast(ScriptOutcome::UnsupportedLibraries);
}

const PROFILE_SCRIPT: &str = r#"
import "forge-std/Script.sol";
library Lib { function value() external pure returns (uint256) { return 1; } }
contract UsesLib { function value() external pure returns (uint256) { return Lib.value(); } }
contract ProfileScript is Script {
    function deployCode() external {
        vm.createSelectFork("first");
        vm.broadcast();
        vm.deployCode("ProfileScript.sol:UsesLib");
        vm.createSelectFork("second");
        vm.broadcast();
        vm.deployCode("ProfileScript.sol:UsesLib");
    }

    function primary() external {
        vm.broadcast();
        vm.deployCode("ProfileScript.sol:UsesLib");
    }

    function create() external {
        vm.createSelectFork("second");
        vm.broadcast();
        new UsesLib();
    }

    function getCode() external {
        vm.createSelectFork("first");
        vm.assertEq(vm.getCode("ProfileScript.sol:UsesLib"), type(UsesLib).creationCode);
        vm.createSelectFork("second");
        vm.assertNotEq(vm.getCode("ProfileScript.sol:UsesLib"), type(UsesLib).creationCode);
        vm.assertNotEq(vm.getDeployedCode("ProfileScript.sol:UsesLib"), type(UsesLib).runtimeCode);
        vm.assertEq(
            vm.getCode("out/ProfileScript.sol/UsesLib.json"), vm.getCode("ProfileScript.sol:UsesLib")
        );
    }

    function tstore() external {
        // Init code running `TSTORE(0, 1)`, which shanghai does not have.
        bytes memory code = hex"600160005d00";
        vm.createSelectFork("second");
        vm.broadcast();
        assembly { pop(create(0, add(code, 0x20), mload(code))) }
    }
}
"#;

const FIRST_LIB: Address = address!("0x1111111111111111111111111111111111111111");
const SECOND_LIB: Address = address!("0x2222222222222222222222222222222222222222");
const PRIVATE_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

#[forgetest_init]
async fn links_endpoint_profile_libraries_on_forks(prj: _, cmd: _) {
    let handles = add_profile_script(&prj, true, "", NodeConfig::test()).await;

    cmd.args(["script", "ProfileScript", "--sig", "deployCode()", "--broadcast"])
        .args(["--private-key", PRIVATE_KEY])
        .assert_success();
    let path = prj.root().join("broadcast/multi/ProfileScript.sol-latest/deployCode.json");
    let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
    let deployments = sequence["deployments"].as_array().unwrap();
    assert_eq!(deployments.len(), 2);
    for (((deployment, handle), library), profile) in
        deployments.iter().zip(&handles).zip([FIRST_LIB, SECOND_LIB]).zip([None, Some("second")])
    {
        assert_eq!(deployment["profile"].as_str(), profile);
        assert_deployed_with_library(deployment, handle, library).await;
    }
}

#[forgetest_init]
async fn deploys_endpoint_profile_libraries_on_rpc_url(prj: _, cmd: _) {
    let handles = add_profile_script(&prj, false, "optimizer = true", NodeConfig::test()).await;

    cmd.args(["script", "ProfileScript", "--sig", "primary()", "--broadcast"])
        .args(["--rpc-url", "second", "--private-key", PRIVATE_KEY])
        .assert_success();
    let path = prj.root().join("broadcast/ProfileScript.sol/31339/primary-latest.json");
    let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
    assert_eq!(sequence["profile"].as_str(), Some("second"));
    let library = sequence["libraries"][0].as_str().unwrap().rsplit(':').next().unwrap();
    let library = library.parse::<Address>().unwrap();
    assert!(!handles[1].http_provider().get_code_at(library).await.unwrap().is_empty());
    assert_deployed_with_library(&sequence, &handles[1], library).await;
}

#[forgetest_init]
async fn can_not_create_run_build_on_endpoint_profile_forks(prj: _, cmd: _) {
    let _handles = add_profile_script(&prj, true, "", NodeConfig::test()).await;

    cmd.args(["script", "ProfileScript", "--sig", "create()", "--broadcast"])
        .args(["--private-key", PRIVATE_KEY])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: script failed: the endpoint's profile builds [..]/src/ProfileScript.sol:UsesLib differently; deploy its build with `vm.deployCode("UsesLib")` instead of `new UsesLib`

"#]]);
}

#[forgetest_init]
async fn can_not_deploy_endpoint_profile_libraries_without_create2_deployer(prj: _, cmd: _) {
    let no_create2_deployer = NodeConfig::test().with_disable_default_create2_deployer(true);
    let _handles = add_profile_script(&prj, false, "", no_create2_deployer).await;

    cmd.args(["script", "ProfileScript", "--sig", "deployCode()", "--broadcast"])
        .args(["--private-key", PRIVATE_KEY, "--non-interactive"])
        .assert_failure()
        .stderr_eq(str![[r#"
Warning: Multi chain deployment is still under development. Use with caution.
Warning: Script contains a transaction to [..] which does not contain any code.
Error: CREATE2 deployer 0x4e59b44847b379578588920cA78FbF26c0B4956C is not deployed on chain 31339

"#]]);
}

#[forgetest_init]
async fn uses_endpoint_profile_artifacts_on_forks(prj: _, cmd: _) {
    let _handles = add_profile_script(&prj, true, "", NodeConfig::test()).await;

    cmd.args(["script", "ProfileScript", "--sig", "getCode()"]).assert_success();
}

#[forgetest_init]
async fn simulates_with_endpoint_profile_evm_version(prj: _, cmd: _) {
    let _handles =
        add_profile_script(&prj, true, r#"evm_version = "shanghai""#, NodeConfig::test()).await;

    // Execution uses the run's cancun spec, the simulation on `second` its profile's shanghai.
    cmd.args(["script", "ProfileScript", "--sig", "tstore()"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
Script ran successfully.
...
## Setting up 1 EVM.
  [106] → new <unknown>@0x5b73C5498c1E3b4dbA84de0F1833c4a029d90519
    └─ ← [NotActivated]


"#]])
        .stderr_eq(str![[r#"
Error: Failed with `EvmError: NotActivated`:

Error: Simulated execution failed.

"#]]);
}

#[forgetest_init]
async fn can_resume_endpoint_profile_script_on_forks(prj: _, cmd: _) {
    let handles = add_profile_script(&prj, true, "", NodeConfig::test()).await;

    cmd.args(["script", "ProfileScript", "--sig", "deployCode()", "--broadcast"])
        .args(["--sender", "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"])
        .assert_failure()
        .stderr_eq(str![[r#"
Warning: Multi chain deployment is still under development. Use with caution.
Error: No associated wallet for addresses: [0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266]. Unlocked wallets: []

"#]]);
    cmd.forge_fuse()
        .args(["script", "ProfileScript", "--sig", "deployCode()", "--broadcast", "--resume"])
        .args(["--multi", "--private-key", PRIVATE_KEY])
        .assert_success();
    let path = prj.root().join("broadcast/multi/ProfileScript.sol-latest/deployCode.json");
    let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
    let deployments = sequence["deployments"].as_array().unwrap();
    assert_eq!(deployments[1]["profile"].as_str(), Some("second"));
    assert_deployed_with_library(&deployments[1], &handles[1], SECOND_LIB).await;
}

#[forgetest]
async fn can_not_change_fork_during_broadcast(prj: _, cmd: _) {
    let (_, handle1) = spawn(NodeConfig::test()).await;
    let (_, handle2) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());

    tester
        .load_private_keys(&[0, 1])
        .await
        .add_deployer(0)
        .add_sig("MultiChainBroadcastNoLink", "deployError(string memory,string memory)")
        .args(&[&handle1.http_endpoint(), &handle2.http_endpoint()])
        .broadcast(ScriptOutcome::ErrorSelectForkOnBroadcast);
}

#[forgetest]
async fn can_resume_multi_chain_script(prj: _, cmd: _) {
    let (_, handle1) = spawn(NodeConfig::test()).await;
    let (_, handle2) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());

    tester
        .add_sig("MultiChainBroadcastNoLink", "deploy(string memory,string memory)")
        .args(&[&handle1.http_endpoint(), &handle2.http_endpoint()])
        .broadcast(ScriptOutcome::MissingWallet)
        .load_private_keys(&[0, 1])
        .await
        .arg("--multi")
        .resume(ScriptOutcome::OkBroadcast);
}

#[forgetest]
async fn resume_multi_chain_does_not_replay_completed_chain(prj: _, cmd: _) {
    let (api1, handle1) = spawn(NodeConfig::test()).await;
    let (api2, handle2) = spawn(NodeConfig::test()).await;
    let (rpc1, chain1_submissions) =
        spawn_rpc_proxy_recording_method(handle1.http_endpoint(), "eth_sendRawTransaction").await;
    let (recording_rpc2, chain2_submissions) =
        spawn_rpc_proxy_recording_method(handle2.http_endpoint(), "eth_sendRawTransaction").await;
    let (rpc2, reject_chain2) =
        spawn_rpc_proxy_rejecting_method_when_enabled(recording_rpc2, "eth_sendRawTransaction")
            .await;
    reject_chain2.store(true, Ordering::SeqCst);

    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());
    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("MultiChainBroadcastNoLink", "deploy(string memory,string memory)")
        .args(&[&rpc1, &rpc2])
        .arg("--broadcast");
    let stderr =
        String::from_utf8_lossy(&tester.cmd.assert_failure().get_output().stderr).into_owned();
    assert!(stderr.contains("method is not allowed"), "{stderr}");

    assert_eq!(api1.transaction_count(tester.accounts_pub[0], None).await.unwrap().to::<u32>(), 1);
    assert_eq!(api1.transaction_count(tester.accounts_pub[1], None).await.unwrap().to::<u32>(), 1);
    assert_eq!(api2.transaction_count(tester.accounts_pub[0], None).await.unwrap().to::<u32>(), 0);
    assert_eq!(api2.transaction_count(tester.accounts_pub[1], None).await.unwrap().to::<u32>(), 0);
    assert_eq!(chain1_submissions.lock().unwrap().len(), 2);
    assert!(chain2_submissions.lock().unwrap().is_empty());

    let path = foundry_common::fs::json_files(&prj.root().join("broadcast/multi"))
        .find(|path| path.to_string_lossy().contains("-latest"))
        .expect("no latest multi-chain broadcast artifact");
    let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
    let deployments = sequence["deployments"].as_array().unwrap();
    assert_eq!(deployments.len(), 2);
    assert_eq!(deployments[0]["receipts"].as_array().unwrap().len(), 2);
    assert!(deployments[1]["receipts"].as_array().unwrap().is_empty());
    let completed_operations = deployments[0]["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|transaction| {
            (
                transaction["hash"].clone(),
                transaction["contractAddress"].clone(),
                transaction["transaction"]["from"].clone(),
                transaction["transaction"]["input"].clone(),
                transaction["transaction"]["nonce"].clone(),
                transaction["transaction"]["chainId"].clone(),
            )
        })
        .collect::<Vec<_>>();
    let completed_receipts = deployments[0]["receipts"].clone();

    reject_chain2.store(false, Ordering::SeqCst);
    tester.clear();
    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("MultiChainBroadcastNoLink", "deploy(string memory,string memory)")
        .args(&[&rpc1, &rpc2])
        .arg("--multi")
        .arg("--resume");
    tester.cmd.assert_success();

    assert_eq!(api1.transaction_count(tester.accounts_pub[0], None).await.unwrap().to::<u32>(), 1);
    assert_eq!(api1.transaction_count(tester.accounts_pub[1], None).await.unwrap().to::<u32>(), 1);
    assert_eq!(api2.transaction_count(tester.accounts_pub[0], None).await.unwrap().to::<u32>(), 2);
    assert_eq!(api2.transaction_count(tester.accounts_pub[1], None).await.unwrap().to::<u32>(), 3);
    assert_eq!(chain1_submissions.lock().unwrap().len(), 2);
    assert_eq!(chain2_submissions.lock().unwrap().len(), 5);

    let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
    let deployments = sequence["deployments"].as_array().unwrap();
    let resumed_completed_operations = deployments[0]["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|transaction| {
            (
                transaction["hash"].clone(),
                transaction["contractAddress"].clone(),
                transaction["transaction"]["from"].clone(),
                transaction["transaction"]["input"].clone(),
                transaction["transaction"]["nonce"].clone(),
                transaction["transaction"]["chainId"].clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(resumed_completed_operations, completed_operations);
    assert_eq!(deployments[0]["receipts"], completed_receipts);
    assert_eq!(deployments[0]["receipts"].as_array().unwrap().len(), 2);
    assert_eq!(deployments[1]["receipts"].as_array().unwrap().len(), 5);
    assert!(
        deployments.iter().all(|deployment| deployment["pending"].as_array().unwrap().is_empty())
    );
    for (deployment, handle) in deployments.iter().zip([&handle1, &handle2]) {
        let provider = handle.http_provider();
        for transaction in deployment["transactions"].as_array().unwrap() {
            let address = transaction["contractAddress"]
                .as_str()
                .expect("deployment transaction is missing its contract address")
                .parse::<Address>()
                .unwrap();
            assert!(!provider.get_code_at(address).await.unwrap().is_empty());
        }
    }
}

#[forgetest]
async fn resume_multi_chain_after_lost_submission_response(prj: _, cmd: _) {
    let (api1, handle1) = spawn(NodeConfig::test()).await;
    let (api2, handle2) = spawn(NodeConfig::test()).await;
    let (rpc1, chain1_submissions) =
        spawn_rpc_proxy_recording_method(handle1.http_endpoint(), "eth_sendRawTransaction").await;
    let (rpc2, chain2_submissions, reached, release) = spawn_rpc_proxy_blocking_first_submission(
        handle2.http_endpoint(),
        "eth_sendRawTransaction",
        true,
    )
    .await;
    let planned_operations = |sequence: &Value| {
        sequence["deployments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|deployment| {
                deployment["transactions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|transaction| {
                        (
                            transaction["contractAddress"].clone(),
                            transaction["transaction"]["from"].clone(),
                            transaction["transaction"]["input"].clone(),
                            transaction["transaction"]["nonce"].clone(),
                            transaction["transaction"]["chainId"].clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };

    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());
    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("MultiChainBroadcastNoLink", "deploy(string memory,string memory)")
        .args(&[&rpc1, &rpc2])
        .arg("--broadcast");
    let mut child = KillOnDrop::spawn(tester.cmd.cmd());
    tokio::time::timeout(Duration::from_secs(60), reached.notified())
        .await
        .expect("Forge did not submit to chain 2");

    // Chain 2 accepted and mined the first submission, but Forge never received the response.
    let accepted = chain2_submissions.lock().unwrap()[0][0].clone();
    let accepted_hash = keccak256(hex::decode(accepted.as_str().unwrap()).unwrap());
    tokio::time::timeout(Duration::from_secs(30), async {
        while handle2
            .http_provider()
            .get_transaction_receipt(accepted_hash)
            .await
            .unwrap()
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("chain 2 did not mine the accepted submission");
    assert_eq!(chain1_submissions.lock().unwrap().len(), 2);
    let recovery_path = foundry_common::fs::json_files(&prj.root().join("cache"))
        .find(|path| path.to_string_lossy().ends_with(".recovery.json"))
        .expect("no authoritative recovery snapshot");
    let recovery: Value = foundry_common::fs::read_json_file(&recovery_path).unwrap();
    let attempt = &recovery["deployments"][1]["attempts"][0];
    assert_eq!(attempt["kind"]["kind"], "signed");
    assert_eq!(attempt["kind"]["payload"]["payload"], accepted);
    let path = foundry_common::fs::json_files(&prj.root().join("broadcast/multi"))
        .find(|path| path.to_string_lossy().contains("-latest"))
        .expect("no latest multi-chain broadcast artifact");
    let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
    let planned = planned_operations(&sequence);
    assert_eq!(planned.iter().map(Vec::len).collect::<Vec<_>>(), [2, 5]);
    // Forge is still waiting for the first chain 2 response and has not recorded its outcome.
    assert_eq!(chain2_submissions.lock().unwrap().len(), 1);
    assert!(sequence["deployments"][1]["transactions"][0]["hash"].is_null());
    assert!(sequence["deployments"][1]["pending"].as_array().unwrap().is_empty());
    assert!(sequence["deployments"][1]["receipts"].as_array().unwrap().is_empty());

    assert!(child.is_running(), "Forge exited before it could be interrupted");
    let output = child.kill_and_wait();
    assert!(!output.status.success(), "Forge unexpectedly succeeded");
    #[cfg(unix)]
    assert_eq!(output.status.signal(), Some(9), "Forge was not terminated by SIGKILL");
    release.notify_one();

    tester.clear();
    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("MultiChainBroadcastNoLink", "deploy(string memory,string memory)")
        .args(&[&rpc1, &rpc2])
        .arg("--multi")
        .arg("--resume");
    tester.cmd.assert_success();

    // Chain 1 is not resubmitted, and chain 2 reconciles its mined operation instead of resending
    // it, so exactly one payload is submitted per operation.
    assert_eq!(chain1_submissions.lock().unwrap().len(), 2);
    assert_eq!(chain2_submissions.lock().unwrap().len(), 5);
    let chain2_payloads = chain2_submissions
        .lock()
        .unwrap()
        .iter()
        .map(|params| params[0].as_str().unwrap().to_string())
        .collect::<HashSet<_>>();
    assert_eq!(chain2_payloads.len(), 5);
    assert_eq!(api1.transaction_count(tester.accounts_pub[0], None).await.unwrap().to::<u32>(), 1);
    assert_eq!(api1.transaction_count(tester.accounts_pub[1], None).await.unwrap().to::<u32>(), 1);
    assert_eq!(api2.transaction_count(tester.accounts_pub[0], None).await.unwrap().to::<u32>(), 2);
    assert_eq!(api2.transaction_count(tester.accounts_pub[1], None).await.unwrap().to::<u32>(), 3);

    let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
    assert_eq!(planned_operations(&sequence), planned);
    let deployments = sequence["deployments"].as_array().unwrap();
    assert_eq!(
        deployments[1]["transactions"][0]["hash"].as_str().unwrap().parse::<B256>().unwrap(),
        accepted_hash
    );
    assert_eq!(deployments[0]["receipts"].as_array().unwrap().len(), 2);
    assert_eq!(deployments[1]["receipts"].as_array().unwrap().len(), 5);
    assert!(
        deployments.iter().all(|deployment| deployment["pending"].as_array().unwrap().is_empty())
    );
    for (deployment, handle) in deployments.iter().zip([&handle1, &handle2]) {
        let provider = handle.http_provider();
        for transaction in deployment["transactions"].as_array().unwrap() {
            let address = transaction["contractAddress"].as_str().unwrap().parse().unwrap();
            assert!(!provider.get_code_at(address).await.unwrap().is_empty());
        }
    }
}

/// Adds `ProfileScript` with endpoints `first` and `second`, which names the `second` profile.
///
/// With `configured_libraries`, `Lib` has a different configured address in each profile;
/// otherwise both profiles deploy it through the CREATE2 deployer.
async fn add_profile_script(
    prj: &TestProject,
    configured_libraries: bool,
    second_profile: &str,
    second_node: NodeConfig,
) -> [NodeHandle; 2] {
    let (_, first) = spawn(NodeConfig::test().with_chain_id(Some(31338u64))).await;
    let (_, second) = spawn(second_node.with_chain_id(Some(31339u64))).await;
    prj.add_source("ProfileScript.sol", PROFILE_SCRIPT);
    let [first_lib, second_lib] = [FIRST_LIB, SECOND_LIB].map(|address| {
        if configured_libraries {
            format!(r#"libraries = ["src/ProfileScript.sol:Lib:{address}"]"#)
        } else {
            String::new()
        }
    });
    // `unset` and `offline` are never forked, so they must not be resolved or contacted.
    prj.create_file(
        Config::FILE_NAME,
        &format!(
            r#"
[profile.default]
evm_version = "cancun"
{first_lib}

[profile.second]
{second_lib}
{second_profile}

[rpc_endpoints]
first = "{}"
second = {{ endpoint = "{}", profile = "second" }}
unset = {{ endpoint = "${{UNSET_PROFILE_RPC_URL}}", profile = "second" }}
offline = {{ endpoint = "http://127.0.0.1:1", profile = "second" }}
"#,
            first.http_endpoint(),
            second.http_endpoint()
        ),
    );
    [first, second]
}

/// Asserts that the last contract deployed by `sequence` on `handle` is linked to `library`.
async fn assert_deployed_with_library(sequence: &Value, handle: &NodeHandle, library: Address) {
    let address = sequence["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tx| tx["contractAddress"].as_str())
        .map(|address| address.parse::<Address>().unwrap())
        .next_back()
        .unwrap();
    let code = handle.http_provider().get_code_at(address).await.unwrap();
    assert!(code.windows(20).any(|window| window == library.as_slice()));
}
