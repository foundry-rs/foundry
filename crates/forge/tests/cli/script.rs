//! Contains various tests related to `forge script`.

use crate::constants::TEMPLATE_CONTRACT;
use alloy_primitives::{hex, Address, Bytes};
use anvil::{spawn, NodeConfig};
use foundry_test_utils::{rpc, ScriptOutcome, ScriptTester};
use regex::Regex;
use serde_json::Value;
use std::{env, path::PathBuf, str::FromStr};

// Tests that fork cheat codes can be used in script
forgetest_init!(
    #[ignore]
    can_use_fork_cheat_codes_in_script,
    |prj, cmd| {
        let script = prj
            .add_source(
                "Foo",
                r#"
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

        let rpc = foundry_test_utils::rpc::next_http_rpc_endpoint();

        cmd.arg("script").arg(script).args(["--fork-url", rpc.as_str(), "-vvvvv"]).assert_success();
    }
);

// Tests that the `run` command works correctly
forgetest!(can_execute_script_command2, |prj, cmd| {
    let script = prj
        .add_source(
            "Foo",
            r#"
contract Demo {
    event log_string(string);
    function run() external {
        emit log_string("script ran");
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("script").arg(script).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Script ran successfully.
[GAS]

== Logs ==
  script ran

"#]]);
});

// Tests that the `run` command works correctly when path *and* script name is specified
forgetest!(can_execute_script_command_fqn, |prj, cmd| {
    let script = prj
        .add_source(
            "Foo",
            r#"
contract Demo {
    event log_string(string);
    function run() external {
        emit log_string("script ran");
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("script").arg(format!("{}:Demo", script.display())).assert_success().stdout_eq(str![[
        r#"
...
Script ran successfully.
[GAS]

== Logs ==
  script ran
...
"#
    ]]);
});

// Tests that the run command can run arbitrary functions
forgetest!(can_execute_script_command_with_sig, |prj, cmd| {
    let script = prj
        .add_source(
            "Foo",
            r#"
contract Demo {
    event log_string(string);
    function myFunction() external {
        emit log_string("script ran");
    }
}
   "#,
        )
        .unwrap();

    cmd.arg("script").arg(script).arg("--sig").arg("myFunction()").assert_success().stdout_eq(
        str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Script ran successfully.
[GAS]

== Logs ==
  script ran

"#]],
    );
});

static FAILING_SCRIPT: &str = r#"
import "forge-std/Script.sol";

contract FailingScript is Script {
    function run() external {
        revert("failed");
    }
}
"#;

// Tests that execution throws upon encountering a revert in the script.
forgetest_async!(assert_exit_code_error_on_failure_script, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());
    let script = prj.add_source("FailingScript", FAILING_SCRIPT).unwrap();

    // set up command
    cmd.arg("script").arg(script);

    // run command and assert error exit code
    cmd.assert_failure().stderr_eq(str![[r#"
Error: 
script failed: revert: failed

"#]]);
});

// Tests that execution throws upon encountering a revert in the script with --json option.
// <https://github.com/foundry-rs/foundry/issues/2508>
forgetest_async!(assert_exit_code_error_on_failure_script_with_json, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());
    let script = prj.add_source("FailingScript", FAILING_SCRIPT).unwrap();

    // set up command
    cmd.arg("script").arg(script).arg("--json");

    // run command and assert error exit code
    cmd.assert_failure().stderr_eq(str![[r#"
Error: 
script failed: revert: failed

"#]]);
});

// Tests that the manually specified gas limit is used when using the --unlocked option
forgetest_async!(can_execute_script_command_with_manual_gas_limit_unlocked, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());
    let deploy_script = prj
        .add_source(
            "Foo",
            r#"
import "forge-std/Script.sol";

contract GasWaster {
    function wasteGas(uint256 minGas) public {
        require(gasleft() >= minGas, "Gas left needs to be higher");
    }
}
contract DeployScript is Script {
    function run() external {
        vm.startBroadcast();
        GasWaster gasWaster = new GasWaster();
        gasWaster.wasteGas{gas: 500000}(200000);
    }
}
   "#,
        )
        .unwrap();

    let deploy_contract = deploy_script.display().to_string() + ":DeployScript";

    let node_config =
        NodeConfig::test().with_eth_rpc_url(Some(rpc::next_http_archive_rpc_endpoint())).silent();
    let (_api, handle) = spawn(node_config).await;
    let dev = handle.dev_accounts().next().unwrap();
    cmd.set_current_dir(prj.root());

    cmd.args([
        "script",
        &deploy_contract,
        "--root",
        prj.root().to_str().unwrap(),
        "--fork-url",
        &handle.http_endpoint(),
        "--sender",
        format!("{dev:?}").as_str(),
        "-vvvvv",
        "--slow",
        "--broadcast",
        "--unlocked",
        "--ignored-error-codes=2018", // `wasteGas` can be restricted to view
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Traces:
  [..] DeployScript::run()
    ├─ [0] VM::startBroadcast()
    │   └─ ← [Return] 
    ├─ [..] → new GasWaster@[..]
    │   └─ ← [Return] 226 bytes of code
    ├─ [..] GasWaster::wasteGas(200000 [2e5])
    │   └─ ← [Stop] 
    └─ ← [Stop] 


Script ran successfully.

## Setting up 1 EVM.
==========================
Simulated On-chain Traces:

  [45299] → new GasWaster@[..]
    └─ ← [Return] 226 bytes of code

  [226] GasWaster::wasteGas(200000 [2e5])
    └─ ← [Stop] 


==========================

Chain 1

[ESTIMATED_GAS_PRICE]

[ESTIMATED_TOTAL_GAS_USED]

[ESTIMATED_AMOUNT_REQUIRED]

==========================


==========================

ONCHAIN EXECUTION COMPLETE & SUCCESSFUL.

[SAVED_TRANSACTIONS]

[SAVED_SENSITIVE_VALUES]


"#]]);
});

// Tests that the manually specified gas limit is used.
forgetest_async!(can_execute_script_command_with_manual_gas_limit, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());
    let deploy_script = prj
        .add_source(
            "Foo",
            r#"
import "forge-std/Script.sol";

contract GasWaster {
    function wasteGas(uint256 minGas) public {
        require(gasleft() >= minGas, "Gas left needs to be higher");
    }
}
contract DeployScript is Script {
    function run() external {
        vm.startBroadcast();
        GasWaster gasWaster = new GasWaster();
        gasWaster.wasteGas{gas: 500000}(200000);
    }
}
   "#,
        )
        .unwrap();

    let deploy_contract = deploy_script.display().to_string() + ":DeployScript";

    let node_config =
        NodeConfig::test().with_eth_rpc_url(Some(rpc::next_http_archive_rpc_endpoint())).silent();
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
        "--slow",
        "--broadcast",
        "--private-key",
        &private_key,
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (2018): Function state mutability can be restricted to view
 [FILE]:7:5:
  |
7 |     function wasteGas(uint256 minGas) public {
  |     ^ (Relevant source part starts here and spans across multiple lines).

Traces:
  [..] DeployScript::run()
    ├─ [0] VM::startBroadcast()
    │   └─ ← [Return] 
    ├─ [..] → new GasWaster@[..]
    │   └─ ← [Return] 226 bytes of code
    ├─ [..] GasWaster::wasteGas(200000 [2e5])
    │   └─ ← [Stop] 
    └─ ← [Stop] 


Script ran successfully.

## Setting up 1 EVM.
==========================
Simulated On-chain Traces:

  [45299] → new GasWaster@[..]
    └─ ← [Return] 226 bytes of code

  [226] GasWaster::wasteGas(200000 [2e5])
    └─ ← [Stop] 


==========================

Chain 1

[ESTIMATED_GAS_PRICE]

[ESTIMATED_TOTAL_GAS_USED]

[ESTIMATED_AMOUNT_REQUIRED]

==========================


==========================

ONCHAIN EXECUTION COMPLETE & SUCCESSFUL.

[SAVED_TRANSACTIONS]

[SAVED_SENSITIVE_VALUES]


"#]]);
});

// Tests that the run command can run functions with arguments
forgetest!(can_execute_script_command_with_args, |prj, cmd| {
    let script = prj
        .add_source(
            "Foo",
            r#"
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

    cmd.arg("script")
        .arg(script)
        .arg("--sig")
        .arg("run(uint256,uint256)")
        .arg("1")
        .arg("2")
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Script ran successfully.
[GAS]

== Logs ==
  script ran
  1
  2

"#]]);
});

// Tests that the run command can run functions with return values
forgetest!(can_execute_script_command_with_returned, |prj, cmd| {
    let script = prj
        .add_source(
            "Foo",
            r#"
contract Demo {
    event log_string(string);
    function run() external returns (uint256 result, uint8) {
        emit log_string("script ran");
        return (255, 3);
    }
}"#,
        )
        .unwrap();

    cmd.arg("script").arg(script).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Script ran successfully.
[GAS]

== Return ==
result: uint256 255
1: uint8 3

== Logs ==
  script ran

"#]]);
});

forgetest_async!(can_broadcast_script_skipping_simulation, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());
    // This example script would fail in on-chain simulation
    let deploy_script = prj
        .add_source(
            "DeployScript",
            r#"
import "forge-std/Script.sol";

contract HashChecker {
    bytes32 public lastHash;

    function update() public {
        bytes32 newHash = blockhash(block.number - 1);
        require(newHash != lastHash, "Hash didn't change");
        lastHash = newHash;
    }

    function checkLastHash() public view {
        require(lastHash != bytes32(0), "Hash shouldn't be zero");
    }
}

contract DeployScript is Script {
    HashChecker public hashChecker;

    function run() external {
        vm.startBroadcast();
        hashChecker = new HashChecker();
    }
}"#,
        )
        .unwrap();

    let deploy_contract = deploy_script.display().to_string() + ":DeployScript";

    let node_config =
        NodeConfig::test().with_eth_rpc_url(Some(rpc::next_http_archive_rpc_endpoint())).silent();
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
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Traces:
  [..] DeployScript::run()
    ├─ [0] VM::startBroadcast()
    │   └─ ← [Return] 
    ├─ [..] → new HashChecker@[..]
    │   └─ ← [Return] 378 bytes of code
    └─ ← [Stop] 


Script ran successfully.

SKIPPING ON CHAIN SIMULATION.


==========================

ONCHAIN EXECUTION COMPLETE & SUCCESSFUL.

[SAVED_TRANSACTIONS]

[SAVED_SENSITIVE_VALUES]


"#]]);

    let run_log = std::fs::read_to_string("broadcast/DeployScript.sol/1/run-latest.json").unwrap();
    let run_object: Value = serde_json::from_str(&run_log).unwrap();
    let contract_address = &run_object["receipts"][0]["contractAddress"]
        .as_str()
        .unwrap()
        .parse::<Address>()
        .unwrap()
        .to_string();

    let run_code = r#"
import "forge-std/Script.sol";
import { HashChecker } from "./DeployScript.sol";

contract RunScript is Script {
    HashChecker public hashChecker;

    function run() external {
        vm.startBroadcast();
        hashChecker = HashChecker(CONTRACT_ADDRESS);
        uint numUpdates = 8;
        vm.roll(block.number - numUpdates);
        for(uint i = 0; i < numUpdates; i++) {
            vm.roll(block.number + 1);
            hashChecker.update();
            hashChecker.checkLastHash();
        }
    }
}"#
    .replace("CONTRACT_ADDRESS", contract_address);

    let run_script = prj.add_source("RunScript", &run_code).unwrap();
    let run_contract = run_script.display().to_string() + ":RunScript";

    cmd.forge_fuse()
        .args([
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
        ])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Traces:
  [..] RunScript::run()
    ├─ [0] VM::startBroadcast()
    │   └─ ← [Return] 
    ├─ [0] VM::roll([..])
    │   └─ ← [Return] 
    ├─ [0] VM::roll([..])
    │   └─ ← [Return] 
    ├─ [..] [..]::update()
    │   └─ ← [Stop] 
    ├─ [..] [..]::checkLastHash() [staticcall]
    │   └─ ← [Stop] 
    ├─ [0] VM::roll([..])
    │   └─ ← [Return] 
    ├─ [..] [..]::update()
    │   └─ ← [Stop] 
    ├─ [..] [..]::checkLastHash() [staticcall]
    │   └─ ← [Stop] 
    ├─ [0] VM::roll([..])
    │   └─ ← [Return] 
    ├─ [..] [..]::update()
    │   └─ ← [Stop] 
    ├─ [..] [..]::checkLastHash() [staticcall]
    │   └─ ← [Stop] 
    ├─ [0] VM::roll([..])
    │   └─ ← [Return] 
    ├─ [..] [..]::update()
    │   └─ ← [Stop] 
    ├─ [..] [..]::checkLastHash() [staticcall]
    │   └─ ← [Stop] 
    ├─ [0] VM::roll([..])
    │   └─ ← [Return] 
    ├─ [..] [..]::update()
    │   └─ ← [Stop] 
    ├─ [..] [..]::checkLastHash() [staticcall]
    │   └─ ← [Stop] 
    ├─ [0] VM::roll([..])
    │   └─ ← [Return] 
    ├─ [..] [..]::update()
    │   └─ ← [Stop] 
    ├─ [..] [..]::checkLastHash() [staticcall]
    │   └─ ← [Stop] 
    ├─ [0] VM::roll([..])
    │   └─ ← [Return] 
    ├─ [..] [..]::update()
    │   └─ ← [Stop] 
    ├─ [..] [..]::checkLastHash() [staticcall]
    │   └─ ← [Stop] 
    ├─ [0] VM::roll([..])
    │   └─ ← [Return] 
    ├─ [..] [..]::update()
    │   └─ ← [Stop] 
    ├─ [..] [..]::checkLastHash() [staticcall]
    │   └─ ← [Stop] 
    └─ ← [Stop] 


Script ran successfully.

SKIPPING ON CHAIN SIMULATION.


==========================

ONCHAIN EXECUTION COMPLETE & SUCCESSFUL.

[SAVED_TRANSACTIONS]

[SAVED_SENSITIVE_VALUES]


"#]]);
});

forgetest_async!(can_deploy_script_without_lib, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("BroadcastTestNoLinking", "deployDoesntPanic()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 1), (1, 2)])
        .await;
});

forgetest_async!(can_deploy_script_with_lib, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("BroadcastTest", "deploy()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 2), (1, 1)])
        .await;
});

forgetest_async!(can_deploy_script_private_key, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_addresses(&[Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap()])
        .await
        .add_sig("BroadcastTest", "deployPrivateKey()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment_addresses(&[(
            Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap(),
            3,
        )])
        .await;
});

forgetest_async!(can_deploy_unlocked, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .sender("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".parse().unwrap())
        .unlocked()
        .add_sig("BroadcastTest", "deployOther()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast);
});

forgetest_async!(can_deploy_script_remember_key, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_addresses(&[Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap()])
        .await
        .add_sig("BroadcastTest", "deployRememberKey()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment_addresses(&[(
            Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap(),
            2,
        )])
        .await;
});

forgetest_async!(can_deploy_script_remember_key_and_resume, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .add_deployer(0)
        .load_addresses(&[Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap()])
        .await
        .add_sig("BroadcastTest", "deployRememberKeyResume()")
        .simulate(ScriptOutcome::OkSimulation)
        .resume(ScriptOutcome::MissingWallet)
        // load missing wallet
        .load_private_keys(&[0])
        .await
        .run(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment_addresses(&[(
            Address::from_str("0x90F79bf6EB2c4f870365E785982E1f101E93b906").unwrap(),
            1,
        )])
        .await
        .assert_nonce_increment(&[(0, 2)])
        .await;
});

forgetest_async!(can_resume_script, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(&[0])
        .await
        .add_sig("BroadcastTest", "deploy()")
        .simulate(ScriptOutcome::OkSimulation)
        .resume(ScriptOutcome::MissingWallet)
        // load missing wallet
        .load_private_keys(&[1])
        .await
        .run(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 2), (1, 1)])
        .await;
});

