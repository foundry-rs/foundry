//! Contains various tests related to forge script

use crate::utils::KillOnDrop;
use alloy_primitives::{Address, B256, U256, address, bytes, hex, keccak256};
use alloy_provider::Provider;
use anvil::{NodeConfig, spawn};
use foundry_test_utils::{
    ScriptOutcome, ScriptTester,
    rpc::{
        spawn_rpc_proxy_blocking_first_submission, spawn_rpc_proxy_mapping_method,
        spawn_rpc_proxy_recording_method, spawn_rpc_proxy_rejecting_method_when_enabled,
    },
    util::{TestCommand, TestProject},
};
use serde_json::Value;
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

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
async fn can_not_deploy_multi_chain_script_with_lib(prj: _, cmd: _) {
    let (_, handle1) = spawn(NodeConfig::test()).await;
    let (_, handle2) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());

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
    tester.cmd.assert_failure().stderr_eq(str![[r#"
Warning: Multi chain deployment is still under development. Use with caution.
Error: chain 31337 (sequence #2): Failed to send transaction after 4 attempts Err(HTTP error 403 with body: {"jsonrpc":"2.0","id":[..],"error":{"code":-32004,"message":"method is not allowed"}}); HTTP error 403 with body: {"jsonrpc":"2.0","id":[..],"error":{"code":-32004,"message":"method is not allowed"}}

"#]]);

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

    // A new broadcast refuses to replace the unfinished one, so the completed chain is not
    // redeployed.
    tester.clear();
    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("MultiChainBroadcastNoLink", "deploy(string memory,string memory)")
        .args(&[&rpc1, &rpc2])
        .arg("--broadcast");
    tester.cmd.assert_failure().stderr_eq(str![[r#"
Warning: Multi chain deployment is still under development. Use with caution.
Error: the previous broadcast `[..]/broadcast/multi/[..]-latest/deploy.json` has unfinished operations; finish it with `--multi --resume`, or pass `--start-fresh` to archive it and broadcast anew

"#]]);
    assert_eq!(chain1_submissions.lock().unwrap().len(), 2);
    assert!(chain2_submissions.lock().unwrap().is_empty());

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

#[forgetest]
async fn multi_resume_warns_that_rpc_url_is_not_used(prj: _, cmd: _) {
    let (_, handle1) = spawn(NodeConfig::test()).await;
    let (_, handle2) = spawn(NodeConfig::test()).await;
    let (recording_rpc2, chain2_submissions) =
        spawn_rpc_proxy_recording_method(handle2.http_endpoint(), "eth_sendRawTransaction").await;
    let (rpc2, reject_chain2) =
        spawn_rpc_proxy_rejecting_method_when_enabled(recording_rpc2, "eth_sendRawTransaction")
            .await;
    let (rpc_flag, flag_submissions) =
        spawn_rpc_proxy_recording_method(handle2.http_endpoint(), "eth_sendRawTransaction").await;
    reject_chain2.store(true, Ordering::SeqCst);

    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());
    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("MultiChainBroadcastNoLink", "deploy(string memory,string memory)")
        .args(&[&handle1.http_endpoint(), &rpc2])
        .arg("--broadcast");
    tester.cmd.assert_failure();
    assert!(chain2_submissions.lock().unwrap().is_empty());

    // The flag does not move chain 2 off its saved endpoint.
    reject_chain2.store(false, Ordering::SeqCst);
    tester.clear();
    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("MultiChainBroadcastNoLink", "deploy(string memory,string memory)")
        .args(&[&handle1.http_endpoint(), &rpc2])
        .args(&["--multi", "--resume", "--rpc-url", &rpc_flag]);
    tester.cmd.assert_success().stderr_eq(str![[r#"
Warning: Multichain resume submits and checks transactions through the RPC endpoints saved by the original run; `--rpc-url` is not used for that.

Deployed contracts (chain 31337):
  NoLink  0x5FbDB2315678afecb367f032d93F642f64180aa3
  NoLink  0x8464135c8F25Da09e49BC8782676a84730C318bC

Deployed contracts (chain 31337):
  NoLink  0x8464135c8F25Da09e49BC8782676a84730C318bC
  NoLink  0x71C95911E9a5D330f4D621842EC243EE1343292e
  NoLink  0x948B3c65b89DF0B4894ABE91E6D02FE579834F8F
  NoLink  0x5FbDB2315678afecb367f032d93F642f64180aa3
  NoLink  0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512

"#]]);
    assert!(flag_submissions.lock().unwrap().is_empty());
    assert_eq!(chain2_submissions.lock().unwrap().len(), 5);

    // An RPC URL configured in `foundry.toml` is not the flag and does not warn.
    prj.update_config(|config| config.eth_rpc_url = Some(rpc_flag.clone()));
    tester.clear();
    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("MultiChainBroadcastNoLink", "deploy(string memory,string memory)")
        .args(&[&handle1.http_endpoint(), &rpc2])
        .args(&["--multi", "--resume"]);
    tester.cmd.assert_success().stderr_eq(str![[r#"

Deployed contracts (chain 31337):
  NoLink  0x5FbDB2315678afecb367f032d93F642f64180aa3
  NoLink  0x8464135c8F25Da09e49BC8782676a84730C318bC

Deployed contracts (chain 31337):
  NoLink  0x8464135c8F25Da09e49BC8782676a84730C318bC
  NoLink  0x71C95911E9a5D330f4D621842EC243EE1343292e
  NoLink  0x948B3c65b89DF0B4894ABE91E6D02FE579834F8F
  NoLink  0x5FbDB2315678afecb367f032d93F642f64180aa3
  NoLink  0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512

"#]]);
}

// A failing chain on `--multi --resume` does not stop later chains from being reconciled, and the
// error names the failing chain.
#[forgetest_init]
async fn multi_resume_reconciles_every_chain_before_failing(prj: _, cmd: _) {
    let gate = address!("0x000000000000000000000000000000000000bEEF");
    let script = prj.add_script(
        "MultiResumeRevert.s.sol",
        r#"
import "forge-std/Script.sol";

contract MultiResumeRevert is Script {
    function run(string memory rpc1, string memory rpc2) external {
        vm.createSelectFork(rpc1);
        vm.startBroadcast();
        (bool success,) = address(0x000000000000000000000000000000000000bEEF).call("");
        require(success);
        vm.stopBroadcast();

        vm.createSelectFork(rpc2);
        vm.startBroadcast();
        payable(address(0xdEaD)).transfer(1);
        vm.stopBroadcast();
    }
}
"#,
    );
    let (api1, handle1) = spawn(NodeConfig::test().with_chain_id(Some(1111u64))).await;
    let (api2, handle2) = spawn(NodeConfig::test().with_chain_id(Some(2222u64))).await;
    // Runtime that reverts once storage slot 0 is set.
    api1.anvil_set_code(gate, bytes!("60005415600b57600080fd5b00")).await.unwrap();
    api1.anvil_set_auto_mine(false).await.unwrap();
    api2.anvil_set_auto_mine(false).await.unwrap();
    // While hidden, chain 1111's transaction looks dropped, so the broadcast moves on to 2222.
    let hide = Arc::new(AtomicBool::new(true));
    let rpc1 =
        spawn_rpc_proxy_mapping_method(handle1.http_endpoint(), "eth_getTransactionByHash", {
            let hide = hide.clone();
            move |_, result| if hide.load(Ordering::SeqCst) { Value::Null } else { result }
        })
        .await;
    // Chain 2222's transaction is looked up only once it has been checkpointed.
    let (rpc2, lookups) =
        spawn_rpc_proxy_recording_method(handle2.http_endpoint(), "eth_getTransactionByHash").await;
    let args = [
        "script",
        script.to_str().unwrap(),
        "--sig",
        "run(string,string)",
        &rpc1,
        &rpc2,
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--timeout",
        "1",
    ];
    let path = prj.root().join("broadcast/multi/MultiResumeRevert.s.sol-latest/run.json");

    // Interrupt the broadcast once chain 2222's transaction is pending.
    cmd.forge_fuse().args(args).arg("--broadcast");
    let child = KillOnDrop::spawn(cmd.cmd());
    tokio::time::timeout(Duration::from_secs(60), async {
        while lookups.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("chain 2222's transaction was not checkpointed");
    child.kill_and_wait();
    let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
    assert!(sequence["deployments"][0]["pending"].as_array().unwrap().is_empty());
    let pending = sequence["deployments"][1]["pending"][0].as_str().unwrap();
    let reverted = sequence["deployments"][0]["transactions"][0]["hash"].as_str().unwrap();

    // Chain 1111's transaction reverts and chain 2222's succeeds.
    hide.store(false, Ordering::SeqCst);
    api1.anvil_set_storage_at(gate, U256::ZERO, B256::with_last_byte(1)).await.unwrap();
    api1.mine_one().await.unwrap();
    api2.mine_one().await.unwrap();

    cmd.forge_fuse().args(args).args(["--multi", "--resume"]);
    cmd.assert_failure().stderr_eq(format!("Error: chain 1111: Transaction Failure: {reverted}\n"));
    let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
    let deployments = sequence["deployments"].as_array().unwrap();
    assert_eq!(deployments[0]["reverted"], serde_json::json!([reverted]));
    assert!(deployments[1]["pending"].as_array().unwrap().is_empty());
    let receipts = deployments[1]["receipts"].as_array().unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0]["transactionHash"], pending);
    assert_eq!(receipts[0]["status"], "0x1");
}

/// Calls a gate on the first chain, requires it to succeed, then transfers 1 wei on the second.
const MULTI_RESUME_LATER_REVERT: &str = r#"
import "forge-std/Script.sol";

contract MultiResumeLaterRevert is Script {
    function run(string memory rpc1, string memory rpc2) external {
        vm.createSelectFork(rpc1);
        vm.startBroadcast();
        (bool success,) = address(0x000000000000000000000000000000000000bEEF).call("");
        require(success);
        vm.stopBroadcast();

        vm.createSelectFork(rpc2);
        vm.startBroadcast();
        payable(address(0x000000000000000000000000000000000000dEaD)).transfer(1);
        vm.stopBroadcast();
    }
}
"#;

/// Where the revert of the first sequence's only operation is recorded.
enum RevertRecordedBy {
    Broadcast,
    Resume,
}

// The script stops at a failed call, so resume does not send a later sequence that was never
// submitted, even when the reverted operation was the last in its own sequence.
async fn multi_resume_refuses_unsent_sequence_after_revert(
    prj: TestProject,
    mut cmd: TestCommand,
    recorded_by: RevertRecordedBy,
    same_chain: bool,
) {
    let gate = address!("0x000000000000000000000000000000000000bEEF");
    let recipient = address!("0x000000000000000000000000000000000000dEaD");
    let script = prj.add_script("MultiResumeLaterRevert.s.sol", MULTI_RESUME_LATER_REVERT);
    let (api1, handle1) = spawn(NodeConfig::test().with_chain_id(Some(1111u64))).await;
    let node2 = if same_chain {
        None
    } else {
        Some(spawn(NodeConfig::test().with_chain_id(Some(2222u64))).await)
    };
    let (api2, handle2) = node2.as_ref().map_or((&api1, &handle1), |(api, handle)| (api, handle));
    // Runtime that reverts once storage slot 0 is set.
    api1.anvil_set_code(gate, bytes!("60005415600b57600080fd5b00")).await.unwrap();
    api1.anvil_set_auto_mine(false).await.unwrap();
    // Distinct proxy URLs keep the two sequences apart even on the same chain.
    let (rpc1, submissions1) =
        spawn_rpc_proxy_recording_method(handle1.http_endpoint(), "eth_sendRawTransaction").await;
    let (rpc2, submissions2) =
        spawn_rpc_proxy_recording_method(handle2.http_endpoint(), "eth_sendRawTransaction").await;
    let sender = handle1.dev_accounts().next().unwrap();
    let label = if same_chain { "chain 1111 (sequence #1)" } else { "chain 1111" };
    let args = [
        "script",
        script.to_str().unwrap(),
        "--sig",
        "run(string,string)",
        &rpc1,
        &rpc2,
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--slow",
    ];
    let path = prj.root().join("broadcast/multi/MultiResumeLaterRevert.s.sol-latest/run.json");

    // The gate call passes simulation, then reverts on chain before the transfer is sent.
    cmd.forge_fuse().args(args).arg("--broadcast");
    let mut child = KillOnDrop::spawn(cmd.cmd());
    let reverted = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Ok(sequence) = foundry_common::fs::read_json_file::<Value>(&path)
                && let Some(hash) = sequence["deployments"][0]["pending"].get(0)
            {
                break hash.as_str().unwrap().to_string();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the gate call was not checkpointed");
    let mine_revert = async || {
        api1.anvil_set_storage_at(gate, U256::ZERO, B256::with_last_byte(1)).await.unwrap();
        api1.mine_one().await.unwrap();
        api1.anvil_set_auto_mine(true).await.unwrap();
    };
    let transaction_failure = format!("Error: {label}: Transaction Failure: {reverted}\n");
    match recorded_by {
        RevertRecordedBy::Broadcast => {
            mine_revert().await;
            tokio::time::timeout(Duration::from_secs(60), async {
                while child.is_running() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("forge did not finish");
            let output = child.kill_and_wait();
            assert!(!output.status.success());
            assert_eq!(
                String::from_utf8(output.stderr).unwrap(),
                format!(
                    "Warning: Multi chain deployment is still under development. Use with caution.\n{transaction_failure}"
                )
            );
        }
        RevertRecordedBy::Resume => {
            assert!(child.is_running(), "Forge exited before it could be interrupted");
            assert!(!child.kill_and_wait().status.success());
            mine_revert().await;
            cmd.forge_fuse().args(args).args(["--multi", "--resume"]);
            cmd.assert_failure().stderr_eq(transaction_failure);
        }
    }

    // Every resume refuses without sending the transfer that `require(success)` blocks.
    for _ in 0..2 {
        let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
        let deployments = sequence["deployments"].as_array().unwrap();
        assert_eq!(deployments[0]["reverted"], serde_json::json!([reverted]));
        assert!(deployments[1]["transactions"][0]["hash"].is_null());
        assert_eq!(submissions1.lock().unwrap().len(), 1);
        assert!(submissions2.lock().unwrap().is_empty());
        assert_eq!(
            api2.transaction_count(sender, None).await.unwrap().to::<u32>(),
            u32::from(same_chain)
        );
        assert_eq!(handle2.http_provider().get_balance(recipient).await.unwrap(), U256::ZERO);

        cmd.forge_fuse().args(args).args(["--multi", "--resume"]);
        cmd.assert_failure().stderr_eq(format!(
            "Error: {label}: transaction {reverted} reverted, so the script would not have sent the operations after it; resume will not send them. Pass `--broadcast --start-fresh` to archive this broadcast and run the script again\n"
        ));
    }
    assert!(submissions2.lock().unwrap().is_empty());
    assert_eq!(handle2.http_provider().get_balance(recipient).await.unwrap(), U256::ZERO);
}

#[forgetest_init]
async fn multi_resume_refuses_unsent_chain_after_broadcast_revert(prj: _, cmd: _) {
    multi_resume_refuses_unsent_sequence_after_revert(prj, cmd, RevertRecordedBy::Broadcast, false)
        .await;
}

#[forgetest_init]
async fn multi_resume_refuses_unsent_chain_after_resume_revert(prj: _, cmd: _) {
    multi_resume_refuses_unsent_sequence_after_revert(prj, cmd, RevertRecordedBy::Resume, false)
        .await;
}

#[forgetest_init]
async fn multi_resume_refuses_unsent_same_chain_sequence_after_revert(prj: _, cmd: _) {
    multi_resume_refuses_unsent_sequence_after_revert(prj, cmd, RevertRecordedBy::Resume, true)
        .await;
}

// A later sequence's operation that was submitted before an upstream revert was recorded is not new
// work: resume replays its saved signed bytes after the revert, and refuses only the sequences that
// were never submitted, after the replay.
async fn multi_resume_replays_later_signed_attempt_after_revert(
    prj: TestProject,
    mut cmd: TestCommand,
    unsent_sequence: bool,
) {
    let gate = address!("0x000000000000000000000000000000000000bEEF");
    let recipient = address!("0x000000000000000000000000000000000000dEaD");
    let script = prj.add_script(
        "MultiResumeAttemptRevert.s.sol",
        r#"
import "forge-std/Script.sol";

contract MultiResumeAttemptRevert is Script {
    function run(string memory rpc1, string memory rpc2, string memory rpc3) external {
        vm.createSelectFork(rpc1);
        vm.startBroadcast();
        (bool success,) = address(0x000000000000000000000000000000000000bEEF).call("");
        require(success);
        vm.stopBroadcast();

        vm.createSelectFork(rpc2);
        vm.startBroadcast();
        payable(address(0x000000000000000000000000000000000000dEaD)).transfer(1);
        vm.stopBroadcast();

        if (bytes(rpc3).length > 0) {
            vm.createSelectFork(rpc3);
            vm.startBroadcast();
            payable(address(0x000000000000000000000000000000000000dEaD)).transfer(1);
            vm.stopBroadcast();
        }
    }
}
"#,
    );
    let (api1, handle1) = spawn(NodeConfig::test().with_chain_id(Some(1111u64))).await;
    let (api2, handle2) = spawn(NodeConfig::test().with_chain_id(Some(2222u64))).await;
    let (api3, handle3) = spawn(NodeConfig::test().with_chain_id(Some(3333u64))).await;
    // Runtime that reverts once storage slot 0 is set.
    api1.anvil_set_code(gate, bytes!("60005415600b57600080fd5b00")).await.unwrap();
    api1.anvil_set_auto_mine(false).await.unwrap();
    api2.anvil_set_auto_mine(false).await.unwrap();
    // While hidden, chain 1111's transaction looks dropped, so the broadcast moves on to 2222.
    let hide = Arc::new(AtomicBool::new(true));
    let rpc1 =
        spawn_rpc_proxy_mapping_method(handle1.http_endpoint(), "eth_getTransactionByHash", {
            let hide = hide.clone();
            move |_, result| if hide.load(Ordering::SeqCst) { Value::Null } else { result }
        })
        .await;
    let (rpc2, submissions2) =
        spawn_rpc_proxy_recording_method(handle2.http_endpoint(), "eth_sendRawTransaction").await;
    let (rpc3, submissions3) =
        spawn_rpc_proxy_recording_method(handle3.http_endpoint(), "eth_sendRawTransaction").await;
    let sender = handle2.dev_accounts().next().unwrap();
    let args = [
        "script",
        script.to_str().unwrap(),
        "--sig",
        "run(string,string,string)",
        &rpc1,
        &rpc2,
        if unsent_sequence { &rpc3 } else { "" },
        "--private-key",
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        "--timeout",
        "1",
    ];
    let path = prj.root().join("broadcast/multi/MultiResumeAttemptRevert.s.sol-latest/run.json");

    // Interrupt the broadcast once chain 2222's transfer is pending, before chain 3333 is reached.
    cmd.forge_fuse().args(args).arg("--broadcast");
    let child = KillOnDrop::spawn(cmd.cmd());
    let (attempt, reverted) = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Ok(sequence) = foundry_common::fs::read_json_file::<Value>(&path)
                && let Some(hash) = sequence["deployments"][1]["pending"].get(0)
            {
                break (
                    hash.as_str().unwrap().parse::<B256>().unwrap(),
                    sequence["deployments"][0]["transactions"][0]["hash"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("chain 2222's transfer was not checkpointed");
    child.kill_and_wait();
    let payload = submissions2.lock().unwrap()[0].clone();

    // Chain 1111's call reverts, while chain 2222's transfer leaves the mempool unmined, so its
    // outcome stays unresolved after resume records the revert.
    hide.store(false, Ordering::SeqCst);
    api1.anvil_set_storage_at(gate, U256::ZERO, B256::with_last_byte(1)).await.unwrap();
    api1.mine_one().await.unwrap();
    assert_eq!(api2.anvil_drop_transaction(attempt).await.unwrap(), Some(attempt));
    // An empty block lowers the base fee, so signing the transfer again would change its bytes.
    api2.mine_one().await.unwrap();
    api2.anvil_set_auto_mine(true).await.unwrap();
    cmd.forge_fuse().args(args).args(["--multi", "--resume"]);
    cmd.assert_failure().stderr_eq(format!(
        "Warning: Some transactions were discarded by the RPC node. Use `--resume` to retry these transactions.\nError: chain 1111: Transaction Failure: {reverted}\n"
    ));
    let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
    let deployments = sequence["deployments"].as_array().unwrap();
    assert_eq!(deployments.len(), 2 + usize::from(unsent_sequence));
    assert_eq!(deployments[0]["reverted"], serde_json::json!([reverted]));
    assert!(deployments[1]["pending"].as_array().unwrap().is_empty());
    assert!(deployments[1]["receipts"].as_array().unwrap().is_empty());
    assert_eq!(api2.transaction_count(sender, None).await.unwrap().to::<u32>(), 0);

    // The existing attempt is replayed with identical bytes and mined. Chain 3333's transfer was
    // never submitted, so resume refuses it once the replay is done, and again on every later run.
    let refusal = format!(
        "Error: chain 1111: transaction {reverted} reverted, so the script would not have sent the operations after it; resume will not send them. Pass `--broadcast --start-fresh` to archive this broadcast and run the script again\n"
    );
    let warning = format!(
        "Warning: Transaction {reverted} on chain 1111 reverted; resume will not resubmit it.\n"
    );
    cmd.forge_fuse().args(args).args(["--multi", "--resume"]);
    if unsent_sequence {
        cmd.assert_failure().stderr_eq(format!("{warning}{refusal}"));
        cmd.forge_fuse().args(args).args(["--multi", "--resume"]);
        cmd.assert_failure().stderr_eq(refusal);
    } else {
        cmd.assert_success().stderr_eq(format!(
            "{warning}Warning: ONCHAIN EXECUTION COMPLETE WITH REVERTED TRANSACTIONS:\nchain 1111: {reverted}\n"
        ));
    }
    assert_eq!(*submissions2.lock().unwrap(), [payload.clone(), payload]);
    assert_eq!(api2.transaction_count(sender, None).await.unwrap().to::<u32>(), 1);
    assert_eq!(handle2.http_provider().get_balance(recipient).await.unwrap(), U256::from(1));
    assert!(submissions3.lock().unwrap().is_empty());
    assert_eq!(api3.transaction_count(sender, None).await.unwrap().to::<u32>(), 0);
    assert_eq!(handle3.http_provider().get_balance(recipient).await.unwrap(), U256::ZERO);
    let sequence: Value = foundry_common::fs::read_json_file(&path).unwrap();
    let deployments = sequence["deployments"].as_array().unwrap();
    assert_eq!(deployments[0]["reverted"], serde_json::json!([reverted]));
    let receipts = deployments[1]["receipts"].as_array().unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0]["transactionHash"].as_str().unwrap().parse::<B256>().unwrap(), attempt);
    assert_eq!(receipts[0]["status"], "0x1");
    if unsent_sequence {
        assert!(deployments[2]["transactions"][0]["hash"].is_null());
    }
}

#[forgetest_init]
async fn multi_resume_replays_later_signed_attempt_after_revert_without_unsent_work(
    prj: _,
    cmd: _,
) {
    multi_resume_replays_later_signed_attempt_after_revert(prj, cmd, false).await;
}

#[forgetest_init]
async fn multi_resume_replays_later_signed_attempt_before_refusing_unsent_work(prj: _, cmd: _) {
    multi_resume_replays_later_signed_attempt_after_revert(prj, cmd, true).await;
}
