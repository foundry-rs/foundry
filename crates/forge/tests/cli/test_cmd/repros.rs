//! Regression tests for specific GitHub issues

use foundry_test_utils::str;

forgetest_init!(issue_3055, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "Issue3055.t.sol",
        r#"
import "forge-std/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3055
/// forge-config: default.assertions_revert = false
contract Issue3055Test is Test {
    function test_snapshot() external {
        uint256 snapshotId = vm.snapshotState();
        assertEq(uint256(0), uint256(1));
        vm.revertToState(snapshotId);
    }

    function test_snapshot2() public {
        uint256 snapshotId = vm.snapshotState();
        assertTrue(false);
        vm.revertToState(snapshotId);
        assertTrue(true);
    }

    function test_snapshot3(uint256) public {
        vm.expectRevert();
        // Call exposed_snapshot3() using this to perform an external call,
        // so we can properly test for reverts.
        this.exposed_snapshot3();
    }

    function exposed_snapshot3() public {
        uint256 snapshotId = vm.snapshotState();
        assertTrue(false);
        vm.revertToState(snapshotId);
    }
}
"#,
    );

    cmd.arg("test").assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 3 tests for test/Issue3055.t.sol:Issue3055Test
[FAIL] test_snapshot() ([GAS])
[FAIL] test_snapshot2() ([GAS])
[FAIL: next call did not revert as expected; counterexample: calldata=[..] args=[..] test_snapshot3(uint256) (runs: 0, [AVG_GAS])
Suite result: FAILED. 0 passed; 3 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 3 failed, 0 skipped (3 total tests)

Failing tests:
Encountered 3 failing tests in test/Issue3055.t.sol:Issue3055Test
[FAIL] test_snapshot() ([GAS])
[FAIL] test_snapshot2() ([GAS])
[FAIL: next call did not revert as expected; counterexample: calldata=[..] args=[..] test_snapshot3(uint256) (runs: 0, [AVG_GAS])

Encountered a total of 3 failing tests, 0 tests succeeded

"#]]);
});

forgetest_init!(issue_3189, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "Issue3189.t.sol",
        r#"
import "forge-std/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3189
contract MyContract {
    function foo(uint256 arg) public returns (uint256) {
        return arg + 2;
    }
}

contract MyContractUser is Test {
    MyContract immutable myContract;

    constructor() {
        myContract = new MyContract();
    }

    function foo(uint256 arg) public returns (uint256 ret) {
        ret = myContract.foo(arg);
        assertEq(ret, arg + 1, "Invariant failed");
    }
}

contract Issue3189Test is Test {
    function testFoo() public {
        MyContractUser user = new MyContractUser();
        user.foo(123);
    }
}
"#,
    );

    cmd.arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/Issue3189.t.sol:Issue3189Test
[FAIL: Invariant failed: 125 != 124] testFoo() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Issue3189.t.sol:Issue3189Test
[FAIL: Invariant failed: 125 != 124] testFoo() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

forgetest_init!(issue_3596, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "Issue3596.t.sol",
        r#"
import "forge-std/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3596
contract Issue3596Test is Test {
    function testDealTransfer() public {
        address addr = vm.addr(1337);
        vm.startPrank(addr);
        vm.deal(addr, 20000001 ether);
        payable(address(this)).transfer(20000000 ether);

        Nested nested = new Nested();
        nested.doStuff();
        vm.stopPrank();
    }
}

contract Nested {
    function doStuff() public {
        doRevert();
    }

    function doRevert() public {
        revert("This fails");
    }
}
"#,
    );

    cmd.arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/Issue3596.t.sol:Issue3596Test
[FAIL: EvmError: Revert] testDealTransfer() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Issue3596.t.sol:Issue3596Test
[FAIL: EvmError: Revert] testDealTransfer() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

