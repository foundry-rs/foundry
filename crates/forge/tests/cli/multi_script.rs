//! Contains various tests related to forge script

use crate::utils::KillOnDrop;
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, B256, U256, address, bytes, hex, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use anvil::{NodeConfig, spawn};
use foundry_test_utils::{
    ScriptOutcome, ScriptTester,
    rpc::{
        spawn_rpc_proxy_blocking_first_submission, spawn_rpc_proxy_mapping_method,
        spawn_rpc_proxy_recording_method, spawn_rpc_proxy_rejecting_method_when_enabled,
    },
    str,
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

const PRIVATE_KEY: &str = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

#[forgetest_init]
async fn uses_endpoint_profile_builds_on_forks(prj: _, cmd: _) {
    let first_lib = Address::repeat_byte(0x11);
    let second_lib = Address::repeat_byte(0x22);
    let (_, first) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(31338u64))
            .with_hardfork(Some("paris".parse().unwrap())),
    )
    .await;
    let (_, second) = spawn(NodeConfig::test().with_chain_id(Some(31339u64))).await;
    prj.add_source(
        "ProfileScript.sol",
        r#"
import "forge-std/Script.sol";
library Lib { function value() external pure returns (uint256) { return 1; } }
contract UsesLib { function value() external pure returns (uint256) { return Lib.value(); } }
library Unconfigured { function value() external pure returns (uint256) { return 2; } }
contract UsesUnconfigured {
    function value() external pure returns (uint256) { return Unconfigured.value(); }
}
contract Plain {
    uint256 public x;
    constructor(uint256 x_) { x = x_; }
    function bump() external { x += 1; }
}
interface IPing { function ping() external view returns (uint256); }
contract Creator {
    function created(bytes memory initCode) public returns (bool ok) {
        assembly { ok := iszero(iszero(create(0, add(initCode, 0x20), mload(initCode)))) }
    }
    function notCreated(bytes memory initCode) external { require(!this.created{gas: 1e6}(initCode)); }
    function callBack(IPing target) external view returns (uint256) { return target.ping(); }
}
bytes constant PUSH0 = hex"5f00";
contract ProfileScript is Script {
    function run() external {
        vm.createSelectFork(vm.rpcUrl("first"));
        vm.startBroadcast();
        Creator creator = new Creator();
        creator.notCreated(PUSH0);
        vm.stopBroadcast();
        vm.assertEq(address(creator).code, vm.getDeployedCode("ProfileScript.sol:Creator"));
        vm.assertTrue(creator.created(hex"00"));
        vm.assertEq(address(new Plain(0)).code, vm.getDeployedCode("ProfileScript.sol:Plain"));
        bytes memory initCode =
            abi.encodePacked(vm.getCode("ProfileScript.sol:Plain"), abi.encode(uint256(7)));
        vm.assertEq(
            address(new Plain{salt: bytes32(uint256(7))}(7)),
            vm.computeCreate2Address(bytes32(uint256(7)), keccak256(initCode), address(this))
        );
        vm.assertEq(creator.callBack(IPing(address(this))), 1);
        bytes memory code = vm.getCode("ProfileScript.sol:Plain");
        string memory path = string.concat(vm.projectRoot(), "/out/ProfileScript.sol/Plain.json");
        vm.assertEq(vm.getArtifactPathByCode(code), path);
        vm.assertEq(vm.getCode(path), code);
        vm.assertEq(
            vm.getArtifactPathByDeployedCode(vm.getDeployedCode("ProfileScript.sol:Plain")), path
        );
        vm.assertEq(vm.getArtifactPathByCode(type(Plain).creationCode), path);
        bytes memory firstCode = vm.getCode("ProfileScript.sol:UsesLib");
        deploy(1);
        vm.createSelectFork("second");
        vm.assertNotEq(vm.getCode("ProfileScript.sol:UsesLib"), firstCode);
        vm.assertEq(
            vm.getCode("out/ProfileScript.sol/UsesLib.json"), vm.getCode("ProfileScript.sol:UsesLib")
        );
        vm.assertEq(vm.getCode("external/External.json"), hex"6001");
        deploy(2);
    }

    function deploy(uint256 x) internal {
        bytes32 salt = bytes32(x);
        bytes memory initCode =
            abi.encodePacked(vm.getCode("ProfileScript.sol:Plain"), abi.encode(x));
        vm.startBroadcast();
        vm.deployCode("ProfileScript.sol:UsesLib");
        Plain created_ = new Plain(x);
        Plain salted = new Plain{salt: salt}(x);
        salted.bump();
        vm.stopBroadcast();
        vm.assertEq(address(created_).code, vm.getDeployedCode("ProfileScript.sol:Plain"));
        vm.assertEq(address(salted), vm.computeCreate2Address(salt, keccak256(initCode)));
    }

    function ping() external pure returns (uint256) {
        return 1;
    }

    function single() external {
        Creator creator = new Creator();
        vm.assertFalse(creator.created{gas: 1e6}(PUSH0));
        vm.assertEq(creator.callBack(IPing(address(this))), 1);
        vm.setEvmVersion("shanghai");
        vm.assertTrue(creator.created{gas: 1e6}(PUSH0));
        vm.broadcast();
        vm.deployCode("ProfileScript.sol:UsesLib");
    }
}
contract ProfileFailures is Script {
    function unlinked() external {
        vm.createSelectFork("first");
        new UsesUnconfigured();
    }

    function stale() external {
        vm.createSelectFork("first");
        vm.getCode("out/Stale.sol/Stale.json");
    }

    function unset() external {
        vm.setEnv("UNSET_PROFILE_RPC_URL", vm.rpcUrl("first"));
        vm.createSelectFork("unset");
    }

    function unbuilt() external {
        vm.setEnv("UNSET_PROFILE_RPC_URL", "http://127.0.0.1:9");
        vm.createSelectFork("unset");
    }

    function unprofiled() external {
        vm.setEnv("PLAIN_RPC_URL", vm.rpcUrl("first"));
        vm.createSelectFork("plain");
    }
}
"#,
    );
    prj.create_file(
        "external/External.json",
        r#"{"abi":[],"bytecode":{"object":"0x6001"},"deployedBytecode":{"object":"0x6001"}}"#,
    );
    // `unset` and `plain` do not resolve, `offline` is unreachable, and `retrying` shares the URL
    // of `first` with other settings.
    prj.create_file(
        "foundry.toml",
        &format!(
            r#"
[profile.default]
evm_version = "shanghai"
fs_permissions = [{{ access = "read", path = "./out" }}, {{ access = "read", path = "./external" }}]
script_execution_protection = false

[profile.first]
evm_version = "paris"
optimizer = true
libraries = ["src/ProfileScript.sol:Lib:{first_lib}"]

[profile.second]
evm_version = "cancun"
libraries = ["src/ProfileScript.sol:Lib:{second_lib}"]

[rpc_endpoints]
first = {{ endpoint = "{}", profile = "first" }}
second = {{ endpoint = "{}", profile = "second" }}
unset = {{ endpoint = "${{UNSET_PROFILE_RPC_URL}}", profile = "second" }}
plain = "${{PLAIN_RPC_URL}}"
offline = {{ endpoint = "http://127.0.0.1:1", profile = "second" }}
retrying = {{ endpoint = "{}", profile = "first", retries = 3 }}
"#,
            first.http_endpoint(),
            second.http_endpoint(),
            first.http_endpoint()
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

    // `--evm-version` applies to the run build only, and `--force` keeps the run's artifacts.
    cmd.args(["script", "ProfileScript", "--broadcast", "--private-key", PRIVATE_KEY])
        .args(["--evm-version", "shanghai", "--force"])
        .assert_success();
    let deployments = &read("multi/ProfileScript.sol-latest/run.json")["deployments"];
    let creator = &deployments[0]["transactions"][1];
    assert_eq!(creator["contractName"], "Creator");
    assert_eq!(creator["function"], "notCreated(bytes)");
    for (handle, deployment, library, offset) in
        [(&first, &deployments[0], first_lib, 2), (&second, &deployments[1], second_lib, 0)]
    {
        // `vm.deployCode`, `new`, `new` with salt and a call to the salted contract.
        let transactions = deployment["transactions"].as_array().unwrap();
        assert_eq!(transactions.len(), offset + 4);
        assert!(linked(&code(handle, deployment, offset).await, library));
        assert_eq!(transactions[offset + 1]["contractName"], "Plain");
        assert_eq!(transactions[offset + 3]["contractName"], "Plain");
        assert_eq!(transactions[offset + 3]["function"], "bump()");
        assert_eq!(
            code(handle, deployment, offset + 1).await,
            code(handle, deployment, offset + 2).await
        );
    }

    // The `--rpc-url` endpoint uses its profile build too.
    cmd.forge_fuse()
        .args(["script", "ProfileScript", "--sig", "single()", "--rpc-url", "first"])
        .args(["--broadcast", "--private-key", PRIVATE_KEY])
        .assert_success();
    let sequence = read("ProfileScript.sol/31338/single-latest.json");
    assert_eq!(sequence["transactions"].as_array().unwrap().len(), 1);
    assert!(linked(&code(&first, &sequence, 0).await, first_lib));

    cmd.forge_fuse()
        .args(["script", "ProfileFailures", "--sig", "unlinked()", "--rpc-url", "first"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: the `--rpc-url` endpoint names a profile, so libraries of the run build can't be deployed on it

"#]]);

    prj.create_file("out/Stale.sol/Stale.json", "{}");
    for (sig, error) in [
        (
            "unlinked()",
            str![[r#"
Error: script failed: no linked artifact for [..]/src/ProfileScript.sol:UsesUnconfigured in the profile build

"#]],
        ),
        (
            "stale()",
            str![[r#"
Error: script failed: vm.getCode: no artifact Stale.sol/Stale.json in the profile build

"#]],
        ),
        (
            "unset()",
            str![[r#"
Error: script failed: vm.createSelectFork: the URL of `unset` does not match a profile build made when forge started

"#]],
        ),
        (
            "unbuilt()",
            str![[r#"
Error: script failed: vm.createSelectFork: the URL of `unset` does not match a profile build made when forge started

"#]],
        ),
        (
            "unprofiled()",
            str![[r#"
Error: script failed: vm.createSelectFork: the URL of `plain` does not match a profile build made when forge started

"#]],
        ),
    ] {
        cmd.forge_fuse()
            .args(["script", "ProfileFailures", "--sig", sig])
            .assert_failure()
            .stderr_eq(error);
    }

    let toml = std::fs::read_to_string(prj.root().join("foundry.toml")).unwrap();
    for (endpoint, error) in [
        (
            format!("shared = \"{}\"", first.http_endpoint()),
            str![[r#"
Error: endpoint `shared` names no profile, but shares its URL with `retrying`, which does

"#]],
        ),
        (
            format!("dup = {{ endpoint = \"{}\", profile = \"second\" }}", first.http_endpoint()),
            str![[r#"
Error: endpoints `retrying` and `dup` share a URL but name different profiles

"#]],
        ),
        (
            "bad = { endpoint = \"http://127.0.0.1:3\", profile = \"missing\" }".to_string(),
            str![[r#"
Error: failed to load profile `missing` of `[rpc_endpoints]`

Context:
- failed to extract foundry config:
foundry config error: unknown profile `missing`

"#]],
        ),
    ] {
        prj.create_file("foundry.toml", &format!("{toml}{endpoint}\n"));
        cmd.forge_fuse().args(["script", "ProfileScript"]).assert_failure().stderr_eq(error);
    }
    prj.create_file("foundry.toml", &toml);

    // No profiled endpoint resolves when forge starts.
    prj.create_file(
        "foundry.toml",
        r#"
[profile.default]
evm_version = "shanghai"
[profile.second]
evm_version = "cancun"
[rpc_endpoints]
unset = { endpoint = "${UNSET_PROFILE_RPC_URL}", profile = "second" }
"#,
    );
    cmd.forge_fuse()
        .args(["script", "ProfileFailures", "--sig", "unbuilt()"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: script failed: vm.createSelectFork: the URL of `unset` does not match a profile build made when forge started

"#]]);
    prj.create_file("foundry.toml", &toml);

    // `forge clean` removes the profile builds.
    cmd.forge_fuse().arg("clean").assert_success();
    assert!(!prj.root().join("cache/profiles").exists());
}

#[forgetest_init]
async fn endpoint_profile_fork_preserves_recorded_initcode_and_metadata(prj: _, cmd: _) {
    let (_, node) = spawn(NodeConfig::test()).await;
    prj.add_source(
        "Recorded.sol",
        r#"
import "forge-std/Script.sol";
contract RecordedChild {
    uint256 public value;
    constructor(uint256 x) { value = x + 7; }
}
contract RecordedInner {}
contract RecordedOuter { RecordedInner public inner = new RecordedInner(); }
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
        new RecordedOuter();
        RecordedFactory factory = new RecordedFactory();
        address nested = factory.deploy(type(RecordedPlain).creationCode);
        bytes memory input = abi.encodePacked(bytes32(block.number), type(RecordedPlain).creationCode);
        (bool ok,) = CREATE2_FACTORY.call(input);
        vm.stopBroadcast();
        require(ok);
        require(direct.value() == 49 && salted.value() == 50);
        require(keccak256(address(direct).code) != keccak256(type(RecordedChild).runtimeCode));
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
        let transactions = sequence["transactions"].as_array().unwrap();
        let names = transactions.iter().map(|tx| tx["contractName"].clone()).collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "RecordedChild",
                "RecordedChild",
                "RecordedOuter",
                "RecordedFactory",
                "RecordedFactory",
                "RecordedPlain"
            ]
        );
        assert_eq!(transactions[0]["arguments"], serde_json::json!(["42"]));
        assert_eq!(transactions[1]["arguments"], serde_json::json!(["43"]));
        assert_eq!(transactions[4]["function"], "deploy(bytes)");
        if !skip_simulation {
            let inner = &transactions[2]["additionalContracts"][0];
            assert_eq!(inner["contractName"], "RecordedInner");
            assert_eq!(inner["exactArtifactMatch"], true);
            let nested = &transactions[4]["additionalContracts"][0];
            assert_eq!(nested["contractName"], "RecordedPlain");
            assert_eq!(nested["exactArtifactMatch"], true);
        }
        for (tx, expected) in transactions.iter().take(2).zip([49, 50]) {
            let address = tx["contractAddress"].as_str().unwrap().parse().unwrap();
            let value = node
                .http_provider()
                .call(
                    TransactionRequest::default()
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
async fn endpoint_profile_fork_replays_transactions_with_their_own_spec(prj: _, cmd: _) {
    let (_, node) = spawn(NodeConfig::test().with_hardfork(Some("paris".parse().unwrap()))).await;
    let provider = node.http_provider();
    let sender = node.dev_accounts().next().unwrap();
    // Creates a contract from its calldata and stores the new address in slot 0.
    let store = TransactionRequest::default()
        .with_from(sender)
        .with_deploy_code(hex!("6f3660006000373660006000f06000550060005260106010f3"));
    let store = provider
        .send_transaction(store.into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap()
        .contract_address
        .unwrap();
    let call = TransactionRequest::default()
        .with_from(sender)
        .with_to(store)
        .with_input(hex!("600160005d00"));
    let hash = provider
        .send_transaction(call.into())
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap()
        .transaction_hash;
    assert_eq!(provider.get_storage_at(store, U256::ZERO).await.unwrap(), U256::ZERO);
    prj.add_script(
        "Replay.s.sol",
        r#"
import "forge-std/Script.sol";
contract Replay is Script {
    function run(bytes32 hash, address store) external {
        vm.createSelectFork("local", hash);
        vm.transact(hash);
        require(vm.load(store, 0) == 0, "replayed with the profile spec");
    }
}
"#,
    );
    prj.create_file(
        "foundry.toml",
        &format!(
            r#"
[profile.default]
evm_version = "shanghai"
[profile.endpoint]
evm_version = "cancun"
[rpc_endpoints]
local = {{ endpoint = "{}", profile = "endpoint" }}
"#,
            node.http_endpoint()
        ),
    );
    cmd.args(["script", "Replay", "--sig", "run(bytes32,address)"])
        .args([hash.to_string(), store.to_string()])
        .assert_success();
}

#[cfg(feature = "monad")]
#[forgetest_init]
async fn monad_endpoint_profile_fork_records_profile_contracts(prj: _, cmd: _) {
    let (_, node) = spawn(NodeConfig::test()).await;
    prj.add_script(
        "Monad.s.sol",
        r#"
import "forge-std/Script.sol";
contract MonadInner {}
contract MonadOuter { MonadInner public inner = new MonadInner(); }
contract MonadScript is Script {
    function run() external {
        vm.broadcast();
        new MonadOuter();
    }
}
"#,
    );
    prj.create_file(
        "foundry.toml",
        &format!(
            r#"
[profile.default]
hardfork = "monad:MonadNine"
[profile.endpoint]
optimizer = true
bytecode_hash = "none"
[rpc_endpoints]
local = {{ endpoint = "{}", profile = "endpoint" }}
"#,
            node.http_endpoint()
        ),
    );
    cmd.args(["script", "MonadScript", "--rpc-url", "local", "--network", "monad"])
        .args(["--private-key", PRIVATE_KEY])
        .assert_success();
    let sequence: Value = foundry_common::fs::read_json_file(
        &prj.root().join("broadcast/Monad.s.sol/31337/dry-run/run-latest.json"),
    )
    .unwrap();
    let outer = &sequence["transactions"][0];
    assert_eq!(outer["contractName"], "MonadOuter");
    assert_eq!(outer["additionalContracts"][0]["contractName"], "MonadInner");
    assert_eq!(outer["additionalContracts"][0]["exactArtifactMatch"], true);
}

#[forgetest_init]
async fn endpoint_profile_fork_does_not_broadcast_failed_creates(prj: _, cmd: _) {
    let (_, node) = spawn(NodeConfig::test()).await;
    prj.add_source("IrOnly.sol", "contract IrOnly {}");
    prj.add_source(
        "Caught.sol",
        r#"
import "forge-std/Script.sol";
contract Plain {}
contract CaughtScript is Script {
    function run() external {
        bytes memory code = vm.getCode("IrOnly.sol:IrOnly");
        vm.createSelectFork("local");
        vm.startBroadcast();
        address created;
        assembly { created := create(0, add(code, 32), mload(code)) }
        require(created == address(0), "created without a profile artifact");
        new Plain();
        vm.stopBroadcast();
    }

    function code() external {
        vm.createSelectFork("local");
        vm.getCode("IrOnly.sol:IrOnly");
    }
}
"#,
    );
    // The run compiles `IrOnly` with the `ir` compiler profile, which the endpoint's profile lacks.
    prj.create_file(
        "foundry.toml",
        &format!(
            r#"
[profile.default]
additional_compiler_profiles = [{{ name = "ir", via_ir = true }}]
compilation_restrictions = [{{ paths = "src/IrOnly.sol", via_ir = true }}]
[profile.endpoint]
additional_compiler_profiles = []
compilation_restrictions = []
[rpc_endpoints]
local = {{ endpoint = "{}", profile = "endpoint" }}
"#,
            node.http_endpoint()
        ),
    );
    cmd.args(["script", "CaughtScript", "--broadcast", "--private-key", PRIVATE_KEY])
        .assert_success();
    let sequence: Value = foundry_common::fs::read_json_file(
        &prj.root().join("broadcast/Caught.sol/31337/run-latest.json"),
    )
    .unwrap();
    let transactions = sequence["transactions"].as_array().unwrap();
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0]["contractName"], "Plain");

    cmd.forge_fuse()
        .args(["script", "CaughtScript", "--sig", "code()"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: script failed: vm.getCode: no artifact for [..]/src/IrOnly.sol:IrOnly in the profile build

"#]]);
}

#[forgetest_init]
async fn endpoint_profile_fork_refuses_run_libraries_it_does_not_link(prj: _, cmd: _) {
    let (_, node) = spawn(NodeConfig::test()).await;
    prj.add_source(
        "Configured.sol",
        r#"
import "forge-std/Script.sol";
library ConfiguredLib { function value() external pure returns (uint256) { return 1; } }
contract UsesConfiguredLib {
    function value() external pure returns (uint256) { return ConfiguredLib.value(); }
}
contract ConfiguredFactory {
    function deploy(bytes memory code) external returns (address result) {
        assembly { result := create(0, add(code, 32), mload(code)) }
    }
}
contract ConfiguredScript is Script {
    function run() external {
        vm.startBroadcast();
        new ConfiguredFactory().deploy(type(UsesConfiguredLib).creationCode);
        vm.stopBroadcast();
    }
}
"#,
    );
    prj.create_file(
        "foundry.toml",
        &format!(
            r#"
[profile.default]
libraries = ["src/Configured.sol:ConfiguredLib:0x3333333333333333333333333333333333333333"]
[profile.endpoint]
libraries = ["src/Configured.sol:ConfiguredLib:0x4444444444444444444444444444444444444444"]
[rpc_endpoints]
local = {{ endpoint = "{}", profile = "endpoint" }}
"#,
            node.http_endpoint()
        ),
    );
    cmd.args(["script", "ConfiguredScript", "--rpc-url", "local", "--private-key", PRIVATE_KEY])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: a broadcast to a profiled endpoint uses a library of the run build that its profile does not link

"#]]);
}

#[forgetest_init]
async fn uses_endpoint_profile_builds_in_test_forks(prj: _, cmd: _) {
    let first_lib = Address::repeat_byte(0x11);
    let second_lib = Address::repeat_byte(0x22);
    let (_, first) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(31338u64))
            .with_hardfork(Some("paris".parse().unwrap())),
    )
    .await;
    let (_, second) = spawn(
        NodeConfig::test()
            .with_chain_id(Some(31339u64))
            .with_hardfork(Some("prague".parse().unwrap())),
    )
    .await;
    prj.add_source(
        "Profiled.sol",
        r#"
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
    function bls() external view returns (bool ok) {
        (ok,) = address(0x0b).staticcall("");
    }
}
contract ForkOnly {
    uint256 immutable seven;
    constructor() { seven = 7; }
    function ping() external view returns (uint256) { return seven; }
}
contract ProfileHandler {
    uint256 public count;
    function hit() external { count++; }
}
bytes constant TSTORE = hex"600160005d00";
"#,
    );
    prj.add_test(
        "ProfileFork.t.sol",
        r#"
import "forge-std/Test.sol";
import "../src/Profiled.sol";
contract ProfileForkTest is Test {
    function test_fork_profile() public {
        bytes memory runCode = address(new UsesLib()).code;
        assertFalse(new Creator().created{gas: 1e6}(TSTORE));
        vm.createSelectFork("second");
        assertTrue(new Creator().created{gas: 1e6}(TSTORE));
        assertTrue(new Creator().bls());
        UsesLib usesLib = new UsesLib();
        bytes memory code = address(usesLib).code;
        assertEq(code, vm.getDeployedCode("Profiled.sol:UsesLib"));
        assertNotEq(code, runCode);
        vm.expectRevert();
        usesLib.value();
        assertEq(new UsesAutoLib(1).value(), 3);
        vm.setEvmVersion("shanghai");
        assertFalse(new Creator().created{gas: 1e6}(TSTORE));
        vm.createSelectFork("first");
        assertEq(new ForkOnly().ping(), 7);
    }

    function test_fork_unset_profile() public {
        vm.setEnv("UNSET_PROFILE_RPC_URL", vm.rpcUrl("first"));
        vm.createSelectFork("unset");
    }
}
contract ProfileInvariantTest is Test {
    ProfileHandler handler;

    function setUp() public {
        vm.createSelectFork("first");
        handler = new ProfileHandler();
        targetContract(address(handler));
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 1
    function invariant_profile_target() public view {
        assertLe(handler.count(), 1);
    }
}
"#,
    );
    prj.create_file(
        "foundry.toml",
        &format!(
            r#"
[profile.default]
evm_version = "shanghai"

[profile.first]
evm_version = "paris"
optimizer = true
libraries = ["src/Profiled.sol:Lib:{first_lib}"]

[profile.second]
evm_version = "cancun"
libraries = ["src/Profiled.sol:Lib:{second_lib}"]

[rpc_endpoints]
first = {{ endpoint = "{}", profile = "first" }}
second = {{ endpoint = "{}", profile = "second" }}
unset = {{ endpoint = "${{UNSET_PROFILE_RPC_URL}}", profile = "second" }}
"#,
            first.http_endpoint(),
            second.http_endpoint()
        ),
    );

    // Traces identify contracts of the profile builds.
    cmd.args(["test", "--mt", "test_fork_profile"]).assert_success();
    cmd.forge_fuse()
        .args(["test", "--mt", "test_fork_profile", "-vvvv"])
        .assert_success()
        .stdout_eq(str![[r#"
...
    ├─ [..] VM::deployCode("src/Profiled.sol:UsesAutoLib", [..])
    │   ├─ [..] → new UsesAutoLib@[..]
...
    ├─ [..] UsesAutoLib::value() [staticcall]
    │   ├─ [..] AutoLib::value() [delegatecall]
...
    ├─ [..] VM::deployCode("src/Profiled.sol:ForkOnly")
    │   ├─ [..] → new ForkOnly@[..]
    │   │   └─ ← [Return] 150 bytes of code
    │   └─ ← [Return] ForkOnly: [[..]]
    ├─ [..] ForkOnly::ping() [staticcall]
    │   └─ ← [Return] 7
...
"#]]);

    // Invariant campaigns identify targets created from profile builds.
    cmd.forge_fuse().args(["test", "--mc", "ProfileInvariantTest"]).assert_success();

    cmd.forge_fuse()
        .args(["test", "--mt", "test_fork_unset_profile"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: vm.createSelectFork: the URL of `unset` does not match a profile build made when forge started] test_fork_unset_profile() ([GAS])
...
"#]]);

    // Coverage credits code that only runs from profile builds, and keeps the path filter.
    prj.add_test("Unmatched.t.sol", "contract Unmatched { function test_unmatched() public {} }");
    cmd.forge_fuse()
        .args(["coverage", "--match-path", "test/ProfileFork.t.sol", "--mt", "test_fork_profile"])
        .args(["--report", "lcov"])
        .assert_success();
    let lcov = std::fs::read_to_string(prj.root().join("lcov.info")).unwrap();
    assert!(lcov.contains("FNDA:1,ForkOnly.constructor"), "{lcov}");
    assert!(lcov.contains("FNDA:1,UsesAutoLib.constructor"), "{lcov}");
    assert!(lcov.contains("FNDA:1,ForkOnly.ping"), "{lcov}");
    assert!(!lcov.contains("Unmatched"), "{lcov}");
    // The coverage build of the `first` profile has the optimizer off.
    let enabled = std::fs::read_dir(prj.root().join("cache/profiles"))
        .unwrap()
        .filter_map(|dir| {
            let artifact =
                dir.unwrap().path().join("coverage/artifacts/Profiled.sol/ForkOnly.json");
            std::fs::read_to_string(artifact).ok()
        })
        .map(|json| {
            serde_json::from_str::<Value>(&json).unwrap()["metadata"]["settings"]["optimizer"]
                ["enabled"]
                .clone()
        })
        .collect::<Vec<_>>();
    assert!(!enabled.is_empty() && enabled.iter().all(|enabled| enabled == false), "{enabled:?}");
}
