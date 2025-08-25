//! Table tests.

use foundry_test_utils::{forgetest_init, str};

forgetest_init!(should_run_table_tests, |prj, cmd| {
    prj.add_test(
        "CounterTable.t.sol",
        r#"
import "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTableTest is Test {
    Counter counter = new Counter();

    uint256[] public fixtureAmount = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    bool[] public fixtureSwap = [true, true, false, true, false, true, false, true, false, true];
    bool[] public fixtureDiffSwap = [true, false];
    function fixtureNoFixture() public returns (address[] memory) {
    }

    function tableWithNoParamFail() public {
        counter.increment();
    }

    function tableWithParamNoFixtureFail(uint256 noFixture) public {
        require(noFixture != 100);
        counter.increment();
    }

    function tableSingleParamPass(uint256 amount) public {
        require(amount != 100, "Amount cannot be 100");
        counter.increment();
    }

    function tableSingleParamFail(uint256 amount) public {
        require(amount != 10, "Amount cannot be 10");
        counter.increment();
    }

    function tableMultipleParamsNoParamFail(uint256 amount, bool noSwap) public {
        require(amount != 100 && noSwap, "Amount cannot be 100");
        counter.increment();
    }

    function tableMultipleParamsDifferentFixturesFail(uint256 amount, bool diffSwap) public {
        require(amount != 100 && diffSwap, "Amount cannot be 100");
        counter.increment();
    }

    function tableMultipleParamsFail(uint256 amount, bool swap) public {
        require(amount == 3 && swap, "Cannot swap");
        counter.increment();
    }

    function tableMultipleParamsPass(uint256 amount, bool swap) public {
        if (amount == 3 && swap) {
            revert();
        }
        counter.increment();
    }
}
    "#,
    )
    .unwrap();

    cmd.args(["test", "--mc", "CounterTable", "-vvvv"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 8 tests for test/CounterTable.t.sol:CounterTableTest
[FAIL: 2 fixtures defined for diffSwap (expected 10)] tableMultipleParamsDifferentFixturesFail(uint256,bool) ([GAS])
[FAIL: Cannot swap; counterexample: calldata=0x717892ca00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000001 args=[1, true]] tableMultipleParamsFail(uint256,bool) ([GAS])
Traces:
  [..] CounterTableTest::tableMultipleParamsFail(1, true)
    └─ ← [Revert] Cannot swap

[FAIL: No fixture defined for param noSwap] tableMultipleParamsNoParamFail(uint256,bool) ([GAS])
[PASS] tableMultipleParamsPass(uint256,bool) ([GAS])
Traces:
  [..] CounterTableTest::tableMultipleParamsPass(10, true)
    ├─ [..] Counter::increment()
    │   └─ ← [Stop]
    └─ ← [Stop]

[FAIL: Amount cannot be 10; counterexample: calldata=0x44fa2375000000000000000000000000000000000000000000000000000000000000000a args=[10]] tableSingleParamFail(uint256) ([GAS])
Traces:
  [..] CounterTableTest::tableSingleParamFail(10)
    └─ ← [Revert] Amount cannot be 10

[PASS] tableSingleParamPass(uint256) ([GAS])
Traces:
  [..] CounterTableTest::tableSingleParamPass(10)
    ├─ [..] Counter::increment()
    │   └─ ← [Stop]
    └─ ← [Stop]

[FAIL: Table test should have at least one parameter] tableWithNoParamFail() ([GAS])
[FAIL: Table test should have at least one fixture] tableWithParamNoFixtureFail(uint256) ([GAS])
Suite result: FAILED. 2 passed; 6 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 6 failed, 0 skipped (8 total tests)

Failing tests:
Encountered 6 failing tests in test/CounterTable.t.sol:CounterTableTest
[FAIL: 2 fixtures defined for diffSwap (expected 10)] tableMultipleParamsDifferentFixturesFail(uint256,bool) ([GAS])
[FAIL: Cannot swap; counterexample: calldata=0x717892ca00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000001 args=[1, true]] tableMultipleParamsFail(uint256,bool) ([GAS])
[FAIL: No fixture defined for param noSwap] tableMultipleParamsNoParamFail(uint256,bool) ([GAS])
[FAIL: Amount cannot be 10; counterexample: calldata=0x44fa2375000000000000000000000000000000000000000000000000000000000000000a args=[10]] tableSingleParamFail(uint256) ([GAS])
[FAIL: Table test should have at least one parameter] tableWithNoParamFail() ([GAS])
[FAIL: Table test should have at least one fixture] tableWithParamNoFixtureFail(uint256) ([GAS])

Encountered a total of 6 failing tests, 2 tests succeeded

"#]]);
});