forgetest_async!(can_deploy_broadcast_wrap, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .add_deployer(2)
        .load_private_keys(&[0, 1, 2])
        .await
        .add_sig("BroadcastTest", "deployOther()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 4), (1, 4), (2, 1)])
        .await;
});

forgetest_async!(panic_no_deployer_set, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("BroadcastTest", "deployOther()")
        .simulate(ScriptOutcome::WarnSpecifyDeployer)
        .broadcast(ScriptOutcome::MissingSender);
});

forgetest_async!(can_deploy_no_arg_broadcast, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .add_deployer(0)
        .load_private_keys(&[0])
        .await
        .add_sig("BroadcastTest", "deployNoArgs()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 3)])
        .await;
});

forgetest_async!(can_deploy_with_create2, |prj, cmd| {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    // Prepare CREATE2 Deployer
    api.anvil_set_code(
        foundry_evm::constants::DEFAULT_CREATE2_DEPLOYER,
        Bytes::from_static(foundry_evm::constants::DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE),
    )
    .await
    .unwrap();

    tester
        .add_deployer(0)
        .load_private_keys(&[0])
        .await
        .add_sig("BroadcastTestNoLinking", "deployCreate2()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 2)])
        .await
        // Running again results in error, since we're repeating the salt passed to CREATE2
        .run(ScriptOutcome::ScriptFailed);
});

forgetest_async!(can_deploy_and_simulate_25_txes_concurrently, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(&[0])
        .await
        .add_sig("BroadcastTestNoLinking", "deployMany()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 25)])
        .await;
});

forgetest_async!(can_deploy_and_simulate_mixed_broadcast_modes, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(&[0])
        .await
        .add_sig("BroadcastMix", "deployMix()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 15)])
        .await;
});

forgetest_async!(deploy_with_setup, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(&[0])
        .await
        .add_sig("BroadcastTestSetup", "run()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 6)])
        .await;
});

forgetest_async!(fail_broadcast_staticcall, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(&[0])
        .await
        .add_sig("BroadcastTestNoLinking", "errorStaticCall()")
        .simulate(ScriptOutcome::StaticCallNotAllowed);
});

