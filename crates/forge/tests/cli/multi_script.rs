//! Contains various tests related to forge script

use crate::utils::KillOnDrop;
use alloy_primitives::{Address, B256, hex, keccak256};
use alloy_provider::Provider;
use anvil::{NodeConfig, spawn};
use foundry_test_utils::{
    ScriptOutcome, ScriptTester,
    rpc::{
        spawn_rpc_proxy_blocking_first_submission, spawn_rpc_proxy_recording_method,
        spawn_rpc_proxy_rejecting_method_when_enabled,
    },
    str,
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

#[forgetest_init]
async fn uses_endpoint_profile_builds_on_forks(prj: _, cmd: _) {
    let first_lib = Address::repeat_byte(0x11);
    let second_lib = Address::repeat_byte(0x22);
    let (_, first) = spawn(NodeConfig::test().with_chain_id(Some(31338u64))).await;
    let (_, second) = spawn(NodeConfig::test().with_chain_id(Some(31339u64))).await;
    prj.add_source(
        "ProfileScript.sol",
        r#"
import "forge-std/Script.sol";
library Lib { function value() external pure returns (uint256) { return 1; } }
contract UsesLib { function value() external pure returns (uint256) { return Lib.value(); } }
library AutoLib { function value() external pure returns (uint256) { return 2; } }
contract UsesAutoLib {
    uint256 x;
    constructor(uint256 x_) { x = x_; }
    function value() external view returns (uint256) { return AutoLib.value() + x; }
}
contract Creator {
    function created(bytes memory initCode) external returns (bool ok) {
        assembly { ok := iszero(iszero(create(0, add(initCode, 0x20), mload(initCode)))) }
    }
}
bytes constant PUSH0 = hex"5f00";
bytes constant TSTORE = hex"600160005d00";
contract ProfileScript is Script {
    function run() external {
        vm.createSelectFork("first");
        Creator creator = new Creator();
        vm.assertTrue(creator.created{gas: 1e6}(hex"00") && !creator.created{gas: 1e6}(PUSH0));
        bytes memory firstCode = vm.getCode("ProfileScript.sol:UsesLib");
        deploy(1);
        vm.createSelectFork("second");
        vm.assertTrue(new Creator().created{gas: 1e6}(TSTORE));
        vm.assertNotEq(vm.getCode("ProfileScript.sol:UsesLib"), firstCode);
        vm.assertEq(
            vm.getCode("out/ProfileScript.sol/UsesLib.json"), vm.getCode("ProfileScript.sol:UsesLib")
        );
        deploy(2);
    }

    function deploy(uint256 x) internal {
        bytes32 salt = bytes32(x);
        bytes memory initCode =
            abi.encodePacked(vm.getCode("ProfileScript.sol:UsesAutoLib"), abi.encode(x));
        vm.startBroadcast();
        vm.deployCode("ProfileScript.sol:UsesLib");
        UsesAutoLib created_ = new UsesAutoLib(x);
        UsesAutoLib salted = new UsesAutoLib{salt: salt}(x);
        vm.stopBroadcast();
        vm.assertEq(address(created_).code, vm.getDeployedCode("ProfileScript.sol:UsesAutoLib"));
        vm.assertEq(address(salted), vm.computeCreate2Address(salt, keccak256(initCode)));
        vm.assertEq(created_.value(), 2 + x);
    }

    function single() external {
        vm.broadcast();
        vm.deployCode("ProfileScript.sol:UsesLib");
    }
}
"#,
    );
    prj.add_test(
        "ProfileFork.t.sol",
        r#"
import "forge-std/Test.sol";
import "../src/ProfileScript.sol";
contract ProfileForkTest is Test {
    function test_fork_profile() public {
        bytes memory runCode = address(new UsesLib()).code;
        assertFalse(new Creator().created{gas: 1e6}(TSTORE));
        vm.createSelectFork("second");
        assertTrue(new Creator().created{gas: 1e6}(TSTORE));
        bytes memory code = address(new UsesLib()).code;
        assertEq(code, vm.getDeployedCode("ProfileScript.sol:UsesLib"));
        assertNotEq(code, runCode);
    }
}
"#,
    );
    // `unset` and `offline` are never forked, so they must not be resolved or contacted.
    prj.create_file(
        "foundry.toml",
        &format!(
            r#"
[profile.default]
evm_version = "shanghai"

[profile.first]
evm_version = "paris"
libraries = ["src/ProfileScript.sol:Lib:{first_lib}"]

[profile.second]
evm_version = "cancun"
libraries = ["src/ProfileScript.sol:Lib:{second_lib}"]

[rpc_endpoints]
first = {{ endpoint = "{}", profile = "first" }}
second = {{ endpoint = "{}", profile = "second" }}
unset = {{ endpoint = "${{UNSET_PROFILE_RPC_URL}}", profile = "second" }}
offline = {{ endpoint = "http://127.0.0.1:1", profile = "second" }}
"#,
            first.http_endpoint(),
            second.http_endpoint()
        ),
    );
    let read = |path: &str| -> Value {
        foundry_common::fs::read_json_file(&prj.root().join("broadcast").join(path)).unwrap()
    };
    // Returns the code of the contract deployed by transaction `index` of `sequence`.
    let code = async |handle: &anvil::NodeHandle, sequence: &Value, index: usize| {
        let address = sequence["transactions"][index]["contractAddress"].as_str().unwrap();
        handle.http_provider().get_code_at(address.parse().unwrap()).await.unwrap()
    };
    let linked =
        |code: &[u8], library: Address| code.windows(20).any(|window| window == library.as_slice());

    cmd.args(["script", "ProfileScript", "--broadcast", "--private-key", PRIVATE_KEY])
        .assert_success();
    let deployments = &read("multi/ProfileScript.sol-latest/run.json")["deployments"];
    for (handle, deployment, library, x) in
        [(&first, &deployments[0], first_lib, 1u8), (&second, &deployments[1], second_lib, 2)]
    {
        // The CREATE2 library deployment, `vm.deployCode`, `new` and `new` with salt.
        assert_eq!(deployment["transactions"].as_array().unwrap().len(), 4);
        assert!(linked(&code(handle, deployment, 1).await, library));
        let uses_auto_lib = deployment["transactions"][2]["contractAddress"].as_str().unwrap();
        let value = handle
            .http_provider()
            .call(
                alloy_rpc_types::TransactionRequest::default()
                    .to(uses_auto_lib.parse().unwrap())
                    .input(hex!("3fa4f245").to_vec().into())
                    .into(),
            )
            .await
            .unwrap();
        assert_eq!(value[..], B256::with_last_byte(2 + x)[..]);
        assert_eq!(code(handle, deployment, 2).await, code(handle, deployment, 3).await);
    }

    // The `--rpc-url` endpoint uses its profile build too.
    cmd.forge_fuse()
        .args(["script", "ProfileScript", "--sig", "single()", "--rpc-url", "second"])
        .args(["--broadcast", "--private-key", PRIVATE_KEY])
        .assert_success();
    let sequence = read("ProfileScript.sol/31339/single-latest.json");
    let last = sequence["transactions"].as_array().unwrap().len() - 1;
    assert!(linked(&code(&second, &sequence, last).await, second_lib));

    // Tests use profile builds and specs on forks too, and profile builds are cached.
    cmd.forge_fuse().args(["test", "--mt", "test_fork_profile"]).assert_success();
    cmd.forge_fuse().args(["test", "--mt", "test_fork_profile"]).assert_success().stdout_eq(str![
        [r#"
No files changed, compilation skipped
No files changed, compilation skipped
No files changed, compilation skipped

Ran 1 test for test/ProfileFork.t.sol:ProfileForkTest
[PASS] test_fork_profile() ([GAS])
...
"#]
    ]);
}

const PRIVATE_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

#[forgetest_init]
async fn endpoint_profile_fork_preserves_recorded_initcode_and_metadata(prj: _, cmd: _) {
    let (_, node) = spawn(NodeConfig::test()).await;
    prj.add_source(
        "Recorded.sol",
        r#"
import "forge-std/Script.sol";
library RecordedLib { function value() external pure returns (uint256) { return 7; } }
contract RecordedChild {
    uint256 public value;
    constructor(uint256 x) { value = RecordedLib.value() + x; }
}
contract RecordedPlain { function value() external pure returns (uint256) { return 9; } }
contract RecordedFactory {
    function deploy(bytes memory code) external returns (address result) {
        assembly { result := create(0, add(code, 32), mload(code)) }
    }
}
contract RecordedScript is Script {
    function run() external {
        vm.startBroadcast();
        RecordedChild direct = new RecordedChild(42);
        RecordedChild salted = new RecordedChild{salt: bytes32(block.number)}(43);
        RecordedFactory factory = new RecordedFactory();
        address nested = factory.deploy(type(RecordedPlain).creationCode);
        vm.stopBroadcast();
        require(direct.value() == 49 && salted.value() == 50);
        require(keccak256(nested.code) == keccak256(type(RecordedPlain).runtimeCode));
    }
}
"#,
    );
    prj.create_file(
        "foundry.toml",
        &format!(
            r#"
[profile.default]
optimizer = false
[profile.endpoint]
optimizer = true
optimizer_runs = 777
bytecode_hash = "none"
create2_library_salt = "0x0000000000000000000000000000000000000000000000000000000000000042"
[rpc_endpoints]
local = {{ endpoint = "{}", profile = "endpoint" }}
"#,
            node.http_endpoint()
        ),
    );
    for skip_simulation in [false, true] {
        cmd.forge_fuse().args([
            "script",
            "RecordedScript",
            "--rpc-url",
            "local",
            "--broadcast",
            "--private-key",
            PRIVATE_KEY,
        ]);
        if skip_simulation {
            cmd.arg("--skip-simulation");
        }
        cmd.assert_success();
        let sequence: Value = foundry_common::fs::read_json_file(
            &prj.root().join("broadcast/Recorded.sol/31337/run-latest.json"),
        )
        .unwrap();
        let children = sequence["transactions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|tx| tx["contractName"] == "RecordedChild")
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0]["arguments"], serde_json::json!(["42"]));
        assert_eq!(children[1]["arguments"], serde_json::json!(["43"]));
        for (tx, expected) in children.into_iter().zip([49, 50]) {
            let address = tx["contractAddress"].as_str().unwrap().parse().unwrap();
            let value = node
                .http_provider()
                .call(
                    alloy_rpc_types::TransactionRequest::default()
                        .to(address)
                        .input(hex!("3fa4f245").to_vec().into())
                        .into(),
                )
                .await
                .unwrap();
            assert_eq!(value[..], B256::with_last_byte(expected)[..]);
        }
    }
}

#[forgetest_init]
fn endpoint_profiles_work_in_coverage_and_mutation(prj: _, cmd: _) {
    prj.add_source("ProfileCounter.sol", "contract ProfileCounter { function value() external pure returns (uint256) { return 1; } }");
    prj.add_test(
        "ProfileCounter.t.sol",
        r#"
import "forge-std/Test.sol";
import "../src/ProfileCounter.sol";
contract ProfileCounterTest is Test {
    function test_value() public { assertEq(new ProfileCounter().value(), 1); }
}
"#,
    );
    prj.create_file(
        "foundry.toml",
        r#"
[profile.other]
optimizer = true
[rpc_endpoints]
unused = { endpoint = "http://127.0.0.1:1", profile = "other" }
"#,
    );
    cmd.args(["coverage", "--match-contract", "ProfileCounterTest"]).assert_success();
    cmd.forge_fuse()
        .args([
            "test",
            "--mutate",
            "src/ProfileCounter.sol",
            "--mutation-jobs",
            "1",
            "--match-contract",
            "ProfileCounterTest",
            "--json",
        ])
        .assert_json_stdout(str![[r#"
{"summary":{"total":1,"killed":1,"survived":0,"invalid":0,"skipped":0,"timed_out":0,"mutation_score":100.0,"duration_secs":"{...}"},"survived_mutants":{}}
"#]]);
}
