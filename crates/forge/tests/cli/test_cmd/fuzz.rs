use alloy_primitives::U256;
use foundry_test_utils::{forgetest_init, str};

forgetest_init!(test_can_scrape_bytecode, |prj, cmd| {
    prj.update_config(|config| config.optimizer = Some(true));
    prj.add_source(
        "FuzzerDict.sol",
        r#"
// https://github.com/foundry-rs/foundry/issues/1168
contract FuzzerDict {
    // Immutables should get added to the dictionary.
    address public immutable immutableOwner;
    // Regular storage variables should also get added to the dictionary.
    address public storageOwner;

    constructor(address _immutableOwner, address _storageOwner) {
        immutableOwner = _immutableOwner;
        storageOwner = _storageOwner;
    }
}
   "#,
    );

    prj.add_test(
        "FuzzerDictTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import "src/FuzzerDict.sol";

contract FuzzerDictTest is Test {
    FuzzerDict fuzzerDict;

    function setUp() public {
        fuzzerDict = new FuzzerDict(address(100), address(200));
    }

    /// forge-config: default.fuzz.runs = 2000
    function testImmutableOwner(address who) public {
        assertTrue(who != fuzzerDict.immutableOwner());
    }

    /// forge-config: default.fuzz.runs = 2000
    function testStorageOwner(address who) public {
        assertTrue(who != fuzzerDict.storageOwner());
    }
}
   "#,
    );

    // Test that immutable address is used as fuzzed input, causing test to fail.
    cmd.args(["test", "--fuzz-seed", "119", "--mt", "testImmutableOwner"]).assert_failure();
    // Test that storage address is used as fuzzed input, causing test to fail.
    cmd.forge_fuse()
        .args(["test", "--fuzz-seed", "119", "--mt", "testStorageOwner"])
        .assert_failure();
});

// tests that inline max-test-rejects config is properly applied
forgetest_init!(test_inline_max_test_rejects, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "Contract.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract InlineMaxRejectsTest is Test {
    /// forge-config: default.fuzz.max-test-rejects = 1
    function test_fuzz_bound(uint256 a) public {
        vm.assume(false);
    }
}
   "#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: `vm.assume` rejected too many inputs (1 allowed)] test_fuzz_bound(uint256) (runs: 0, [AVG_GAS])
...
"#]]);
});

// Tests that test timeout config is properly applied.
// If test doesn't timeout after one second, then test will fail with `rejected too many inputs`.
forgetest_init!(test_fuzz_timeout, |prj, cmd| {
    prj.wipe_contracts();

    prj.add_test(
        "Contract.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract FuzzTimeoutTest is Test {
    /// forge-config: default.fuzz.max-test-rejects = 50000
    /// forge-config: default.fuzz.timeout = 1
    function test_fuzz_bound(uint256 a) public pure {
        vm.assume(a == 0);
    }
}
   "#,
    );

    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Contract.t.sol:FuzzTimeoutTest
[PASS] test_fuzz_bound(uint256) (runs: [..], [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest_init!(test_fuzz_fail_on_revert, |prj, cmd| {
    prj.wipe_contracts();
    prj.update_config(|config| config.fuzz.fail_on_revert = false);
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        require(number > 10000000000, "low number");
        number = newNumber;
    }
}
   "#,
    );

    prj.add_test(
        "CounterTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import "src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function testFuzz_SetNumberRequire(uint256 x) public {
        counter.setNumber(x);
        require(counter.number() == 1);
    }

    function testFuzz_SetNumberAssert(uint256 x) public {
        counter.setNumber(x);
        assertEq(counter.number(), 1);
    }
}
   "#,
    );

    // Tests should not fail as revert happens in Counter contract.
    cmd.args(["test", "--mc", "CounterTest"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/CounterTest.t.sol:CounterTest
[PASS] testFuzz_SetNumberAssert(uint256) (runs: 256, [AVG_GAS])
[PASS] testFuzz_SetNumberRequire(uint256) (runs: 256, [AVG_GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);

    // Tested contract does not revert.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }
}
   "#,
    );

    // Tests should fail as revert happens in cheatcode (assert) and test (require) contract.
    cmd.assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/CounterTest.t.sol:CounterTest
[FAIL: assertion failed: [..]] testFuzz_SetNumberAssert(uint256) (runs: 0, [AVG_GAS])
[FAIL: EvmError: Revert; [..]] testFuzz_SetNumberRequire(uint256) (runs: 0, [AVG_GAS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]
...

"#]]);
});

// Test 256 runs regardless number of test rejects.
// <https://github.com/foundry-rs/foundry/issues/9054>
forgetest_init!(test_fuzz_runs_with_rejects, |prj, cmd| {
    prj.add_test(
        "FuzzWithRejectsTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract FuzzWithRejectsTest is Test {
    function testFuzzWithRejects(uint256 x) public pure {
        vm.assume(x < 1_000_000);
    }
}
   "#,
    );

    // Tests should not fail as revert happens in Counter contract.
    cmd.args(["test", "--mc", "FuzzWithRejectsTest"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/FuzzWithRejectsTest.t.sol:FuzzWithRejectsTest
[PASS] testFuzzWithRejects(uint256) (runs: 256, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// Test that counterexample is not replayed if test changes.
// <https://github.com/foundry-rs/foundry/issues/11927>
forgetest_init!(test_fuzz_replay_with_changed_test, |prj, cmd| {
    prj.update_config(|config| config.fuzz.seed = Some(U256::from(100u32)));
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterTest is Test {
    function testFuzz_SetNumber(uint256 x) public pure {
        require(x > 200);
    }
}
   "#,
    );
    // Tests should fail and record counterexample with value 2.
    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/Counter.t.sol:CounterTest
[FAIL: EvmError: Revert; counterexample: calldata=0x5c7f60d70000000000000000000000000000000000000000000000000000000000000002 args=[2]] testFuzz_SetNumber(uint256) (runs: 19, [AVG_GAS])
...

"#]]);

    // Change test to assume counterexample 2 is discarded.
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterTest is Test {
    function testFuzz_SetNumber(uint256 x) public pure {
        vm.assume(x != 2);
    }
}
   "#,
    );
    // Test should pass when replay failure with changed assume logic.
    cmd.forge_fuse().args(["test"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint256) (runs: 256, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Change test signature.
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterTest is Test {
    function testFuzz_SetNumber(uint8 x) public pure {
    }
}
   "#,
    );
    // Test should pass when replay failure with changed function signature.
    cmd.forge_fuse().args(["test"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint8) (runs: 256, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Change test back to the original one that produced the counterexample.
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterTest is Test {
    function testFuzz_SetNumber(uint256 x) public pure {
        require(x > 200);
    }
}
   "#,
    );
    // Test should fail with replayed counterexample 2 (0 runs).
    cmd.forge_fuse().args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/Counter.t.sol:CounterTest
[FAIL: EvmError: Revert; counterexample: calldata=0x5c7f60d70000000000000000000000000000000000000000000000000000000000000002 args=[2]] testFuzz_SetNumber(uint256) (runs: 0, [AVG_GAS])
...

"#]]);
});