forgetest_init!(issue_2851, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "Issue2851.t.sol",
        r#"
import "forge-std/Test.sol";

contract Backdoor {
    uint256 public number = 1;

    function backdoor(uint256 newNumber) public payable {
        uint256 x = newNumber - 1;
        if (x == 6912213124124531) {
            number = 0;
        }
    }
}

// https://github.com/foundry-rs/foundry/issues/2851
contract Issue2851Test is Test {
    Backdoor back;

    function setUp() public {
        back = new Backdoor();
    }

    /// forge-config: default.fuzz.seed = "111"
    function invariantNotZero() public {
        assertEq(back.number(), 1);
    }
}
"#,
    );

    cmd.arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/Issue2851.t.sol:Issue2851Test
[FAIL: assertion failed: 0 != 1]
	[Sequence] (original: 148, shrunk: 1)
		sender=0x0000000000000000000000000000000000000561 addr=[test/Issue2851.t.sol:Backdoor]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=backdoor(uint256) args=[6912213124124532 [6.912e15]]
 invariantNotZero() (runs: 0, calls: 0, reverts: 1)

╭----------+----------+-------+---------+----------╮
| Contract | Selector | Calls | Reverts | Discards |
+==================================================+
| Backdoor | backdoor | 149   | 1       | 0        |
╰----------+----------+-------+---------+----------╯

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Issue2851.t.sol:Issue2851Test
[FAIL: assertion failed: 0 != 1]
	[Sequence] (original: 148, shrunk: 1)
		sender=0x0000000000000000000000000000000000000561 addr=[test/Issue2851.t.sol:Backdoor]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=backdoor(uint256) args=[6912213124124532 [6.912e15]]
 invariantNotZero() (runs: 0, calls: 0, reverts: 1)

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

forgetest_init!(issue_6170, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "Issue6170.t.sol",
        r#"
import "forge-std/Test.sol";

contract Emitter {
    event Values(uint256 indexed a, uint256 indexed b);

    function plsEmit(uint256 a, uint256 b) external {
        emit Values(a, b);
    }
}

// https://github.com/foundry-rs/foundry/issues/6170
contract Issue6170Test is Test {
    event Values(uint256 indexed a, uint256 b);

    Emitter e = new Emitter();

    function test() public {
        vm.expectEmit(true, true, false, true);
        emit Values(69, 420);
        e.plsEmit(69, 420);
    }
}
"#,
    );

    cmd.arg("test").assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Issue6170.t.sol:Issue6170Test
[FAIL: log != expected log] test() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Issue6170.t.sol:Issue6170Test
[FAIL: log != expected log] test() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

"#]]);
});

forgetest_init!(issue_6355, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "Issue6355.t.sol",
        r#"
import "forge-std/Test.sol";

// https://github.com/foundry-rs/foundry/issues/6355
contract Issue6355Test is Test {
    uint256 snapshotId;
    Target targ;

    function setUp() public {
        snapshotId = vm.snapshotState();
        targ = new Target();
    }

    // this non-deterministically fails sometimes and passes sometimes
    function test_shouldPass() public {
        assertEq(2, targ.num());
    }

    // always fails
    function test_shouldFailWithRevertToState() public {
        assertEq(3, targ.num());
        vm.revertToState(snapshotId);
    }

    // always fails
    function test_shouldFail() public {
        assertEq(3, targ.num());
    }
}

contract Target {
    function num() public pure returns (uint256) {
        return 2;
    }
}
"#,
    );

    cmd.arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 3 tests for test/Issue6355.t.sol:Issue6355Test
[FAIL: assertion failed: 3 != 2] test_shouldFail() ([GAS])
[FAIL: assertion failed: 3 != 2] test_shouldFailWithRevertToState() ([GAS])
[PASS] test_shouldPass() ([GAS])
Suite result: FAILED. 1 passed; 2 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 2 failed, 0 skipped (3 total tests)

Failing tests:
Encountered 2 failing tests in test/Issue6355.t.sol:Issue6355Test
[FAIL: assertion failed: 3 != 2] test_shouldFail() ([GAS])
[FAIL: assertion failed: 3 != 2] test_shouldFailWithRevertToState() ([GAS])

Encountered a total of 2 failing tests, 1 tests succeeded

"#]]);
});
