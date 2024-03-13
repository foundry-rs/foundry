//! Contains tests for checking forge execution context cheatcodes
const FORGE_TEST_CONTEXT_CONTRACT: &str = r#"
import "./test.sol";
interface Vm {
    function isTestContext() external view returns (bool isTest);
    function isTestCoverageContext() external view returns (bool isCoverage);
    function isTestSnapshotContext() external view returns (bool isSnapshot);
    function isTestStandardContext() external view returns (bool isTest);
    function isScriptContext() external view returns (bool isScript);
    function isScriptBroadcastContext() external view returns (bool isScriptBroadcast);
    function isScriptDryRunContext() external view returns (bool isScriptDryRun);
    function isScriptResumeContext() external view returns (bool isScriptResume);
}

contract ForgeContextTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testForgeTestStandardContext() external view {
        require(vm.isTestContext() && !vm.isScriptContext(), "wrong context");
        require(vm.isTestStandardContext(), "wrong context");
        require(!vm.isTestSnapshotContext(), "wrong context");
        require(!vm.isTestCoverageContext(), "wrong context");
        require(!vm.isScriptContext(), "wrong context");
    }
    function testForgeTestSnapshotContext() external view {
        require(vm.isTestContext() && !vm.isScriptContext(), "wrong context");
        require(vm.isTestSnapshotContext(), "wrong context");
        require(!vm.isTestStandardContext(), "wrong context");
        require(!vm.isTestCoverageContext(), "wrong context");
        require(!vm.isScriptContext(), "wrong context");
    }
    function testForgeTestCoverageContext() external view {
        require(vm.isTestContext() && !vm.isScriptContext(), "wrong context");
        require(vm.isTestCoverageContext(), "wrong context");
        require(!vm.isTestStandardContext(), "wrong context");
        require(!vm.isTestSnapshotContext(), "wrong context");
    }

    function runDryRun() external view {
        require(vm.isScriptContext() && !vm.isTestContext(), "wrong context");
        require(vm.isScriptDryRunContext(), "wrong context");
        require(!vm.isScriptBroadcastContext(), "wrong context");
        require(!vm.isScriptResumeContext(), "wrong context");
    }
    function runBroadcast() external view {
        require(vm.isScriptContext() && !vm.isTestContext(), "wrong context");
        require(vm.isScriptBroadcastContext(), "wrong context");
        require(!vm.isScriptDryRunContext(), "wrong context");
        require(!vm.isScriptResumeContext(), "wrong context");
    }
}
   "#;

// tests that context properly set for `forge test` command
forgetest!(can_set_forge_test_standard_context, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source("ForgeContextTest.t.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.args(["test", "--match-test", "testForgeTestStandardContext"]).assert_success();
});

// tests that context properly set for `forge snapshot` command
forgetest!(can_set_forge_test_snapshot_context, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source("ForgeContextTest.t.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.args(["snapshot", "--match-test", "testForgeTestSnapshotContext"]).assert_success();
});

// tests that context properly set for `forge coverage` command
forgetest!(can_set_forge_test_coverage_context, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source("ForgeContextTest.t.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.args(["coverage", "--match-test", "testForgeTestCoverageContext"]).assert_success();
});

// tests that context properly set for `forge script` command
forgetest_async!(can_set_forge_script_dry_run_context, |prj, cmd| {
    prj.insert_ds_test();
    let script =
        prj.add_source("ForgeScriptContextTest.s.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.arg("script").arg(script).args(["--sig", "runDryRun()"]).assert_success();
});

// tests that context properly set for `forge script --broadcast` command
forgetest_async!(can_set_forge_script_broadcast_context, |prj, cmd| {
    prj.insert_ds_test();
    let script =
        prj.add_source("ForgeScriptContextTest.s.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.arg("script").arg(script).args(["--broadcast", "--sig", "runBroadcast()"]).assert_success();
});