forgetest_async!(check_broadcast_log, |prj, cmd| {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    // Prepare CREATE2 Deployer
    let addr = Address::from_str("0x4e59b44847b379578588920ca78fbf26c0b4956c").unwrap();
    let code = hex::decode("7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3").expect("Could not decode create2 deployer init_code").into();
    api.anvil_set_code(addr, code).await.unwrap();

    tester
        .load_private_keys(&[0])
        .await
        .add_sig("BroadcastTestSetup", "run()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 6)])
        .await;

    // Uncomment to recreate the broadcast log
    // std::fs::copy(
    //     "broadcast/Broadcast.t.sol/31337/run-latest.json",
    //     PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/fixtures/broadcast.
    // log. json" ), );

    // Check broadcast logs
    // Ignore timestamp, blockHash, blockNumber, cumulativeGasUsed, effectiveGasPrice,
    // transactionIndex and logIndex values since they can change in between runs
    let re = Regex::new(r#"((timestamp":).[0-9]*)|((blockHash":).*)|((blockNumber":).*)|((cumulativeGasUsed":).*)|((effectiveGasPrice":).*)|((transactionIndex":).*)|((logIndex":).*)"#).unwrap();

    let fixtures_log = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/fixtures/broadcast.log.json"),
    )
    .unwrap();
    let _fixtures_log = re.replace_all(&fixtures_log, "");

    let run_log =
        std::fs::read_to_string("broadcast/Broadcast.t.sol/31337/run-latest.json").unwrap();
    let _run_log = re.replace_all(&run_log, "");

    // similar_asserts::assert_eq!(fixtures_log, run_log);

    // Uncomment to recreate the sensitive log
    // std::fs::copy(
    //     "cache/Broadcast.t.sol/31337/run-latest.json",
    //     PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    //         .join("../../testdata/fixtures/broadcast.sensitive.log.json"),
    // );

    // Check sensitive logs
    // Ignore port number since it can change in between runs
    let re = Regex::new(r":[0-9]+").unwrap();

    let fixtures_log = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/fixtures/broadcast.sensitive.log.json"),
    )
    .unwrap();
    let fixtures_log = re.replace_all(&fixtures_log, "");

    let run_log = std::fs::read_to_string("cache/Broadcast.t.sol/31337/run-latest.json").unwrap();
    let run_log = re.replace_all(&run_log, "");

    // Clean up carriage return OS differences
    let re = Regex::new(r"\r\n").unwrap();
    let fixtures_log = re.replace_all(&fixtures_log, "\n");
    let run_log = re.replace_all(&run_log, "\n");

    similar_asserts::assert_eq!(fixtures_log, run_log);
});

