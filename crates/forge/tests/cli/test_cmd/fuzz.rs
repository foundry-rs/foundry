use alloy_primitives::U256;
use foundry_test_utils::{TestCommand, forgetest_init, str};
use regex::Regex;

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
    prj.update_config(|config| {
        config.fuzz.fail_on_revert = false;
        config.fuzz.seed = Some(U256::from(100u32));
    });
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
    // Tests should fail and record counterexample with value 200.
    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/Counter.t.sol:CounterTest
[FAIL: EvmError: Revert; counterexample: calldata=0x5c7f60d700000000000000000000000000000000000000000000000000000000000000c8 args=[200]] testFuzz_SetNumber(uint256) (runs: 6, [AVG_GAS])
...

"#]]);

    // Change test to assume counterexample 2 is discarded.
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterTest is Test {
    function testFuzz_SetNumber(uint256 x) public pure {
        vm.assume(x != 200);
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
    // Test should fail with replayed counterexample 200 (0 runs).
    cmd.forge_fuse().args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/Counter.t.sol:CounterTest
[FAIL: EvmError: Revert; counterexample: calldata=0x5c7f60d700000000000000000000000000000000000000000000000000000000000000c8 args=[200]] testFuzz_SetNumber(uint256) (runs: 0, [AVG_GAS])
...

"#]]);
});

