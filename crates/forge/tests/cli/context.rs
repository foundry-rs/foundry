//! Contains tests for checking test and snapshot context cheatcodes
const FORGE_TEST_CONTEXT_CONTRACT: &str = r#"
import "./test.sol";
interface Vm {
    function isTestContext() external view returns (bool isTest);
    function isSnapshotContext() external view returns (bool isSnapshot);
}

contract ForgeContextTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testForgeTestContext() external {
        require(vm.isTestContext(), "wrong context");
    }

    function testForgeSnapshotContext() external {
        require(vm.isSnapshotContext(), "wrong context");
    }
}
   "#;

// tests that context properly set for `forge test` command
forgetest!(can_set_forge_test_context, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source("ForgeContextTest.t.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();

    cmd.args(["test", "--match-test", "testForgeTestContext"]);
    assert!(cmd.stdout_lossy().contains("[PASS]") && !cmd.stdout_lossy().contains("[FAIL]"));
});

// tests that context properly set for `forge snapshot` command
forgetest!(can_set_forge_snapshot_context, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source("ForgeContextTest.t.sol", FORGE_TEST_CONTEXT_CONTRACT).unwrap();

    cmd.args(["snapshot", "--match-test", "testForgeSnapshotContext"]);
    assert!(cmd.stdout_lossy().contains("[PASS]") && !cmd.stdout_lossy().contains("[FAIL]"));
});
