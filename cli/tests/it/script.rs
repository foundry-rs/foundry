//! Contains various tests related to forge script
use crate::next_port;
use anvil::{spawn, NodeConfig};
use ethers::abi::Address;
use foundry_cli_test_utils::{
    forgetest, forgetest_async,
    util::{TestCommand, TestProject},
    ScriptOutcome, ScriptTester,
};

use regex::Regex;
use std::{env, path::PathBuf, str::FromStr};
use yansi::Paint;

// Tests that the `run` command works correctly
forgetest!(can_execute_script_command, |prj: TestProject, mut cmd: TestCommand| {
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
    let output = cmd.stdout_lossy();
    assert!(output.ends_with(&format!(
        "Compiler run successful
{}
Gas used: 1751

== Logs ==
  script ran
",
        Paint::green("Script ran successfully.")
    ),));
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
    let output = cmd.stdout_lossy();
    assert!(output.ends_with(&format!(
        "Compiler run successful
{}
Gas used: 1751

== Logs ==
  script ran
",
        Paint::green("Script ran successfully.")
    ),));
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
    let output = cmd.stdout_lossy();
    assert!(output.ends_with(&format!(
        "Compiler run successful
{}
Gas used: 3957

== Logs ==
  script ran
  1
  2
",
        Paint::green("Script ran successfully.")
    ),));
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
    let output = cmd.stdout_lossy();
    assert!(output.ends_with(&format!(
        "Compiler run successful
{}
Gas used: 1836

== Return ==
result: uint256 255
1: uint8 3

== Logs ==
  script ran
",
        Paint::green("Script ran successfully.")
    )));
});

forgetest_async!(can_deploy_script_without_lib, |prj: TestProject, cmd: TestCommand| async move {
    let port = next_port();
    let (_api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
    let mut tester = ScriptTester::new(cmd, port, prj.root());

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
    let port = next_port();
    let (_api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
    let mut tester = ScriptTester::new(cmd, port, prj.root());

    tester
        .load_private_keys(vec![0, 1])
        .await
        .add_sig("BroadcastTest", "deploy()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(vec![(0, 2), (1, 1)])
        .await;
});

forgetest_async!(can_resume_script, |prj: TestProject, cmd: TestCommand| async move {
    let port = next_port();
    let (_api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
    let mut tester = ScriptTester::new(cmd, port, prj.root());

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
    let port = next_port();
    let (_api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
    let mut tester = ScriptTester::new(cmd, port, prj.root());

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
    let port = next_port();
    let (_api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
    let mut tester = ScriptTester::new(cmd, port, prj.root());

    tester
        .load_private_keys(vec![0, 1])
        .await
        .add_sig("BroadcastTest", "deployOther()")
        .simulate(ScriptOutcome::WarnSpecifyDeployer)
        .broadcast(ScriptOutcome::MissingSender);
});

forgetest_async!(can_deploy_no_arg_broadcast, |prj: TestProject, cmd: TestCommand| async move {
    let port = next_port();
    let (_api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
    let mut tester = ScriptTester::new(cmd, port, prj.root());

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
    let port = next_port();
    let (api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
    let mut tester = ScriptTester::new(cmd, port, prj.root());

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
    can_deploy_100_txes_concurrently,
    |prj: TestProject, cmd: TestCommand| async move {
        let port = next_port();
        let (_api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
        let mut tester = ScriptTester::new(cmd, port, prj.root());

        tester
            .load_private_keys(vec![0])
            .await
            .add_sig("BroadcastTestNoLinking", "deployMany()")
            .simulate(ScriptOutcome::OkSimulation)
            .broadcast(ScriptOutcome::OkBroadcast)
            .assert_nonce_increment(vec![(0, 100)])
            .await;
    }
);

forgetest_async!(
    can_deploy_mixed_broadcast_modes,
    |prj: TestProject, cmd: TestCommand| async move {
        let port = next_port();
        let (_api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
        let mut tester = ScriptTester::new(cmd, port, prj.root());

        tester
            .load_private_keys(vec![0])
            .await
            .add_sig("BroadcastTestNoLinking", "deployMix()")
            .simulate(ScriptOutcome::OkSimulation)
            .broadcast(ScriptOutcome::OkBroadcast)
            .assert_nonce_increment(vec![(0, 15)])
            .await;
    }
);

forgetest_async!(deploy_with_setup, |prj: TestProject, cmd: TestCommand| async move {
    let port = next_port();
    let (_api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
    let mut tester = ScriptTester::new(cmd, port, prj.root());

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
    let port = next_port();
    let (_api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
    let mut tester = ScriptTester::new(cmd, port, prj.root());

    tester
        .load_private_keys(vec![0])
        .await
        .add_sig("BroadcastTestNoLinking", "errorStaticCall()")
        .simulate(ScriptOutcome::FailedScript);
});

forgetest_async!(
    #[ignore]
    check_broadcast_log,
    |prj: TestProject, cmd: TestCommand| async move {
        let port = next_port();
        let (api, _handle) = spawn(NodeConfig::test().with_port(port)).await;
        let mut tester = ScriptTester::new(cmd, port, prj.root());

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
