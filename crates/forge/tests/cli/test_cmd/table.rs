//! Table tests.

use foundry_test_utils::{forgetest_init, str};

forgetest_init!(should_run_table_tests, |prj, cmd| {
    prj.initialize_default_contracts();
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
    );

    cmd.args(["test", "--mc", "CounterTable", "-vvvvv"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 8 tests for test/CounterTable.t.sol:CounterTableTest
[FAIL: 2 fixtures defined for diffSwap (expected 10)] tableMultipleParamsDifferentFixturesFail(uint256,bool) ([GAS])
[FAIL: Cannot swap; counterexample: calldata=0x717892ca00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000001 args=[1, true]] tableMultipleParamsFail(uint256,bool) (runs: 1, [AVG_GAS])
Traces:
  [..] CounterTableTest::tableMultipleParamsFail(1, true)
    └─ ← [Revert] Cannot swap

Backtrace:
  at CounterTableTest.tableMultipleParamsFail (test/CounterTable.t.sol:[..]:[..])

[FAIL: No fixture defined for param noSwap] tableMultipleParamsNoParamFail(uint256,bool) ([GAS])
[PASS] tableMultipleParamsPass(uint256,bool) (runs: 10, [AVG_GAS])
Traces:
  [..] CounterTableTest::tableMultipleParamsPass(10, true)
    ├─ [..] Counter::increment()
    │   ├─  storage changes:
    │   │   @ 0: 0 → 1
    │   └─ ← [Stop]
    └─ ← [Stop]

[FAIL: Amount cannot be 10; counterexample: calldata=0x44fa2375000000000000000000000000000000000000000000000000000000000000000a args=[10]] tableSingleParamFail(uint256) (runs: 10, [AVG_GAS])
Traces:
  [..] CounterTableTest::tableSingleParamFail(10)
    └─ ← [Revert] Amount cannot be 10

Backtrace:
  at CounterTableTest.tableSingleParamFail (test/CounterTable.t.sol:[..]:[..])

[PASS] tableSingleParamPass(uint256) (runs: 10, [AVG_GAS])
Traces:
  [..] CounterTableTest::tableSingleParamPass(10)
    ├─ [..] Counter::increment()
    │   ├─  storage changes:
    │   │   @ 0: 0 → 1
    │   └─ ← [Stop]
    └─ ← [Stop]

[FAIL: Table test should have at least one parameter] tableWithNoParamFail() ([GAS])
[FAIL: Table test should have at least one fixture] tableWithParamNoFixtureFail(uint256) ([GAS])
Suite result: FAILED. 2 passed; 6 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 6 failed, 0 skipped (8 total tests)

Failing tests:
Encountered 6 failing tests in test/CounterTable.t.sol:CounterTableTest
[FAIL: 2 fixtures defined for diffSwap (expected 10)] tableMultipleParamsDifferentFixturesFail(uint256,bool) ([GAS])
[FAIL: Cannot swap; counterexample: calldata=0x717892ca00000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000001 args=[1, true]] tableMultipleParamsFail(uint256,bool) (runs: 1, [AVG_GAS])
[FAIL: No fixture defined for param noSwap] tableMultipleParamsNoParamFail(uint256,bool) ([GAS])
[FAIL: Amount cannot be 10; counterexample: calldata=0x44fa2375000000000000000000000000000000000000000000000000000000000000000a args=[10]] tableSingleParamFail(uint256) (runs: 10, [AVG_GAS])
[FAIL: Table test should have at least one parameter] tableWithNoParamFail() ([GAS])
[FAIL: Table test should have at least one fixture] tableWithParamNoFixtureFail(uint256) ([GAS])

Encountered a total of 6 failing tests, 2 tests succeeded

Tip: Run `forge test --rerun` to retry only the 6 failed tests

"#]]);
});

// Table tests should show logs and contribute to coverage.
// <https://github.com/foundry-rs/foundry/issues/11066>
forgetest_init!(should_show_logs_and_add_coverage, |prj, cmd| {
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 a, uint256 b) public {
        if (a == 1) {
            number = b + 1;
        } else if (a == 2) {
            number = b + 2;
        } else if (a == 3) {
            number = b + 3;
        } else {
            number = a + b;
        }
    }
}
    "#,
    );
    prj.add_test(
        "CounterTest.t.sol",
        r#"
import "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    struct TestCase {
        uint256 a;
        uint256 b;
        uint256 expected;
    }

    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function fixtureNumbers() public pure returns (TestCase[] memory) {
        TestCase[] memory entries = new TestCase[](4);
        entries[0] = TestCase(1, 5, 6);
        entries[1] = TestCase(2, 10, 12);
        entries[2] = TestCase(3, 11, 14);
        entries[3] = TestCase(4, 11, 15);
        return entries;
    }

    function tableSetNumberTest(TestCase memory numbers) public {
        console.log("expected", numbers.expected);
        counter.setNumber(numbers.a, numbers.b);
        require(counter.number() == numbers.expected, "test failed");
    }
}
    "#,
    );

    cmd.args(["test", "-vvv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/CounterTest.t.sol:CounterTest
[PASS] tableSetNumberTest((uint256,uint256,uint256)) (runs: 4, [AVG_GAS])
Logs:
  expected 6
  expected 12
  expected 14
  expected 15

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    cmd.forge_fuse().args(["coverage"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
Analysing contracts...
Running tests...

Ran 1 test for test/CounterTest.t.sol:CounterTest
[PASS] tableSetNumberTest((uint256,uint256,uint256)) (runs: 4, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

╭-----------------+---------------+---------------+---------------+---------------╮
| File            | % Lines       | % Statements  | % Branches    | % Funcs       |
+=================================================================================+
| src/Counter.sol | 100.00% (8/8) | 100.00% (7/7) | 100.00% (6/6) | 100.00% (1/1) |
|-----------------+---------------+---------------+---------------+---------------|
| Total           | 100.00% (8/8) | 100.00% (7/7) | 100.00% (6/6) | 100.00% (1/1) |
╰-----------------+---------------+---------------+---------------+---------------╯

"#]]);
});