forgetest_async!(test_default_sender_balance, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    // Expect the default sender to have uint256.max balance.
    tester
        .add_sig("TestInitialBalance", "runDefaultSender()")
        .simulate(ScriptOutcome::OkSimulation);
});

forgetest_async!(test_custom_sender_balance, |prj, cmd| {
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
forgetest_async!(can_execute_script_with_arguments, |prj, cmd| {
    cmd.args(["init", "--force"]).arg(prj.root()).assert_success().stdout_eq(str![[r#"
Target directory is not empty, but `--force` was specified
Initializing [..]...
Installing forge-std in [..] (url: Some("https://github.com/foundry-rs/forge-std"), tag: None)
    Installed forge-std [..]
    Initialized forge project

"#]]);

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let script = prj.add_script(
                "Counter.s.sol",
                r#"
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
    bytes f;
    Point g;
    string h;

  constructor(address _a, uint _b, int _c, bytes32 _d, bool _e, bytes memory _f, Point memory _g, string memory _h) {
    a = _a;
    b = _b;
    c = _c;
    d = _d;
    e = _e;
    f = _f;
    g = _g;
    h = _h;
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

    cmd
        .forge_fuse()
        .arg("script")
        .arg(script)
        .args([
            "--tc",
            "Script0",
            "--sender",
            "0x00a329c0648769A73afAc7F9381E08FB43dBEA72",
            "--rpc-url",
            handle.http_endpoint().as_str(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
...
Script ran successfully.

## Setting up 1 EVM.

==========================

Chain 31337

[ESTIMATED_GAS_PRICE]

[ESTIMATED_TOTAL_GAS_USED]

[ESTIMATED_AMOUNT_REQUIRED]

==========================

SIMULATION COMPLETE. To broadcast these transactions, add --broadcast and wallet configuration(s) to the previous command. See forge script --help for more.

[SAVED_TRANSACTIONS]

[SAVED_SENSITIVE_VALUES]


"#]]);

    let run_latest = foundry_common::fs::json_files(&prj.root().join("broadcast"))
        .find(|path| path.ends_with("run-latest.json"))
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
});

// test we output arguments <https://github.com/foundry-rs/foundry/issues/3053>
forgetest_async!(can_execute_script_with_arguments_nested_deploy, |prj, cmd| {
    cmd.args(["init", "--force"]).arg(prj.root()).assert_success().stdout_eq(str![[r#"
Target directory is not empty, but `--force` was specified
Initializing [..]...
Installing forge-std in [..] (url: Some("https://github.com/foundry-rs/forge-std"), tag: None)
    Installed forge-std [..]
    Initialized forge project

"#]]);

    let (_api, handle) = spawn(NodeConfig::test()).await;
    let script = prj
        .add_script(
            "Counter.s.sol",
            r#"
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

    cmd
        .forge_fuse()
        .arg("script")
        .arg(script)
        .args([
            "--tc",
            "Script0",
            "--sender",
            "0x00a329c0648769A73afAc7F9381E08FB43dBEA72",
            "--rpc-url",
            handle.http_endpoint().as_str(),
        ])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
...
Script ran successfully.

## Setting up 1 EVM.

==========================

Chain 31337

[ESTIMATED_GAS_PRICE]

[ESTIMATED_TOTAL_GAS_USED]

[ESTIMATED_AMOUNT_REQUIRED]

==========================

SIMULATION COMPLETE. To broadcast these transactions, add --broadcast and wallet configuration(s) to the previous command. See forge script --help for more.

[SAVED_TRANSACTIONS]

[SAVED_SENSITIVE_VALUES]


"#]]);

    let run_latest = foundry_common::fs::json_files(&prj.root().join("broadcast"))
        .find(|file| file.ends_with("run-latest.json"))
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
});

// checks that skipping build
forgetest_init!(can_execute_script_and_skip_contracts, |prj, cmd| {
    let script = prj
        .add_source(
            "Foo",
            r#"
contract Demo {
    event log_string(string);
    function run() external returns (uint256 result, uint8) {
        emit log_string("script ran");
        return (255, 3);
    }
}"#,
        )
        .unwrap();
    cmd.arg("script")
        .arg(script)
        .args(["--skip", "tests", "--skip", TEMPLATE_CONTRACT])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Script ran successfully.
[GAS]

== Return ==
result: uint256 255
1: uint8 3

== Logs ==
  script ran

"#]]);
});

forgetest_async!(can_run_script_with_empty_setup, |prj, cmd| {
    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());

    tester.add_sig("BroadcastEmptySetUp", "run()").simulate(ScriptOutcome::OkNoEndpoint);
});

forgetest_async!(does_script_override_correctly, |prj, cmd| {
    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());

    tester.add_sig("CheckOverrides", "run()").simulate(ScriptOutcome::OkNoEndpoint);
});

forgetest_async!(assert_tx_origin_is_not_overritten, |prj, cmd| {
    cmd.args(["init", "--force"]).arg(prj.root()).assert_success().stdout_eq(str![[r#"
Target directory is not empty, but `--force` was specified
Initializing [..]...
Installing forge-std in [..] (url: Some("https://github.com/foundry-rs/forge-std"), tag: None)
    Installed forge-std [..]
    Initialized forge project

"#]]);

    let script = prj
        .add_script(
            "ScriptTxOrigin.s.sol",
            r#"
import { Script } from "forge-std/Script.sol";

contract ScriptTxOrigin is Script {
    function run() public {
        uint256 pk = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
        vm.startBroadcast(pk); // 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266

        ContractA contractA = new ContractA();
        ContractB contractB = new ContractB();

        contractA.test(address(contractB));
        contractB.method(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);

        require(tx.origin == 0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38);
        vm.stopBroadcast();
    }
}

contract ContractA {
    function test(address _contractB) public {
        require(msg.sender == 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, "sender 1");
        require(tx.origin == 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, "origin 1");
        ContractB contractB = ContractB(_contractB);
        ContractC contractC = new ContractC();
        require(msg.sender == 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, "sender 2");
        require(tx.origin == 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, "origin 2");
        contractB.method(address(this));
        contractC.method(address(this));
        require(msg.sender == 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, "sender 3");
        require(tx.origin == 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, "origin 3");
    }
}

contract ContractB {
    function method(address sender) public view {
        require(msg.sender == sender, "sender");
        require(tx.origin == 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, "origin");
    }
}

contract ContractC {
    function method(address sender) public view {
        require(msg.sender == sender, "sender");
        require(tx.origin == 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, "origin");
    }
}
   "#,
        )
        .unwrap();

    cmd.forge_fuse()
        .arg("script")
        .arg(script)
        .args(["--tc", "ScriptTxOrigin"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Script ran successfully.
[GAS]

If you wish to simulate on-chain transactions pass a RPC URL.

"#]]);
});

forgetest_async!(assert_can_create_multiple_contracts_with_correct_nonce, |prj, cmd| {
    cmd.args(["init", "--force"]).arg(prj.root()).assert_success().stdout_eq(str![[r#"
Target directory is not empty, but `--force` was specified
Initializing [..]...
Installing forge-std in [..] (url: Some("https://github.com/foundry-rs/forge-std"), tag: None)
    Installed forge-std [..]
    Initialized forge project

"#]]);

    let script = prj
        .add_script(
            "ScriptTxOrigin.s.sol",
            r#"
import {Script, console} from "forge-std/Script.sol";

contract Contract {
  constructor() {
    console.log(tx.origin);
  }
}

contract SubContract {
  constructor() {
    console.log(tx.origin);
  }
}

contract BadContract {
  constructor() {
    new SubContract();
    console.log(tx.origin);
  }
}
contract NestedCreate is Script {
  function run() public {
    address sender = address(uint160(uint(keccak256("woops"))));

    vm.broadcast(sender);
    new BadContract();

    vm.broadcast(sender);
    new Contract();
  }
}
   "#,
        )
        .unwrap();

    cmd.forge_fuse()
        .arg("script")
        .arg(script)
        .args(["--tc", "NestedCreate"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Script ran successfully.
[GAS]

== Logs ==
  0x159E2f2F1C094625A2c6c8bF59526d91454c2F3c
  0x159E2f2F1C094625A2c6c8bF59526d91454c2F3c
  0x159E2f2F1C094625A2c6c8bF59526d91454c2F3c

If you wish to simulate on-chain transactions pass a RPC URL.

"#]]);
});

forgetest_async!(assert_can_detect_target_contract_with_interfaces, |prj, cmd| {
    let script = prj
        .add_script(
            "ScriptWithInterface.s.sol",
            r#"
contract Script {
  function run() external {}
}

interface Interface {}
            "#,
        )
        .unwrap();

    cmd.arg("script").arg(script).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Script ran successfully.
[GAS]

"#]]);
});

forgetest_async!(assert_can_detect_unlinked_target_with_libraries, |prj, cmd| {
    let script = prj
        .add_script(
            "ScriptWithExtLib.s.sol",
            r#"
library Lib {
    function f() public {}
}

contract Script {
    function run() external {
        Lib.f();
    }
}
            "#,
        )
        .unwrap();

    cmd.arg("script").arg(script).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Script ran successfully.
[GAS]

If you wish to simulate on-chain transactions pass a RPC URL.

"#]]);
});

forgetest_async!(assert_can_resume_with_additional_contracts, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;
    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .add_deployer(0)
        .add_sig("ScriptAdditionalContracts", "run()")
        .broadcast(ScriptOutcome::MissingWallet)
        .load_private_keys(&[0])
        .await
        .resume(ScriptOutcome::OkBroadcast);
});

forgetest_async!(can_detect_contract_when_multiple_versions, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());

    prj.add_script(
        "A.sol",
        r#"pragma solidity 0.8.20;
import "./B.sol";

contract ScriptA {}
"#,
    )
    .unwrap();

    prj.add_script(
        "B.sol",
        r#"pragma solidity >=0.8.5 <=0.8.20;
import 'forge-std/Script.sol';

contract ScriptB is Script {
    function run() external {
        vm.broadcast();
        address(0).call("");
    }
}
"#,
    )
    .unwrap();

    prj.add_script(
        "C.sol",
        r#"pragma solidity 0.8.5;
import "./B.sol";

contract ScriptC {}
"#,
    )
    .unwrap();

    let mut tester = ScriptTester::new(cmd, None, prj.root(), "script/B.sol");
    tester.cmd.forge_fuse().args(["script", "script/B.sol"]);
    tester.simulate(ScriptOutcome::OkNoEndpoint);
});

forgetest_async!(can_sign_with_script_wallet_single, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());

    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());
    tester
        .add_sig("ScriptSign", "run()")
        .load_private_keys(&[0])
        .await
        .simulate(ScriptOutcome::OkNoEndpoint);
});

forgetest_async!(can_sign_with_script_wallet_multiple, |prj, cmd| {
    let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());
    let acc = tester.accounts_pub[0].to_checksum(None);
    tester
        .add_sig("ScriptSign", "run(address)")
        .arg(&acc)
        .load_private_keys(&[0, 1, 2])
        .await
        .simulate(ScriptOutcome::OkRun);
});

forgetest_async!(fails_with_function_name_and_overloads, |prj, cmd| {
    let script = prj
        .add_script(
            "Script.s.sol",
            r#"
contract Script {
    function run() external {}

    function run(address,uint256) external {}
}
            "#,
        )
        .unwrap();

    cmd.arg("script").args([&script.to_string_lossy(), "--sig", "run"]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: 
Multiple functions with the same name `run` found in the ABI

"#]]);
});

forgetest_async!(can_decode_custom_errors, |prj, cmd| {
    cmd.args(["init", "--force"]).arg(prj.root()).assert_success().stdout_eq(str![[r#"
Target directory is not empty, but `--force` was specified
Initializing [..]...
Installing forge-std in [..] (url: Some("https://github.com/foundry-rs/forge-std"), tag: None)
    Installed forge-std [..]
    Initialized forge project

"#]]);

    let script = prj
        .add_script(
            "CustomErrorScript.s.sol",
            r#"
import { Script } from "forge-std/Script.sol";

contract ContractWithCustomError {
    error CustomError();

    constructor() {
        revert CustomError();
    }
}

contract CustomErrorScript is Script {
    ContractWithCustomError test;

    function run() public {
        test = new ContractWithCustomError();
    }
}
"#,
        )
        .unwrap();

    cmd.forge_fuse().arg("script").arg(script).args(["--tc", "CustomErrorScript"]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: 
script failed: CustomError()

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/7620
forgetest_async!(can_run_zero_base_fee, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());
    prj.add_script(
        "Foo",
        r#"
import "forge-std/Script.sol";

contract SimpleScript is Script {
    function run() external returns (bool success) {
        vm.startBroadcast();
        (success, ) = address(0).call("");
    }
}
   "#,
    )
    .unwrap();

    let node_config = NodeConfig::test().with_base_fee(Some(0));
    let (_api, handle) = spawn(node_config).await;
    let dev = handle.dev_accounts().next().unwrap();

    // Firstly run script with non-zero gas prices to ensure that eth_feeHistory contains non-zero
    // values.
    cmd.args([
        "script",
        "SimpleScript",
        "--fork-url",
        &handle.http_endpoint(),
        "--sender",
        format!("{dev:?}").as_str(),
        "--broadcast",
        "--unlocked",
        "--with-gas-price",
        "2000000",
        "--priority-gas-price",
        "100000",
        "--non-interactive",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
...
Script ran successfully.

== Return ==
success: bool true

## Setting up 1 EVM.
Script contains a transaction to 0x0000000000000000000000000000000000000000 which does not contain any code.

==========================

Chain 31337

[ESTIMATED_GAS_PRICE]

[ESTIMATED_TOTAL_GAS_USED]

[ESTIMATED_AMOUNT_REQUIRED]

==========================


==========================

ONCHAIN EXECUTION COMPLETE & SUCCESSFUL.

[SAVED_TRANSACTIONS]

[SAVED_SENSITIVE_VALUES]


"#]]);

    // Ensure that we can correctly estimate gas when base fee is zero but priority fee is not.
    cmd.forge_fuse()
        .args([
            "script",
            "SimpleScript",
            "--fork-url",
            &handle.http_endpoint(),
            "--sender",
            format!("{dev:?}").as_str(),
            "--broadcast",
            "--unlocked",
            "--non-interactive",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
No files changed, compilation skipped
...
Script ran successfully.

== Return ==
success: bool true

## Setting up 1 EVM.
Script contains a transaction to 0x0000000000000000000000000000000000000000 which does not contain any code.

==========================

Chain 31337

[ESTIMATED_GAS_PRICE]

[ESTIMATED_TOTAL_GAS_USED]

[ESTIMATED_AMOUNT_REQUIRED]

==========================


==========================

ONCHAIN EXECUTION COMPLETE & SUCCESSFUL.

[SAVED_TRANSACTIONS]

[SAVED_SENSITIVE_VALUES]


"#]]);
});

// https://github.com/foundry-rs/foundry/pull/7742
forgetest_async!(unlocked_no_sender, |prj, cmd| {
    foundry_test_utils::util::initialize(prj.root());
    prj.add_script(
        "Foo",
        r#"
import "forge-std/Script.sol";

contract SimpleScript is Script {
    function run() external returns (bool success) {
        vm.startBroadcast(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
        (success, ) = address(0).call("");
    }
}
   "#,
    )
    .unwrap();

    let (_api, handle) = spawn(NodeConfig::test()).await;

    cmd.args([
        "script",
        "SimpleScript",
        "--fork-url",
        &handle.http_endpoint(),
        "--broadcast",
        "--unlocked",
        "--non-interactive",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
...
Script ran successfully.

== Return ==
success: bool true

## Setting up 1 EVM.
Script contains a transaction to 0x0000000000000000000000000000000000000000 which does not contain any code.

==========================

Chain 31337

[ESTIMATED_GAS_PRICE]

[ESTIMATED_TOTAL_GAS_USED]

[ESTIMATED_AMOUNT_REQUIRED]

==========================


==========================

ONCHAIN EXECUTION COMPLETE & SUCCESSFUL.

[SAVED_TRANSACTIONS]

[SAVED_SENSITIVE_VALUES]


"#]]);
});

// https://github.com/foundry-rs/foundry/issues/7833
forgetest_async!(error_no_create2, |prj, cmd| {
    let (_api, handle) =
        spawn(NodeConfig::test().with_disable_default_create2_deployer(true)).await;

    foundry_test_utils::util::initialize(prj.root());
    prj.add_script(
        "Foo",
        r#"
import "forge-std/Script.sol";

contract SimpleContract {}

contract SimpleScript is Script {
    function run() external {
        vm.startBroadcast(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
        new SimpleContract{salt: bytes32(0)}();
    }
}
   "#,
    )
    .unwrap();

    cmd.args([
        "script",
        "SimpleScript",
        "--fork-url",
        &handle.http_endpoint(),
        "--broadcast",
        "--unlocked",
    ]);

    cmd.assert_failure().stderr_eq(str![[r#"
Error: 
script failed: missing CREATE2 deployer

"#]]);
});

forgetest_async!(can_switch_forks_in_setup, |prj, cmd| {
    let (_api, handle) =
        spawn(NodeConfig::test().with_disable_default_create2_deployer(true)).await;

    foundry_test_utils::util::initialize(prj.root());
    let url = handle.http_endpoint();

    prj.add_script(
        "Foo",
        &r#"
import "forge-std/Script.sol";

contract SimpleScript is Script {
    function setUp() external {
        uint256 initialFork = vm.activeFork();
        vm.createSelectFork("<url>");
        vm.selectFork(initialFork);
    }

    function run() external {
        assert(vm.getNonce(msg.sender) == 0);
    }
}
   "#
        .replace("<url>", &url),
    )
    .unwrap();

    cmd.args([
        "script",
        "SimpleScript",
        "--fork-url",
        &url,
        "--sender",
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (2018): Function state mutability can be restricted to view
  [FILE]:13:5:
   |
13 |     function run() external {
   |     ^ (Relevant source part starts here and spans across multiple lines).

Script ran successfully.

"#]]);
});

// Asserts that running the same script twice only deploys library once.
forgetest_async!(can_deploy_library_create2, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("BroadcastTest", "deploy()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 2), (1, 1)])
        .await;

    tester.clear();

    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("BroadcastTest", "deploy()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 1), (1, 1)])
        .await;
});

// Asserts that running the same script twice only deploys library once when using different
// senders.
forgetest_async!(can_deploy_library_create2_different_sender, |prj, cmd| {
    let (_api, handle) = spawn(NodeConfig::test()).await;

    let mut tester = ScriptTester::new_broadcast(cmd, &handle.http_endpoint(), prj.root());

    tester
        .load_private_keys(&[0, 1])
        .await
        .add_sig("BroadcastTest", "deploy()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(0, 2), (1, 1)])
        .await;

    tester.clear();

    // Run different script from the same contract (which requires the same library).
    tester
        .load_private_keys(&[2])
        .await
        .add_sig("BroadcastTest", "deployNoArgs()")
        .simulate(ScriptOutcome::OkSimulation)
        .broadcast(ScriptOutcome::OkBroadcast)
        .assert_nonce_increment(&[(2, 2)])
        .await;
});
