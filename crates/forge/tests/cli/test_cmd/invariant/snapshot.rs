use super::*;

// A state deeper than one run: the counter has to reach 20 while each run makes at most 10
// calls. Runs resumed from earlier runs' states reach it, and the reported sequence still
// starts from setUp, so the persisted failure replays.
#[forgetest_init]
fn snapshot_reaches_states_deeper_than_one_run(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.invariant.depth = 10;
        config.invariant.runs = 5000;
        config.fuzz.seed = Some(U256::from(7u32));
    });
    prj.add_test(
        "SnapshotTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract Counter {
    uint256 public count;

    function inc() external {
        count++;
    }

    function reset(uint8 x) external {
        if (x < 64) count = 0;
    }
}

contract SnapshotTest is Test {
    Counter counter;

    function setUp() public {
        counter = new Counter();
    }

    function invariant_shallow() public view {
        require(counter.count() < 20, "deep");
    }
}
"#,
    );

    cmd.args(["test", "--mt", "invariant_shallow"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: deep]
...
"#]]);
    // The persisted counterexample replays from setUp.
    cmd.assert_failure().stdout_eq(str![[r#"
...
[FAIL: deep]
...
"#]]);
}