forgetest_init!(fuzz_basic, |prj, cmd| {
    prj.add_test(
        "Fuzz.t.sol",
        r#"
import "forge-std/Test.sol";

contract FuzzTest is Test {
    constructor() {
        emit log("constructor");
    }

    function setUp() public {
        emit log("setUp");
    }

    function testShouldFailFuzz(uint8 x) public {
        emit log("testFailFuzz");
        require(x > 128, "should revert");
    }

    function testSuccessfulFuzz(uint128 a, uint128 b) public {
        emit log("testSuccessfulFuzz");
        assertEq(uint256(a) + uint256(b), uint256(a) + uint256(b));
    }

    function testToStringFuzz(bytes32 data) public {
        vm.toString(data);
    }
}
   "#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Ran 3 tests for test/Fuzz.t.sol:FuzzTest
[FAIL: should revert; counterexample: calldata=[..] args=[..]] testShouldFailFuzz(uint8) (runs: [..], [AVG_GAS])
[PASS] testSuccessfulFuzz(uint128,uint128) (runs: 256, [AVG_GAS])
[PASS] testToStringFuzz(bytes32) (runs: 256, [AVG_GAS])
Suite result: FAILED. 2 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 1 failed, 0 skipped (3 total tests)

Failing tests:
Encountered 1 failing test in test/Fuzz.t.sol:FuzzTest
[FAIL: should revert; counterexample: calldata=[..] args=[..]] testShouldFailFuzz(uint8) (runs: [..], [AVG_GAS])

Encountered a total of 1 failing tests, 2 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// Test that showcases PUSH collection on normal fuzzing.
// Ignored until we collect them in a smarter way.
forgetest_init!(
    #[ignore]
    fuzz_collection,
    |prj, cmd| {
        prj.update_config(|config| {
            config.invariant.depth = 100;
            config.invariant.runs = 1000;
            config.fuzz.runs = 1000;
            config.fuzz.seed = Some(U256::from(6u32));
        });
        prj.add_test(
            "FuzzCollection.t.sol",
            r#"
import "forge-std/Test.sol";

contract SampleContract {
    uint256 public counter;
    uint256 public counterX2;
    address public owner = address(0xBEEF);
    bool public found_needle;

    event Incremented(uint256 counter);

    modifier onlyOwner() {
        require(msg.sender == owner, "ONLY_OWNER");
        _;
    }

    function compare(uint256 val) public {
        if (val == 0x4446) {
            found_needle = true;
        }
    }

    function incrementBy(uint256 numToIncrement) public onlyOwner {
        counter += numToIncrement;
        counterX2 += numToIncrement * 2;

        emit Incremented(counter);
    }

    function breakTheInvariant(uint256 x) public {
        if (x == 0x5556) {
            counterX2 = 0;
        }
    }
}

contract SampleContractTest is Test {
    event Incremented(uint256 counter);

    SampleContract public sample;

    function setUp() public {
        sample = new SampleContract();
    }

    function testIncrement(address caller) public {
        vm.startPrank(address(caller));

        vm.expectRevert("ONLY_OWNER");
        sample.incrementBy(1);
    }

    function testNeedle(uint256 needle) public {
        sample.compare(needle);
        require(!sample.found_needle(), "needle found.");
    }

    function invariantCounter() public {
        require(sample.counter() * 2 == sample.counterX2(), "broken counter.");
    }
}
   "#,
        );

        cmd.args(["test"]).assert_failure().stdout_eq(str![[r#""#]]);
    }
);

forgetest_init!(fuzz_failure_persist, |prj, cmd| {
    let persist_dir = prj.cache().parent().unwrap().join("persist");
    assert!(!persist_dir.exists());
    prj.update_config(|config| {
        config.fuzz.failure_persist_dir = Some(persist_dir.clone());
    });

    prj.add_test(
        "FuzzFailurePersist.t.sol",
        r#"
import "forge-std/Test.sol";

struct TestTuple {
    address user;
    uint256 amount;
}

contract FuzzFailurePersistTest is Test {
    function test_persist_fuzzed_failure(
        uint256 x,
        int256 y,
        address addr,
        bool cond,
        string calldata test,
        TestTuple calldata tuple,
        address[] calldata addresses
    ) public {
        // dummy assume to trigger runs
        vm.assume(x > 1 && x < 1111111111111111111111111111);
        vm.assume(y > 1 && y < 1111111111111111111111111111);
        require(false);
    }
}
   "#,
    );

    let mut calldata = None;
    let expected = str![[r#"
...
Ran 1 test for test/FuzzFailurePersist.t.sol:FuzzFailurePersistTest
[FAIL: EvmError: Revert; counterexample: calldata=[..] args=[..]] test_persist_fuzzed_failure(uint256,int256,address,bool,string,(address,uint256),address[]) (runs: 0, [AVG_GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]];
    let mut check = |cmd: &mut TestCommand, same: bool| {
        let assert = cmd.assert_failure();
        let output = assert.get_output();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let calldata = calldata.get_or_insert_with(|| {
            let re = Regex::new(r"calldata=(0x[0-9a-fA-F]+)").unwrap();
            re.captures(&stdout).unwrap().get(1).unwrap().as_str().to_string()
        });
        assert_eq!(stdout.contains(calldata.as_str()), same, "\n{stdout}");
        assert.stdout_eq(expected.clone());
    };

    cmd.arg("test");

    // Run several times, asserting that the failure persists and is the same.
    for _ in 0..3 {
        check(&mut cmd, true);
        assert!(persist_dir.exists());
    }

    // Change dir and run again, asserting that the failure persists. It should be a new failure.
    let new_persist_dir = prj.cache().parent().unwrap().join("persist2");
    assert!(!new_persist_dir.exists());
    prj.update_config(|config| {
        config.fuzz.failure_persist_dir = Some(new_persist_dir.clone());
    });
    check(&mut cmd, false);
    assert!(new_persist_dir.exists());
});

// https://github.com/foundry-rs/foundry/pull/735 behavior changed with https://github.com/foundry-rs/foundry/issues/3521
// random values (instead edge cases) are generated if no fixtures defined
forgetest_init!(fuzz_int, |prj, cmd| {
    prj.add_test(
        "FuzzInt.t.sol",
        r#"
import "forge-std/Test.sol";

contract FuzzNumbersTest is Test {
    function testPositive(int256) public {
        assertTrue(true);
    }

    function testNegativeHalf(int256 val) public {
        assertTrue(val < 2 ** 128 - 1);
    }

    function testNegative0(int256 val) public {
        assertTrue(val == 0);
    }

    function testNegative1(int256 val) public {
        assertTrue(val == -1);
    }

    function testNegative2(int128 val) public {
        assertTrue(val == 1);
    }

    function testNegativeMax0(int256 val) public {
        assertTrue(val == type(int256).max);
    }

    function testNegativeMax1(int256 val) public {
        assertTrue(val == type(int256).max - 2);
    }

    function testNegativeMin0(int256 val) public {
        assertTrue(val == type(int256).min);
    }

    function testNegativeMin1(int256 val) public {
        assertTrue(val == type(int256).min + 2);
    }

    function testEquality(int256 x, int256 y) public {
        int256 xy;

        unchecked {
            xy = x * y;
        }

        if ((x != 0 && xy / x != y)) {
            return;
        }

        assertEq(((xy - 1) / 1e18) + 1, (xy - 1) / (1e18 + 1));
    }
}
   "#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Ran 10 tests for test/FuzzInt.t.sol:FuzzNumbersTest
[FAIL: assertion failed[..]] testEquality(int256,int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative0(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative1(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative2(int128) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeHalf(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeMax0(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeMax1(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeMin0(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeMin1(int256) (runs: [..], [AVG_GAS])
[PASS] testPositive(int256) (runs: 256, [AVG_GAS])
Suite result: FAILED. 1 passed; 9 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 9 failed, 0 skipped (10 total tests)
...
"#]]);
});

forgetest_init!(fuzz_positive, |prj, cmd| {
    prj.add_test(
        "FuzzPositive.t.sol",
        r#"
import "forge-std/Test.sol";

contract FuzzPositive is Test {
    function testSuccessChecker(uint256 val) public {
        assertTrue(true);
    }

    function testSuccessChecker2(int256 val) public {
        assert(val == val);
    }

    function testSuccessChecker3(uint32 val) public {
        assert(val + 0 == val);
    }
}
   "#,
    );

    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
...
Ran 3 tests for test/FuzzPositive.t.sol:FuzzPositive
[PASS] testSuccessChecker(uint256) (runs: 256, [AVG_GAS])
[PASS] testSuccessChecker2(int256) (runs: 256, [AVG_GAS])
[PASS] testSuccessChecker3(uint32) (runs: 256, [AVG_GAS])
Suite result: ok. 3 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 3 tests passed, 0 failed, 0 skipped (3 total tests)

"#]]);
});

// https://github.com/foundry-rs/foundry/pull/735 behavior changed with https://github.com/foundry-rs/foundry/issues/3521
// random values (instead edge cases) are generated if no fixtures defined
forgetest_init!(fuzz_uint, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(100u32));
    });
    prj.add_test(
        "FuzzUint.t.sol",
        r#"
import "forge-std/Test.sol";

contract FuzzNumbersTest is Test {
    function testPositive(uint256) public {
        assertTrue(true);
    }

    function testNegativeHalf(uint256 val) public {
        assertTrue(val < 2 ** 128 - 1);
    }

    function testNegative0(uint256 val) public {
        assertTrue(val == 0);
    }

    function testNegative2(uint256 val) public {
        assertTrue(val == 2);
    }

    function testNegative2Max(uint256 val) public {
        assertTrue(val == type(uint256).max - 2);
    }

    function testNegativeMax(uint256 val) public {
        assertTrue(val == type(uint256).max);
    }

    function testEquality(uint256 x, uint256 y) public {
        uint256 xy;

        unchecked {
            xy = x * y;
        }

        if ((x != 0 && xy / x != y)) {
            return;
        }

        assertEq(((xy - 1) / 1e18) + 1, (xy - 1) / (1e18 + 1));
    }
}
   "#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Ran 7 tests for test/FuzzUint.t.sol:FuzzNumbersTest
[FAIL: assertion failed[..]] testEquality(uint256,uint256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative0(uint256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative2(uint256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative2Max(uint256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeHalf(uint256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeMax(uint256) (runs: [..], [AVG_GAS])
[PASS] testPositive(uint256) (runs: 256, [AVG_GAS])
Suite result: FAILED. 1 passed; 6 failed; 0 skipped; [ELAPSED]
...
"#]]);
});

forgetest_init!(should_fuzz_literals, |prj, cmd| {
    // Add a source with magic (literal) values
    prj.add_source(
        "Magic.sol",
        r#"
        contract Magic {
            // plain literals
            address constant DAI = 0x6B175474E89094C44Da98b954EedeAC495271d0F;
            uint64 constant MAGIC_NUMBER = 1122334455;
            int32 constant MAGIC_INT = -777;
            bytes32 constant MAGIC_WORD = "abcd1234";
            bytes constant MAGIC_BYTES = hex"deadbeef";
            string constant MAGIC_STRING = "xyzzy";

            function checkAddr(address v) external pure { assert(v != DAI); }
            function checkWord(bytes32 v) external pure { assert(v != MAGIC_WORD); }
            function checkNumber(uint64 v) external pure { assert(v != MAGIC_NUMBER); }
            function checkInteger(int32 v) external pure { assert(v != MAGIC_INT); }
            function checkString(string memory v) external pure { assert(keccak256(abi.encodePacked(v)) != keccak256(abi.encodePacked(MAGIC_STRING))); }
            function checkBytesFromHex(bytes memory v) external pure { assert(keccak256(v) != keccak256(MAGIC_BYTES)); }
            function checkBytesFromString(bytes memory v) external pure { assert(keccak256(v) != keccak256(abi.encodePacked(MAGIC_STRING))); }
        }
        "#,
    );

    prj.add_test(
        "MagicFuzz.t.sol",
        r#"
            import {Test} from "forge-std/Test.sol";
            import {Magic} from "src/Magic.sol";

            contract MagicTest is Test {
                Magic public magic;
                function setUp() public { magic = new Magic(); }

                function testFuzz_Addr(address v) public view { magic.checkAddr(v); }
                function testFuzz_Number(uint64 v) public view { magic.checkNumber(v); }
                function testFuzz_Integer(int32 v) public view { magic.checkInteger(v); }
                function testFuzz_Word(bytes32 v) public view { magic.checkWord(v); }
                function testFuzz_String(string memory v) public view { magic.checkString(v); }
                function testFuzz_BytesFromHex(bytes memory v) public view { magic.checkBytesFromHex(v); }
                function testFuzz_BytesFromString(bytes memory v) public view { magic.checkBytesFromString(v); }
            }
        "#,
    );

    // Helper to create expected output for a test failure
    let expected_fail = |test_name: &str, type_sig: &str, value: &str, runs: u32| -> String {
        format!(
            r#"No files changed, compilation skipped

Ran 1 test for test/MagicFuzz.t.sol:MagicTest
[FAIL: panic: assertion failed (0x01); counterexample: calldata=[..] args=[{value}]] {test_name}({type_sig}) (runs: {runs}, [AVG_GAS])
[..]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
...
Encountered a total of 1 failing tests, 0 tests succeeded
...
"#
        )
    };

    // Test address literal fuzzing
    let mut test_literal = |seed: u32,
                            test_name: &'static str,
                            type_sig: &'static str,
                            expected_value: &'static str,
                            expected_runs: u32| {
        // the fuzzer is UNABLE to find a breaking input (fast) when NOT seeding from the AST
        prj.update_config(|config| {
            config.fuzz.runs = 100;
            config.fuzz.dictionary.max_fuzz_dictionary_literals = 0;
            config.fuzz.seed = Some(U256::from(seed));
        });
        cmd.forge_fuse().args(["test", "--match-test", test_name]).assert_success();

        // the fuzzer is ABLE to find a breaking input when seeding from the AST
        prj.update_config(|config| {
            config.fuzz.dictionary.max_fuzz_dictionary_literals = 10_000;
        });

        let expected_output = expected_fail(test_name, type_sig, expected_value, expected_runs);
        cmd.forge_fuse()
            .args(["test", "--match-test", test_name])
            .assert_failure()
            .stdout_eq(expected_output);
    };

    test_literal(100, "testFuzz_Addr", "address", "0x6B175474E89094C44Da98b954EedeAC495271d0F", 28);
    test_literal(200, "testFuzz_Number", "uint64", "1122334455 [1.122e9]", 5);
    test_literal(300, "testFuzz_Integer", "int32", "-777", 0);
    test_literal(
        400,
        "testFuzz_Word",
        "bytes32",
        "0x6162636431323334000000000000000000000000000000000000000000000000", /* bytes32("abcd1234") */
        7,
    );
    test_literal(500, "testFuzz_BytesFromHex", "bytes", "0xdeadbeef", 5);
    test_literal(600, "testFuzz_String", "string", "\"xyzzy\"", 35);
    test_literal(999, "testFuzz_BytesFromString", "bytes", "0x78797a7a79", 19); // abi.encodePacked("xyzzy")
});

// Tests that `vm.randomUint()` produces different values across fuzz runs.
// Regression test for https://github.com/foundry-rs/foundry/issues/12817
//
// The issue was that `vm.randomUint()` would produce the same sequence of values
// in every fuzz run because the RNG was seeded identically for each run.
// This test verifies that with many fuzz runs and a small range, we eventually
// hit value 0, which proves the RNG varies across runs.
forgetest_init!(test_fuzz_random_uint_varies_across_runs, |prj, cmd| {
    prj.add_test(
        "RandomFuzzTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract RandomFuzzTest is Test {
    function testFuzz_randomUint_shouldFail(uint256) public {
        uint256 rand = vm.randomUint(0, 4);
        assertTrue(rand != 0, "hit value 0");
    }
}
   "#,
    );

    cmd.args(["test", "--fuzz-seed", "1", "--mt", "testFuzz_randomUint_shouldFail"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/RandomFuzzTest.t.sol:RandomFuzzTest
[FAIL: hit value 0; counterexample: [..]] testFuzz_randomUint_shouldFail(uint256) (runs: [..], [AVG_GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)
...
"#]]);
});
