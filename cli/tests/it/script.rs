//! Contains various tests related to forge script
use anvil::{spawn, NodeConfig};
use cast::SimpleCast;
use ethers::abi::Address;
use foundry_cli_test_utils::{
    forgetest, forgetest_async, forgetest_init,
    util::{OutputExt, TestCommand, TestProject},
    ScriptOutcome, ScriptTester,
};
use foundry_utils::rpc;
use regex::Regex;
use serde_json::Value;
use std::{env, path::PathBuf, str::FromStr};

// Tests that fork cheat codes can be used in script
forgetest_init!(
    #[ignore]
    can_use_fork_cheat_codes_in_script,
    |prj: TestProject, mut cmd: TestCommand| {
        let script = prj
            .inner()
            .add_source(
                "Foo",
                r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.10;

import "forge-std/Script.sol";

contract ContractScript is Script {
    function setUp() public {}

    function run() public {
        uint256 fork = vm.activeFork();
        vm.rollFork(11469702);
    }
}
   "#,
            )
            .unwrap();

        let rpc = foundry_utils::rpc::next_http_rpc_endpoint();

        cmd.arg("script").arg(script).args(["--fork-url", rpc.as_str(), "-vvvv"]);
    }
);

// Tests that the `run` command works correctly
forgetest!(can_execute_script_command2, |prj: TestProject, mut cmd: TestCommand| {
    let script = prj
        .inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
contract Demo {
    event log_string(string);
    function run() external {
        emit log_string("script ran");
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("script").arg(script);
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/can_execute_script_command.stdout"),
    );
});

// Tests that the `run` command works correctly when path *and* script name is specified
forgetest!(can_execute_script_command_fqn, |prj: TestProject, mut cmd: TestCommand| {
    let script = prj
        .inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
contract Demo {
    event log_string(string);
    function run() external {
        emit log_string("script ran");
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("script").arg(format!("{}:Demo", script.display()));
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/can_execute_script_command_fqn.stdout"),
    );
});

// Tests that the run command can run arbitrary functions
forgetest!(can_execute_script_command_with_sig, |prj: TestProject, mut cmd: TestCommand| {
    let script = prj
        .inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
contract Demo {
    event log_string(string);
    function myFunction() external {
        emit log_string("script ran");
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("script").arg(script).arg("--sig").arg("myFunction()");
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/can_execute_script_command_with_sig.stdout"),
    );
});

// Tests that the run command can run functions with arguments
forgetest!(can_execute_script_command_with_args, |prj: TestProject, mut cmd: TestCommand| {
    let script = prj
        .inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
contract Demo {
    event log_string(string);
    event log_uint(uint);
    function run(uint256 a, uint256 b) external {
        emit log_string("script ran");
        emit log_uint(a);
        emit log_uint(b);
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("script").arg(script).arg("--sig").arg("run(uint256,uint256)").arg("1").arg("2");
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/can_execute_script_command_with_args.stdout"),
    );
});

// Tests that the run command can run functions with return values
forgetest!(can_execute_script_command_with_returned, |prj: TestProject, mut cmd: TestCommand| {
    let script = prj
        .inner()
        .add_source(
            "Foo",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
contract Demo {
    event log_string(string);
    function run() external returns (uint256 result, uint8) {
        emit log_string("script ran");
        return (255, 3);
    }
}"#,
        )
        .unwrap();
    cmd.arg("script").arg(script);
    cmd.unchecked_output().stdout_matches_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/can_execute_script_command_with_returned.stdout"),
    );
});

forgetest_async!(
    can_broadcast_script_skipping_simulation,
    |prj: TestProject, mut cmd: TestCommand| async move {
        foundry_cli_test_utils::util::initialize(prj.root());
        // This example script would fail in on-chain simulation
        let deploy_script = prj
            .inner()
            .add_source(
                "DeployScript",
                r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
import "forge-std/Script.sol";

contract HashChecker {
    bytes32 public lastHash;
    function update() public {
        bytes32 newHash = blockhash(block.number - 1);
        require(newHash != lastHash, "Hash didn't change");
        lastHash = newHash;
    }

    function checkLastHash() public {
        require(lastHash != bytes32(0),  "Hash shouldn't be zero");
    }
}
contract DeployScript is Script {
    function run() external returns (uint256 result, uint8) {
        vm.startBroadcast();
        HashChecker hashChecker = new HashChecker();
    }
}"#,
            )
            .unwrap();

        let deploy_contract = deploy_script.display().to_string() + ":DeployScript";

        let node_config = NodeConfig::test()
            .with_eth_rpc_url(Some(rpc::next_http_archive_rpc_endpoint()))
            .silent();
        let (_api, handle) = spawn(node_config).await;
        let private_key =
            "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".to_string();
        cmd.set_current_dir(prj.root());

        cmd.args([
            "script",
            &deploy_contract,
            "--root",
            prj.root().to_str().unwrap(),
            "--fork-url",
            &handle.http_endpoint(),
            "-vvvvv",
            "--broadcast",
            "--slow",
            "--skip-simulation",
            "--private-key",
            &private_key,
        ]);

        let output = cmd.stdout_lossy();

        assert!(output.contains("SKIPPING ON CHAIN SIMULATION"));
        assert!(output.contains("ONCHAIN EXECUTION COMPLETE & SUCCESSFUL"));

        let run_log =
            std::fs::read_to_string("broadcast/DeployScript.sol/1/run-latest.json").unwrap();
        let run_object: Value = serde_json::from_str(&run_log).unwrap();
        let contract_address = SimpleCast::to_checksum_address(
            &ethers::prelude::H160::from_str(
                run_object["receipts"][0]["contractAddress"].as_str().unwrap(),
            )
            .unwrap(),
        );

        let run_code = r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.10;
import "forge-std/Script.sol";
import { HashChecker } from "./DeployScript.sol";

contract RunScript is Script {
    function run() external returns (uint256 result, uint8) {
        vm.startBroadcast();
        HashChecker hashChecker = HashChecker(CONTRACT_ADDRESS);
        uint numUpdates = 8;
        vm.roll(block.number - numUpdates);
        for(uint i = 0; i < numUpdates; i++) {
            vm.roll(block.number + 1);
            hashChecker.update();
            hashChecker.checkLastHash();
        }
    }
}"#
        .replace("CONTRACT_ADDRESS", &contract_address);

        let run_script = prj.inner().add_source("RunScript", run_code).unwrap();
        let run_contract = run_script.display().to_string() + ":RunScript";

        cmd.forge_fuse();
        cmd.set_current_dir(prj.root());
        cmd.args([
            "script",
            &run_contract,
            "--root",
            prj.root().to_str().unwrap(),
            "--fork-url",
            &handle.http_endpoint(),
            "-vvvvv",
            "--broadcast",
            "--slow",
            "--skip-simulation",
            "--gas-estimate-multiplier",
            "200",
            "--private-key",
            &private_key,
        ]);

        let output = cmd.stdout_lossy();
        assert!(output.contains("SKIPPING ON CHAIN SIMULATION"));
        assert!(output.contains("ONCHAIN EXECUTION COMPLETE & SUCCESSFUL"));
    }
);

forgetest_async!(can_deploy_script_without_lib, |prj: TestProject, cmd: TestCommand| async move {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(vec![0, 1])
        .await
        .add_sig("BroadcastTestNoLinking", "deployDoesntPanic()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(vec![(0, 1), (1, 2)])
        .await;
});

forgetest_async!(can_deploy_script_with_lib, |prj: TestProject, cmd: TestCommand| async move {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(vec![0, 1])
        .await
        .add_sig("BroadcastTest", "deploy()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(vec![(0, 2), (1, 1)])
        .await;
});

forgetest_async!(
    #[serial_test::serial]
    can_deploy_script_private_key,
    |prj: TestProject, cmd: TestCommand| async move {
        let (_api, handle) = spawn(NodeConfig::test()).await;
        let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

        tester
            .load_addresses(vec![
                Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap()
            ])
            .await
            .add_sig("BroadcastTest", "deployPrivateKey()")
            .simulate(ScriptOutcome::OkSimulation)
            .broadcast(ScriptOutcome::OkBroadcast)
            .assert_nonce_increment_addresses(vec![(
                Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap(),
                3,
            )])
            .await;
    }
);

forgetest_async!(
    #[serial_test::serial]
    can_deploy_script_remember_key,
    |prj: TestProject, cmd: TestCommand| async move {
        let (_api, handle) = spawn(NodeConfig::test()).await;
        let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

        tester
            .load_addresses(vec![
                Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap()
            ])
            .await
            .add_sig("BroadcastTest", "deployRememberKey()")
            .simulate(ScriptOutcome::OkSimulation)
            .broadcast(ScriptOutcome::OkBroadcast)
            .assert_nonce_increment_addresses(vec![(
                Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap(),
                2,
            )])
            .await;
    }
);

forgetest_async!(
    #[serial_test::serial]
    can_deploy_script_remember_key_and_resume,
    |prj: TestProject, cmd: TestCommand| async move {
        let (_api, handle) = spawn(NodeConfig::test()).await;
        let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

        tester
            .add_deployer(0)
            .load_addresses(vec![
                Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap()
            ])
            .await
            .add_sig("BroadcastTest", "deployRememberKeyResume()")
            .simulate(ScriptOutcome::OkSimulation)
            .resume(ScriptOutcome::MissingWallet)
            // load missing wallet
            .load_private_keys(vec![0])
            .await
            .run(ScriptOutcome::OkBroadcast)
            .assert_nonce_increment_addresses(vec![(
                Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap(),
                2,
            )])
            .await
            .assert_nonce_increment(vec![(0, 1)])
            .await;
    }
);

forgetest_async!(can_resume_script, |prj: TestProject, cmd: TestCommand| async move {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(vec![0])
        .await
        .add_sig("BroadcastTest", "deploy()")
        .simulate(ScriptOutcome::OkSimulation)
        .resume(ScriptOutcome::MissingWallet)
        // load missing wallet
        .load_private_keys(vec![1])
        .await
        .run(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(vec![(0, 2), (1, 1)])
        .await;
});

forgetest_async!(can_deploy_broadcast_wrap, |prj: TestProject, cmd: TestCommand| async move {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .add_deployer(2)
        .load_private_keys(vec![0, 1, 2])
        .await
        .add_sig("BroadcastTest", "deployOther()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(vec![(0, 4), (1, 4), (2, 1)])
        .await;
});

forgetest_async!(panic_no_deployer_set, |prj: TestProject, cmd: TestCommand| async move {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(vec![0, 1])
        .await
        .add_sig("BroadcastTest", "deployOther()")
        .simulate(ScriptOutcome::WarnSpecifyDeployer)
        .broadcast(ScriptOutcome::MissingSender);
});

forgetest_async!(can_deploy_no_arg_broadcast, |prj: TestProject, cmd: TestCommand| async move {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .add_deployer(0)
        .load_private_keys(vec![0])
        .await
        .add_sig("BroadcastTest", "deployNoArgs()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(vec![(0, 3)])
        .await;
});

forgetest_async!(can_deploy_with_create2, |prj: TestProject, cmd: TestCommand| async move {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    // Prepare CREATE2 Deployer
    let addr = Address::from_str("0x4e59b44847b379578588920ca78fbf26c0b4956c").unwrap();
    let code = hex::decode("7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3").expect("Could not decode create2 deployer init_code").into();
    api.anvil_set_code(addr, code).await.unwrap();

    tester
        .add_deployer(0)
        .load_private_keys(vec![0])
        .await
        .add_sig("BroadcastTestNoLinking", "deployCreate2()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(vec![(0, 2)])
        .await
        // Running again results in error, since we're repeating the salt passed to CREATE2
        .run(ScriptOutcome::FailedScript);
});

forgetest_async!(
    #[serial_test::serial]
    can_deploy_and_simulate_50_txes_concurrently,
    |prj: TestProject, cmd: TestCommand| async move {
        let (_api, handle) = spawn(NodeConfig::test()).await;
        let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

        tester
            .load_private_keys(vec![0])
            .await
            .add_sig("BroadcastTestNoLinking", "deployMany()")
            .simulate(ScriptOutcome::OkSimulation)
            .broadcast(ScriptOutcome::OkBroadcast)
            .assert_nonce_increment(vec![(0, 50)])
            .await;
    }
);

forgetest_async!(
    #[serial_test::serial]
    can_deploy_and_simulate_mixed_broadcast_modes,
    |prj: TestProject, cmd: TestCommand| async move {
        let (_api, handle) = spawn(NodeConfig::test()).await;
        let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

        tester
            .load_private_keys(vec![0])
            .await
            .add_sig("BroadcastMix", "deployMix()")
            .simulate(ScriptOutcome::OkSimulation)
            .broadcast(ScriptOutcome::OkBroadcast)
            .assert_nonce_increment(vec![(0, 15)])
            .await;
    }
);

forgetest_async!(deploy_with_setup, |prj: TestProject, cmd: TestCommand| async move {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(vec![0])
        .await
        .add_sig("BroadcastTestSetup", "run()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(vec![(0, 6)])
        .await;
});

forgetest_async!(fail_broadcast_staticcall, |prj: TestProject, cmd: TestCommand| async move {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(vec![0])
        .await
        .add_sig("BroadcastTestNoLinking", "errorStaticCall()")
        .simulate(ScriptOutcome::StaticCallNotAllowed);
});

forgetest_async!(
    #[ignore]
    check_broadcast_log,
    |prj: TestProject, cmd: TestCommand| async move {
        let (api, handle) = spawn(NodeConfig::test()).await;
        let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

        // Prepare CREATE2 Deployer
        let addr = Address::from_str("0x4e59b44847b379578588920ca78fbf26c0b4956c").unwrap();
        let code = hex::decode("7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3").expect("Could not decode create2 deployer init_code").into();
        api.anvil_set_code(addr, code).await.unwrap();

        tester
            .load_private_keys(vec![0])
            .await
            .add_sig("BroadcastTestLog", "run()")
            .slow()
            .simulate(ScriptOutcome::OkSimulation)
            .broadcast(ScriptOutcome::OkBroadcast)
            .assert_nonce_increment(vec![(0, 7)])
            .await;

        // Uncomment to recreate log
        // std::fs::copy("broadcast/Broadcast.t.sol/31337/run-latest.json",
        // PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testdata/fixtures/broadcast.log.json"
        // ));

        // Ignore blockhash and timestamp since they can change inbetween runs.
        let re = Regex::new(r#"(blockHash.*?blockNumber)|((timestamp":)[0-9]*)"#).unwrap();

        let fixtures_log = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../testdata/fixtures/broadcast.log.json"),
        )
        .unwrap();
        let fixtures_log = re.replace_all(&fixtures_log, "");

        let run_log =
            std::fs::read_to_string("broadcast/Broadcast.t.sol/31337/run-latest.json").unwrap();
        let run_log = re.replace_all(&run_log, "");

        assert!(fixtures_log == run_log);
    }
);

forgetest_async!(test_default_sender_balance, |prj: TestProject, cmd: TestCommand| async move {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    // Expect the default sender to have uint256.max balance.
    tester
        .add_sig("TestInitialBalance", "runDefaultSender()")
        .simulate(ScriptOutcome::OkSimulation);
});

forgetest_async!(test_custom_sender_balance, |prj: TestProject, cmd: TestCommand| async move {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    // Expect the sender to have its starting balance.
    tester
        .add_deployer(0)
        .add_sig("TestInitialBalance", "runCustomSender()")
        .simulate(ScriptOutcome::OkSimulation);
});

#[derive(serde::Deserialize)]
struct Transactions {
    transactions: Vec<Transaction>,
}

#[derive(serde::Deserialize)]
struct Transaction {
    arguments: Vec<String>,
}

// test we output arguments <https://github.com/foundry-rs/foundry/issues/3053>
forgetest_async!(
    can_execute_script_with_arguments,
    |prj: TestProject, mut cmd: TestCommand| async move {
        cmd.args(["init", "--force"]).arg(prj.root());
        cmd.assert_non_empty_stdout();
        cmd.forge_fuse();

        let (_api, handle) = spawn(NodeConfig::test()).await;
        let script = prj
            .inner()
            .add_script(
                "Counter.s.sol",
                r#"
pragma solidity ^0.8.15;

import "forge-std/Script.sol";

struct Point {
    uint256 x;
    uint256 y;
}

contract A {
    address a;
    uint b;
    int c;
    bytes32 d;
    bool e;

  constructor(address _a, uint _b, int _c, bytes32 _d, bool _e, bytes memory _f, Point memory _g, string memory _h) {
    a = _a;
    b = _b;
    c = _c;
    d = _d;
    e = _e;
  }
}

contract Script0 is Script {
  function run() external {
    vm.broadcast();

    new A(msg.sender, 2 ** 32, -1 * (2 ** 32), keccak256(abi.encode(1)), true, "abcdef", Point(10, 99), "hello");
  }
}
   "#,
            )
            .unwrap();

        cmd.arg("script").arg(script).args([
            "--tc",
            "Script0",
            "--rpc-url",
            handle.http_endpoint().as_str(),
        ]);

        assert!(cmd.stdout_lossy().contains("SIMULATION COMPLETE"));

        let run_latest = foundry_common::fs::json_files(prj.root().join("broadcast"))
            .into_iter()
            .filter(|file| file.ends_with("run-latest.json"))
            .next()
            .expect("No broadcast artifacts");

        let content = foundry_common::fs::read_to_string(run_latest).unwrap();

        let transactions: Transactions = serde_json::from_str(&content).unwrap();
        let transactions = transactions.transactions;
        assert_eq!(transactions.len(), 1);
        assert_eq!(
            transactions[0].arguments,
            vec![
                "0x00a329c0648769A73afAc7F9381E08FB43dBEA72".to_string(),
                "4294967296".to_string(),
                "-4294967296".to_string(),
                "0xb10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6".to_string(),
                "true".to_string(),
                "0x616263646566".to_string(),
                "(10, 99)".to_string(),
                "hello".to_string(),
            ]
        );
    }
);

// test we output arguments <https://github.com/foundry-rs/foundry/issues/3053>
forgetest_async!(
    can_execute_script_with_arguments_nested_deploy,
    |prj: TestProject, mut cmd: TestCommand| async move {
        cmd.args(["init", "--force"]).arg(prj.root());
        cmd.assert_non_empty_stdout();
        cmd.forge_fuse();

        let (_api, handle) = spawn(NodeConfig::test()).await;
        let script = prj
            .inner()
            .add_script(
                "Counter.s.sol",
                r#"
pragma solidity ^0.8.13;

import "forge-std/Script.sol";

contract A {
  address a;
  uint b;
  int c;
  bytes32 d;
  bool e;
  bytes f;
  string g;

  constructor(address _a, uint _b, int _c, bytes32 _d, bool _e, bytes memory _f, string memory _g) {
    a = _a;
    b = _b;
    c = _c;
    d = _d;
    e = _e;
    f = _f;
    g = _g;
  }
}

contract B {
  constructor(address _a, uint _b, int _c, bytes32 _d, bool _e, bytes memory _f, string memory _g) {
    new A(_a, _b, _c, _d, _e, _f, _g);
  }
}

contract Script0 is Script {
  function run() external {
    vm.broadcast();
    new B(msg.sender, 2 ** 32, -1 * (2 ** 32), keccak256(abi.encode(1)), true, "abcdef", "hello");
  }
}
   "#,
            )
            .unwrap();

        cmd.arg("script").arg(script).args([
            "--tc",
            "Script0",
            "--rpc-url",
            handle.http_endpoint().as_str(),
        ]);

        assert!(cmd.stdout_lossy().contains("SIMULATION COMPLETE"));

        let run_latest = foundry_common::fs::json_files(prj.root().join("broadcast"))
            .into_iter()
            .filter(|file| file.ends_with("run-latest.json"))
            .next()
            .expect("No broadcast artifacts");

        let content = foundry_common::fs::read_to_string(run_latest).unwrap();

        let transactions: Transactions = serde_json::from_str(&content).unwrap();
        let transactions = transactions.transactions;
        assert_eq!(transactions.len(), 1);
        assert_eq!(
            transactions[0].arguments,
            vec![
                "0x00a329c0648769A73afAc7F9381E08FB43dBEA72".to_string(),
                "4294967296".to_string(),
                "-4294967296".to_string(),
                "0xb10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6".to_string(),
                "true".to_string(),
                "0x616263646566".to_string(),
                "hello".to_string(),
            ]
        );
    }
);
