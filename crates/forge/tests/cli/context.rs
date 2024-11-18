//! Contains tests for checking forge execution context cheatcodes
const FORGE_TEST_CONTEXT_CONTRACT: &str = r#"
import "./test.sol";
interface Vm {
    enum ForgeContext { TestGroup, Test, Coverage, Snapshot, ScriptGroup, ScriptDryRun, ScriptBroadcast, ScriptResume, Unknown }
    function isContext(ForgeContext context) external view returns (bool isContext);
}

contract ForgeContextTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testForgeTestContext() external view {
        require(vm.isContext(Vm.ForgeContext.TestGroup) && !vm.isContext(Vm.ForgeContext.ScriptGroup), "wrong context");
        require(vm.isContext(Vm.ForgeContext.Test), "wrong context");
        require(!vm.isContext(Vm.ForgeContext.Coverage), "wrong context");
        require(!vm.isContext(Vm.ForgeContext.Snapshot), "wrong context");
    }
    function testForgeSnapshotContext() external view {
        require(vm.isContext(Vm.ForgeContext.TestGroup) && !vm.isContext(Vm.ForgeContext.ScriptGroup), "wrong context");
        require(vm.isContext(Vm.ForgeContext.Snapshot), "wrong context");
        require(!vm.isContext(Vm.ForgeContext.Test), "wrong context");
        require(!vm.isContext(Vm.ForgeContext.Coverage), "wrong context");
    }
    function testForgeCoverageContext() external view {
        require(vm.isContext(Vm.ForgeContext.TestGroup) && !vm.isContext(Vm.ForgeContext.ScriptGroup), "wrong context");
        require(vm.isContext(Vm.ForgeContext.Coverage), "wrong context");
        require(!vm.isContext(Vm.ForgeContext.Test), "wrong context");
        require(!vm.isContext(Vm.ForgeContext.Snapshot), "wrong context");
    }

    function runDryRun() external view {
        require(vm.isContext(Vm.ForgeContext.ScriptGroup) && !vm.isContext(Vm.ForgeContext.TestGroup), "wrong context");
        require(vm.isContext(Vm.ForgeContext.ScriptDryRun), "wrong context");
        require(!vm.isContext(Vm.ForgeContext.ScriptBroadcast), "wrong context");
        require(!vm.isContext(Vm.ForgeContext.ScriptResume), "wrong context");
    }
    function runBroadcast() external view {
        require(vm.isContext(Vm.ForgeContext.ScriptGroup) && !vm.isContext(Vm.ForgeContext.TestGroup), "wrong context");
        require(vm.isContext(Vm.ForgeContext.ScriptBroadcast), "wrong context");
        require(!vm.isContext(Vm.ForgeContext.ScriptDryRun), "wrong context");
        require(!vm.isContext(Vm.ForgeContext.ScriptResume), "wrong context");
    }
}
   "#;

// tests that context properly set for `forge test` command
forgetest!(can_set_forge_test_standard_context, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source("ForgeContextTest.t.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.args(["test", "--match-test", "testForgeTestContext"]).assert_success();
});

// tests that context properly set for `forge snapshot` command
forgetest!(can_set_forge_test_snapshot_context, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source("ForgeContextTest.t.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.args(["snapshot", "--match-test", "testForgeSnapshotContext"]).assert_success();
});

// tests that context properly set for `forge coverage` command
forgetest!(can_set_forge_test_coverage_context, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source("ForgeContextTest.t.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.args(["coverage", "--match-test", "testForgeCoverageContext"]).assert_success();
});

// tests that context properly set for `forge script` command
forgetest!(can_set_forge_script_dry_run_context, |prj, cmd| {
    prj.insert_ds_test();
    let script =
        prj.add_source("ForgeScriptContextTest.s.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.arg("script").arg(script).args(["--sig", "runDryRun()"]).assert_success();
});

// tests that context properly set for `forge script --broadcast` command
forgetest!(can_set_forge_script_broadcast_context, |prj, cmd| {
    prj.insert_ds_test();
    let script =
        prj.add_source("ForgeScriptContextTest.s.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.arg("script").arg(script).args(["--broadcast", "--sig", "runBroadcast()"]).assert_success();
});
