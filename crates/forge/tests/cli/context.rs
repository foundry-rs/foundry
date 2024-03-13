//! Contains tests for checking test and snapshot context cheatcodes
const FORGE_TEST_CONTEXT_CONTRACT: &str = r#"
import "./test.sol";
interface Vm {
    function isTestContext() external view returns (bool isTest);
    function isTestCoverageContext() external view returns (bool isCoverage);
    function isTestSnapshotContext() external view returns (bool isSnapshot);
    function isTestStandardContext() external view returns (bool isTest);
}

contract ForgeContextTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testForgeTestStandardContext() external view {
        require(vm.isTestContext(), "wrong context");
        require(vm.isTestStandardContext(), "wrong context");
        require(!vm.isTestSnapshotContext(), "wrong context");
        require(!vm.isTestCoverageContext(), "wrong context");
    }

    function testForgeTestSnapshotContext() external view {
        require(vm.isTestContext(), "wrong context");
        require(vm.isTestSnapshotContext(), "wrong context");
        require(!vm.isTestStandardContext(), "wrong context");
        require(!vm.isTestCoverageContext(), "wrong context");
    }

    function testForgeTestCoverageContext() external view {
        require(vm.isTestContext(), "wrong context");
        require(vm.isTestCoverageContext(), "wrong context");
        require(!vm.isTestStandardContext(), "wrong context");
        require(!vm.isTestSnapshotContext(), "wrong context");
    }
}
   "#;

// tests that context properly set for `forge test` command
forgetest!(can_set_forge_test_standard_context, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source("ForgeContextTest.t.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.args(["test", "--match-test", "testForgeTestStandardContext"])
        .assert_success();
});

// tests that context properly set for `forge snapshot` command
forgetest!(can_set_forge_test_snapshot_context, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source("ForgeContextTest.t.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.args(["snapshot", "--match-test", "testForgeTestSnapshotContext"])
        .assert_success();
});

// tests that context properly set for `forge coverage` command
forgetest!(can_set_forge_test_coverage_context, |prj, cmd| {
    prj.insert_ds_test();
    prj.add_source("ForgeContextTest.t.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();
    cmd.args(["coverage", "--match-test", "testForgeTestCoverageContext"])
        .assert_success();
});
