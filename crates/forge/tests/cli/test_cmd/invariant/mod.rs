use alloy_primitives::U256;
use foundry_test_utils::{TestCommand, forgetest_init, snapbox::cmd::OutputAssert, str};

mod common;
mod handler;
mod storage;
mod target;

fn assert_invariant(cmd: &mut TestCommand) -> OutputAssert {
    cmd.assert_with(&[
        ("[RUNS]", r"runs: \d+, calls: \d+, reverts: \d+"),
        ("[SEQUENCE]", r"\[Sequence\].*(\n\t\t.*)*"),
        ("[STATS]", r"╭[\s\S]*?╰.*"),
    ])
}

// Tests that a persisted failure doesn't fail due to assume revert if test driver is changed.
forgetest_init!(should_not_fail_replay_assume, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
        config.invariant.max_assume_rejects = 10;
    });

    // Add initial test that breaks invariant.
    prj.add_test(
        "AssumeTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract AssumeHandler is Test {
    function fuzzMe(uint256 a) public {
        require(false, "Invariant failure");
    }
}

contract AssumeTest is Test {
    function setUp() public {
        AssumeHandler handler = new AssumeHandler();
    }
    function invariant_assume() public {}
}
     "#,
    );

    cmd.args(["test", "--mt", "invariant_assume"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: Invariant failure]
...
"#]]);

    // Change test to use assume instead require. Same test should fail with too many inputs
    // rejected message instead persisted failure revert.
    prj.add_test(
        "AssumeTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract AssumeHandler is Test {
    function fuzzMe(uint256 a) public {
        vm.assume(false);
    }
}

contract AssumeTest is Test {
    function setUp() public {
        AssumeHandler handler = new AssumeHandler();
    }
    function invariant_assume() public {}
}
     "#,
    );

    cmd.assert_failure().stdout_eq(str![[r#"
...
[FAIL: `vm.assume` rejected too many inputs (10 allowed)] invariant_assume() (runs: 0, calls: 0, reverts: 0)
...
"#]]);
});

// Test too many inputs rejected for `assumePrecompile`/`assumeForgeAddress`.
// <https://github.com/foundry-rs/foundry/issues/9054>
forgetest_init!(should_revert_with_assume_code, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
        config.invariant.max_assume_rejects = 10;
        config.fuzz.seed = Some(U256::from(100u32));
    });

    // Add initial test that breaks invariant.
    prj.add_test(
        "AssumeTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract BalanceTestHandler is Test {
    address public ref = address(1412323);
    address alice;

    constructor(address _alice) {
        alice = _alice;
    }

    function increment(uint256 amount_, address addr) public {
        assumeNotPrecompile(addr);
        assumeNotForgeAddress(addr);
        assertEq(alice.balance, 100_000 ether);
    }
}

contract BalanceAssumeTest is Test {
    function setUp() public {
        address alice = makeAddr("alice");
        vm.deal(alice, 100_000 ether);
        targetSender(alice);
        BalanceTestHandler handler = new BalanceTestHandler(alice);
        targetContract(address(handler));
    }

    function invariant_balance() public {}
}
     "#,
    );

    cmd.args(["test", "--mt", "invariant_balance"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: `vm.assume` rejected too many inputs (10 allowed)] invariant_balance() (runs: [..], calls: [..], reverts: 0)
...
"#]]);
});

// Test proper message displayed if `targetSelector`/`excludeSelector` called with empty selectors.
// <https://github.com/foundry-rs/foundry/issues/9066>
forgetest_init!(should_not_panic_if_no_selectors, |prj, cmd| {
    prj.add_test(
        "NoSelectorTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract TestHandler is Test {}

contract NoSelectorTest is Test {
    bytes4[] selectors;

    function setUp() public {
        TestHandler handler = new TestHandler();
        targetSelector(FuzzSelector({addr: address(handler), selectors: selectors}));
        excludeSelector(FuzzSelector({addr: address(handler), selectors: selectors}));
    }

    function invariant_panic() public {}
}
     "#,
    );

    cmd.args(["test", "--mt", "invariant_panic"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: failed to set up invariant testing environment: No contracts to fuzz.] invariant_panic() (runs: 0, calls: 0, reverts: 0)
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/3607>
forgetest_init!(should_show_invariant_metrics, |prj, cmd| {
    prj.add_test(
        "SelectorMetricsTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterTest is Test {
    function setUp() public {
        CounterHandler handler = new CounterHandler();
        AnotherCounterHandler handler1 = new AnotherCounterHandler();
        // targetContract(address(handler1));
    }

    /// forge-config: default.invariant.runs = 10
    /// forge-config: default.invariant.show-metrics = true
    function invariant_counter() public {}

    /// forge-config: default.invariant.runs = 10
    /// forge-config: default.invariant.show-metrics = true
    function invariant_counter2() public {}
}

contract CounterHandler is Test {
    function doSomething(uint256 a) public {
        vm.assume(a < 10_000_000);
        require(a < 100_000);
    }

    function doAnotherThing(uint256 a) public {
        vm.assume(a < 10_000_000);
        require(a < 100_000);
    }
}

contract AnotherCounterHandler is Test {
    function doWork(uint256 a) public {
        vm.assume(a < 10_000_000);
        require(a < 100_000);
    }

    function doWorkThing(uint256 a) public {
        vm.assume(a < 10_000_000);
        require(a < 100_000);
    }
}
     "#,
    );

    cmd.args(["test", "--mt", "invariant_"]).assert_success().stdout_eq(str![[r#"
...
[PASS]
Invariant/Property Tests:
[PASS] invariant_counter
[PASS] invariant_counter2
 invariant_counter() (runs: 10, calls: 5000, reverts: [..])

╭-----------------------+----------------+-------+---------+----------╮
| Contract              | Selector       | Calls | Reverts | Discards |
+=====================================================================+
| AnotherCounterHandler | doWork         | [..]  | [..]    | [..]     |
|-----------------------+----------------+-------+---------+----------|
| AnotherCounterHandler | doWorkThing    | [..]  | [..]    | [..]     |
|-----------------------+----------------+-------+---------+----------|
| CounterHandler        | doAnotherThing | [..]  | [..]    | [..]     |
|-----------------------+----------------+-------+---------+----------|
| CounterHandler        | doSomething    | [..]  | [..]    | [..]     |
╰-----------------------+----------------+-------+---------+----------╯

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// Tests that invariant exists with success after configured timeout.
forgetest_init!(should_apply_configured_timeout, |prj, cmd| {
    // Add initial test that breaks invariant.
    prj.add_test(
        "TimeoutTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract TimeoutHandler is Test {
    uint256 public count;

    function increment() public {
        count++;
    }
}

contract TimeoutTest is Test {
    TimeoutHandler handler;

    function setUp() public {
        handler = new TimeoutHandler();
    }

    /// forge-config: default.invariant.runs = 10000
    /// forge-config: default.invariant.depth = 20000
    /// forge-config: default.invariant.timeout = 1
    function invariant_counter_timeout() public view {
        // Invariant will fail if more than 10000 increments.
        // Make sure test timeouts after one second and remaining runs are canceled.
        require(handler.count() < 10000);
    }
}
     "#,
    );

    cmd.args(["test", "--mt", "invariant_counter_timeout"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/TimeoutTest.t.sol:TimeoutTest
[PASS] invariant_counter_timeout() (runs: 0, calls: 0, reverts: 0)

╭----------------+-----------+-------+---------+----------╮
| Contract       | Selector  | Calls | Reverts | Discards |
+=========================================================+
| TimeoutHandler | increment | [..]  | [..]    | [..]     |
╰----------------+-----------+-------+---------+----------╯

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// Tests that selector hits are uniformly distributed
// <https://github.com/foundry-rs/foundry/issues/2986>
forgetest_init!(invariant_selectors_weight, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 10;
    });
    prj.add_source(
        "InvariantHandlers.sol",
        r#"
contract HandlerOne {
    uint256 public hit1;

    function selector1() external {
        hit1 += 1;
    }
}

contract HandlerTwo {
    uint256 public hit2;
    uint256 public hit3;
    uint256 public hit4;
    uint256 public hit5;

    function selector2() external {
        hit2 += 1;
    }

    function selector3() external {
        hit3 += 1;
    }

    function selector4() external {
        hit4 += 1;
    }

    function selector5() external {
        hit5 += 1;
    }
}
   "#,
    );

    prj.add_test(
        "InvariantSelectorsWeightTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import "src/InvariantHandlers.sol";

contract InvariantSelectorsWeightTest is Test {
    HandlerOne handlerOne;
    HandlerTwo handlerTwo;

    function setUp() public {
        handlerOne = new HandlerOne();
        handlerTwo = new HandlerTwo();
    }

    function afterInvariant() public {
        assertEq(handlerOne.hit1(), 2);
        assertEq(handlerTwo.hit2(), 2);
        assertEq(handlerTwo.hit3(), 2);
        assertEq(handlerTwo.hit4(), 1);
        assertEq(handlerTwo.hit5(), 3);
    }

    function invariant_selectors_weight() public view {}
}
   "#,
    );

    cmd.args(["test", "--fuzz-seed", "119", "--mt", "invariant_selectors_weight"]).assert_success();
});

// Tests original and new counterexample lengths are displayed on failure.
// Tests switch from regular sequence output to solidity.
forgetest_init!(invariant_sequence_len, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(10u32));
    });

    prj.add_test(
        "InvariantSequenceLenTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import "src/Counter.sol";

contract InvariantSequenceLenTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        targetContract(address(counter));
    }

    function invariant_increment() public {
        require(counter.number() / 2 < 100000000000000000000000000000000, "invariant increment failure");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_increment"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: invariant increment failure]
	[Sequence] (original: 3, shrunk: 1)
...
"#]]);

    // Check regular sequence output. Shrink disabled to show several lines.
    cmd.forge_fuse().arg("clean").assert_success();
    prj.update_config(|config| {
        config.invariant.shrink_run_limit = 0;
    });
    cmd.forge_fuse().args(["test", "--mt", "invariant_increment"]).assert_failure().stdout_eq(
        str![[r#"
...
Failing tests:
Encountered 1 failing test in test/InvariantSequenceLenTest.t.sol:InvariantSequenceLenTest
[FAIL: invariant increment failure]
	[Sequence] (original: 3, shrunk: 3)
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=increment() args=[]
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=increment() args=[]
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=setNumber(uint256) args=[284406551521730736391345481857560031052359183671404042152984097777 [2.844e65]]
 invariant_increment() (runs: 1, calls: 3, reverts: 0)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]],
    );

    // Check solidity sequence output on same failure.
    cmd.forge_fuse().arg("clean").assert_success();
    prj.update_config(|config| {
        config.invariant.show_solidity = true;
    });
    cmd.forge_fuse().args(["test", "--mt", "invariant_increment"]).assert_failure().stdout_eq(
        str![[r#"
...
Failing tests:
Encountered 1 failing test in test/InvariantSequenceLenTest.t.sol:InvariantSequenceLenTest
[FAIL: invariant increment failure]
	[Sequence] (original: 3, shrunk: 3)
		vm.prank(0x0000000000000000000000000000000000001490);
		Counter(0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f).increment();
		vm.prank(0x8ef7F804bAd9183981A366EA618d9D47D3124649);
		Counter(0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f).increment();
		vm.prank(0x00000000000000000000000000000000000016C5);
		Counter(0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f).setNumber(284406551521730736391345481857560031052359183671404042152984097777);
 invariant_increment() (runs: 1, calls: 3, reverts: 0)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]],
    );

    // Persisted failures should be able to switch output.
    prj.update_config(|config| {
        config.invariant.show_solidity = false;
    });
    cmd.forge_fuse().args(["test", "--mt", "invariant_increment"]).assert_failure().stdout_eq(
        str![[r#"
...
Failing tests:
Encountered 1 failing test in test/InvariantSequenceLenTest.t.sol:InvariantSequenceLenTest
[FAIL: invariant increment failure]
	[Sequence] (original: 3, shrunk: 3)
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=increment() args=[]
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=increment() args=[]
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=setNumber(uint256) args=[284406551521730736391345481857560031052359183671404042152984097777 [2.844e65]]
 invariant_increment() (runs: 1, calls: 1, reverts: 1)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]],
    );
});

// Tests that persisted failure is discarded if test contract was modified.
// <https://github.com/foundry-rs/foundry/issues/9965>
forgetest_init!(invariant_replay_with_different_bytecode, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 5;
    });
    prj.add_source(
        "Ownable.sol",
        r#"
contract Ownable {
    address public owner = address(777);

    function backdoor(address _owner) external {
        owner = address(888);
    }

    function changeOwner(address _owner) external {
    }
}
   "#,
    );
    prj.add_test(
        "OwnableTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import "src/Ownable.sol";

contract OwnableTest is Test {
    Ownable ownable;

    function setUp() public {
        ownable = new Ownable();
    }

    function invariant_never_owner() public {
        require(ownable.owner() != address(888), "never owner");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_never_owner"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: never owner]
...
"#]]);

    // Should replay failure if same test.
    cmd.assert_failure().stdout_eq(str![[r#"
...
[FAIL: never owner]
...
"#]]);

    // Different test driver that should not fail the invariant.
    prj.add_test(
        "OwnableTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import "src/Ownable.sol";

contract OwnableTest is Test {
    Ownable ownable;

    function setUp() public {
        ownable = new Ownable();
        // Ignore selector that fails invariant.
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = Ownable.changeOwner.selector;
        targetSelector(FuzzSelector({addr: address(ownable), selectors: selectors}));
    }

    function invariant_never_owner() public {
        require(ownable.owner() != address(888), "never owner");
    }
}
   "#,
    );
    cmd.assert_with(&[(
        "[FAILURE_PATH]",
        r#""[^"]*/cache/invariant/failures/OwnableTest/invariants/invariant_never_owner""#,
    )])
    .success()
    .stderr_eq(str![[r#"
Warning: Failure from [FAILURE_PATH] file was ignored because invariant test settings have changed: target selectors changed

"#]])
    .stdout_eq(str![[r#"
...
[PASS] invariant_never_owner() (runs: 5, calls: 25, reverts: 0)
...
"#]]);
});

forgetest_init!(invariant_replay_preserves_fail_reason, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 1;
    });
    prj.add_test(
        "InvariantReplayFailReason.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract InvariantReplayFailReason is Test {
    function setUp() public {
        targetContract(address(this));
    }

    function callTarget(uint256) external {}

    function invariant_fail_reason() public {
        fail();
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_fail_reason"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: assertion failed] invariant_fail_reason() (runs: 1, calls: 1, reverts: 0)
...
"#]]);

    // Replay should preserve failure reason instead of generic replay message.
    cmd.assert_failure().stdout_eq(str![[r#"
...
[FAIL: assertion failed] invariant_fail_reason() (runs: 1, calls: 1, reverts: 0)
...
"#]]);
});

forgetest_init!(invariant_replay_preserves_custom_error_reason, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 1;
        config.invariant.fail_on_revert = true;
    });
    prj.add_test(
        "InvariantReplayCustomError.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CustomErrorTarget {
    error InvariantCustomError(uint256, string);

    function breakInvariant() external {
        revert InvariantCustomError(111, "custom");
    }
}

contract CustomErrorHandler is Test {
    CustomErrorTarget target;

    constructor() {
        target = new CustomErrorTarget();
    }

    function callTarget() external {
        target.breakInvariant();
    }
}

contract InvariantReplayCustomError is Test {
    CustomErrorHandler handler;

    function setUp() public {
        handler = new CustomErrorHandler();
        targetContract(address(handler));
    }

    function invariant_custom_error_reason() public view {}
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_custom_error_reason"]).assert_failure().stdout_eq(str![[
        r#"
...
[FAIL: [..]custom[..]][..]
...
"#
    ]]);

    // Replay should preserve custom error string too.
    cmd.assert_failure().stdout_eq(str![[r#"
...
[FAIL: [..]custom[..]][..]
...
"#]]);
});

forgetest_init!(invariant_replay_preserves_invariant_custom_error_reason, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 1;
    });
    prj.add_test(
        "InvariantReplayInvariantCustomError.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract InvariantReplayInvariantCustomError is Test {
    error InvariantCustomError(uint256, string);

    function setUp() public {
        targetContract(address(this));
    }

    function touch(uint256) external {}

    function invariant_custom_error_reason_from_invariant() public pure {
        revert InvariantCustomError(222, "invariant custom");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_custom_error_reason_from_invariant"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: InvariantCustomError(222, "invariant custom")] invariant_custom_error_reason_from_invariant() (runs: 1, calls: 1, reverts: 0)
...
"#]]);

    // Replay should preserve invariant-level custom error string too.
    cmd.assert_failure().stdout_eq(str![[r#"
...
[FAIL: InvariantCustomError(222, "invariant custom")] invariant_custom_error_reason_from_invariant() (runs: 1, calls: 1, reverts: 0)
...
"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/10253>
forgetest_init!(invariant_test_target, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 5;
    });
    prj.add_test(
        "InvariantTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract InvariantTest is Test {
    uint256 count;

    function setCount(uint256  _count) public {
        count = _count;
    }

    function setUp() public {
    }

    function invariant_check_count() public {
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_check_count"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: failed to set up invariant testing environment: No contracts to fuzz.] invariant_check_count() (runs: 0, calls: 0, reverts: 0)
...
"#]]);

    prj.add_test(
        "InvariantTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract InvariantTest is Test {
    uint256 count;

    function setCount(uint256  _count) public {
        count = _count;
    }

    function setUp() public {
        targetContract(address(this));
    }

    function invariant_check_count() public {
    }
}
   "#,
    );

    cmd.forge_fuse().args(["test", "--mt", "invariant_check_count"]).assert_success().stdout_eq(
        str![[r#"
...
[PASS] invariant_check_count() (runs: 5, calls: 25, reverts: 0)
...
"#]],
    );
});

// Tests that reserved test functions are not fuzzed when test is set as target.
// <https://github.com/foundry-rs/foundry/issues/10469>
forgetest_init!(invariant_target_test_contract_selectors, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 100;
    });
    prj.add_test(
        "InvariantTargetTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract InvariantTargetTest is Test {
    bool fooCalled;
    bool testSanityCalled;
    bool testTableCalled;
    uint256 invariantCalledNum;
    uint256 setUpCalledNum;

    function setUp() public {
       targetContract(address(this));
    }

    function beforeTestSetup() public {
    }

    // Only this selector should be targeted.
    function foo() public {
        fooCalled = true;
    }

    function fixtureCalled() public returns (bool[] memory) {
    }

    function table_sanity(bool called) public {
        testTableCalled = called;
    }

    function test_sanity() public {
        testSanityCalled = true;
    }

    function afterInvariant() public {
    }

    function invariant_foo_called() public view {
    }

    function invariant_testSanity_considered_target() public {
    }

    function invariant_setUp_considered_target() public {
        setUpCalledNum++;
    }

    function invariant_considered_target() public {
        invariantCalledNum++;
    }
}
   "#,
    );

    cmd.args(["test", "--mc", "InvariantTargetTest", "--mt", "invariant"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/InvariantTargetTest.t.sol:InvariantTargetTest
[PASS]
Invariant/Property Tests:
[PASS] invariant_considered_target
[PASS] invariant_foo_called
[PASS] invariant_setUp_considered_target
[PASS] invariant_testSanity_considered_target
 invariant_considered_target() (runs: 10, calls: 1000, reverts: 0)

╭---------------------+----------+-------+---------+----------╮
| Contract            | Selector | Calls | Reverts | Discards |
+=============================================================+
| InvariantTargetTest | foo      | 1000  | 0       | 0        |
╰---------------------+----------+-------+---------+----------╯

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// Tests that `targetSelector` and `excludeSelector` applied on test contract selectors are
// applied.
// <https://github.com/foundry-rs/foundry/issues/11006>
forgetest_init!(invariant_target_test_include_exclude_selectors, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 100;
    });
    prj.add_test(
        "InvariantTargetTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract InvariantTargetIncludeTest is Test {
    bool include = true;
    function setUp() public {
       targetContract(address(this));
       bytes4[] memory selectors = new bytes4[](2);
       selectors[0] = this.shouldInclude1.selector;
       selectors[1] = this.shouldInclude2.selector;
       targetSelector(FuzzSelector({addr: address(this), selectors: selectors}));
    }

    function shouldExclude1() public {
        include = false;
    }

    function shouldInclude1() public {
        include = true;
    }

    function shouldExclude2() public {
        include = false;
    }

    function shouldInclude2() public {
        include = true;
    }

    function invariant_include() public view {
        require(include, "does not include");
    }
}

contract InvariantTargetExcludeTest is Test {
    bool include = true;
    function setUp() public {
       targetContract(address(this));
       bytes4[] memory selectors = new bytes4[](2);
       selectors[0] = this.shouldExclude1.selector;
       selectors[1] = this.shouldExclude2.selector;
       excludeSelector(FuzzSelector({addr: address(this), selectors: selectors}));
    }

    function shouldExclude1() public {
        include = false;
    }

    function shouldInclude1() public {
        include = true;
    }

    function shouldExclude2() public {
        include = false;
    }

    function shouldInclude2() public {
        include = true;
    }

    function invariant_exclude() public view {
        require(include, "does not include");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_include"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/InvariantTargetTest.t.sol:InvariantTargetIncludeTest
[PASS] invariant_include() (runs: 10, calls: 1000, reverts: 0)

╭----------------------------+----------------+-------+---------+----------╮
| Contract                   | Selector       | Calls | Reverts | Discards |
+==========================================================================+
| InvariantTargetIncludeTest | shouldInclude1 | [..]   | 0       | 0        |
|----------------------------+----------------+-------+---------+----------|
| InvariantTargetIncludeTest | shouldInclude2 | [..]   | 0       | 0        |
╰----------------------------+----------------+-------+---------+----------╯

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    cmd.forge_fuse().args(["test", "--mt", "invariant_exclude"]).assert_success().stdout_eq(str![
        [r#"
No files changed, compilation skipped

Ran 1 test for test/InvariantTargetTest.t.sol:InvariantTargetExcludeTest
[PASS] invariant_exclude() (runs: 10, calls: 1000, reverts: 0)

╭----------------------------+----------------+-------+---------+----------╮
| Contract                   | Selector       | Calls | Reverts | Discards |
+==========================================================================+
| InvariantTargetExcludeTest | shouldInclude1 | [..]   | 0       | 0        |
|----------------------------+----------------+-------+---------+----------|
| InvariantTargetExcludeTest | shouldInclude2 | [..]   | 0       | 0        |
╰----------------------------+----------------+-------+---------+----------╯

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]
    ]);

    cmd.forge_fuse()
        .args(["test", "--mt", "invariant_include", "--md"])
        .assert_success()
        .stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/InvariantTargetTest.t.sol:InvariantTargetIncludeTest
[PASS] invariant_include() (runs: 10, calls: 1000, reverts: 0)

| Contract                   | Selector       | Calls | Reverts | Discards |
|----------------------------|----------------|-------|---------|----------|
| InvariantTargetIncludeTest | shouldInclude1 | [..]   | 0       | 0        |
| InvariantTargetIncludeTest | shouldInclude2 | [..]   | 0       | 0        |

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    cmd.forge_fuse()
        .args(["test", "--mt", "invariant_exclude", "--md"])
        .assert_success()
        .stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/InvariantTargetTest.t.sol:InvariantTargetExcludeTest
[PASS] invariant_exclude() (runs: 10, calls: 1000, reverts: 0)

| Contract                   | Selector       | Calls | Reverts | Discards |
|----------------------------|----------------|-------|---------|----------|
| InvariantTargetExcludeTest | shouldInclude1 | [..]   | 0       | 0        |
| InvariantTargetExcludeTest | shouldInclude2 | [..]   | 0       | 0        |

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/11453>
forgetest_init!(corpus_dir, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 10;
        config.invariant.corpus.corpus_dir = Some("invariant_corpus".into());

        config.fuzz.runs = 10;
        config.fuzz.corpus.corpus_dir = Some("fuzz_corpus".into());
    });
    prj.add_test(
        "CounterTests.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract Counter1Test is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }

    function testFuzz_SetNumber(uint256 x) public {
        counter.setNumber(x);
        assertEq(counter.number(), x);
    }

    function invariant_counter_called() public view {
    }
}

contract Counter2Test is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }

    function testFuzz_SetNumber(uint256 x) public {
        counter.setNumber(x);
        assertEq(counter.number(), x);
    }

    function invariant_counter_called() public view {
    }
}
   "#,
    );

    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
...
Ran 3 test suites [ELAPSED]: 6 tests passed, 0 failed, 0 skipped (6 total tests)

"#]]);

    assert!(prj.root().join("invariant_corpus").join("Counter1Test").exists());
    assert!(prj.root().join("invariant_corpus").join("Counter2Test").exists());
    assert!(
        !prj.root()
            .join("invariant_corpus")
            .join("Counter1Test")
            .join("invariant_counter_called")
            .exists()
    );
    assert!(
        !prj.root()
            .join("invariant_corpus")
            .join("Counter2Test")
            .join("invariant_counter_called")
            .exists()
    );
    assert!(
        prj.root().join("fuzz_corpus").join("Counter1Test").join("testFuzz_SetNumber").exists()
    );
    assert!(
        prj.root().join("fuzz_corpus").join("Counter2Test").join("testFuzz_SetNumber").exists()
    );
});

forgetest_init!(contract_level_invariant_corpus_dir, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 2;
        config.invariant.depth = 2;
        config.invariant.corpus.corpus_dir = Some("invariant_corpus".into());
    });
    prj.add_test(
        "ContractCorpusTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ContractCorpusHandler {
    function touch(uint256) external {}
}

contract ContractCorpusTest is Test {
    ContractCorpusHandler handler;

    function setUp() public {
        handler = new ContractCorpusHandler();
        targetContract(address(handler));
    }

    function invariant_a() public pure {}

    function invariant_b() public pure {}
}
   "#,
    );

    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test for test/ContractCorpusTest.t.sol:ContractCorpusTest
...
"#]]);

    let contract_dir = prj.root().join("invariant_corpus").join("ContractCorpusTest");
    assert!(contract_dir.exists());
    assert!(!contract_dir.join("invariant_a").exists());
    assert!(!contract_dir.join("invariant_b").exists());
});

forgetest_init!(optimization_invariants_use_function_level_corpus_dir, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 2;
        config.invariant.corpus.corpus_dir = Some("opt_corpus".into());
    });
    prj.add_test(
        "OptimizationCorpusTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract OptimizationCorpusHandler {
    int256 public x;

    function bump() external {
        x++;
    }
}

contract OptimizationCorpusTest is Test {
    OptimizationCorpusHandler handler;

    function setUp() public {
        handler = new OptimizationCorpusHandler();
        targetContract(address(handler));
    }

    function invariant_optimize_a() public view returns (int256) {
        return handler.x();
    }

    function invariant_optimize_b() public view returns (int256) {
        return handler.x() * 10;
    }
}
   "#,
    );

    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
...
Ran 2 tests for test/OptimizationCorpusTest.t.sol:OptimizationCorpusTest
...
"#]]);

    let contract_dir = prj.root().join("opt_corpus").join("OptimizationCorpusTest");
    assert!(contract_dir.join("invariant_optimize_a").join("optimization_best.json").exists());
    assert!(contract_dir.join("invariant_optimize_b").join("optimization_best.json").exists());
    assert!(!contract_dir.join("optimization_best.json").exists());
});

forgetest_init!(json_reports_invariant_predicate_results, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 2;
    });
    prj.add_test(
        "JsonInvariantReport.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract JsonInvariantHandler {
    uint256 public counter;

    function inc() external {
        counter++;
    }
}

contract JsonInvariantReportTest is Test {
    JsonInvariantHandler handler;

    function setUp() public {
        handler = new JsonInvariantHandler();
        targetContract(address(handler));
    }

    function invariant_break() public view {
        require(handler.counter() < 1, "broken");
    }

    function invariant_safe() public pure {}
}
   "#,
    );

    let output = cmd.args(["test", "--json"]).assert_failure();
    let json: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    let suite = json.as_object().unwrap().values().next().unwrap();
    let tests = suite["test_results"].as_object().unwrap();
    assert_eq!(tests.len(), 1);
    let result = tests.values().next().unwrap();
    let predicates = result["invariant_predicate_results"].as_array().unwrap();
    assert_eq!(predicates.len(), 2);

    let broken =
        predicates.iter().find(|predicate| predicate["name"] == "invariant_break").unwrap();
    assert_eq!(broken["status"], "Failure");
    assert_eq!(broken["reason"], "broken");

    let safe = predicates.iter().find(|predicate| predicate["name"] == "invariant_safe").unwrap();
    assert_eq!(safe["status"], "Success");
    assert!(safe.get("reason").is_none());
});

forgetest_init!(invariant_campaign_reports_secondary_skip, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 2;
    });
    prj.add_test(
        "SkipPredicateReport.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract SkipPredicateHandler {
    uint256 public counter;

    function inc() external {
        counter++;
    }
}

contract SkipPredicateReportTest is Test {
    SkipPredicateHandler handler;

    function setUp() public {
        handler = new SkipPredicateHandler();
        targetContract(address(handler));
    }

    function invariant_live() public view {
        require(handler.counter() < 10, "live broken");
    }

    function invariant_skipped() public {
        vm.skip(true, "secondary");
    }
}
   "#,
    );

    let output = cmd.args(["test", "--mt", "invariant_"]).assert_success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("Ran 2 tests for test/SkipPredicateReport.t.sol:SkipPredicateReportTest"),
        "{stdout}"
    );
    assert!(stdout.contains("[PASS] invariant_live"), "{stdout}");
    assert!(stdout.contains("[SKIP: secondary] invariant_skipped"), "{stdout}");
    assert!(stdout.contains("Suite result: ok. 1 passed; 0 failed; 1 skipped;"), "{stdout}");
});

forgetest_init!(junit_reports_invariant_predicates_and_handler_failures, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 2;
        config.invariant.fail_on_revert = false;
    });
    prj.add_source(
        "JunitAssertHandler.sol",
        r#"
contract JunitAssertHandler {
    function alwaysAssert() external {
        assert(false);
    }
}
   "#,
    );
    prj.add_test(
        "JunitInvariantReport.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {JunitAssertHandler} from "../src/JunitAssertHandler.sol";

contract JunitInvariantReportTest is Test {
    JunitAssertHandler handler;

    function setUp() public {
        handler = new JunitAssertHandler();
        targetContract(address(handler));
    }

    function invariant_a() public pure {}

    function invariant_b() public pure {}
}
   "#,
    );

    let output = cmd.args(["test", "--junit", "--mt", "invariant_a"]).assert_failure();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains(r#"<testcase name="invariant_a()"#), "{stdout}");
    assert!(!stdout.contains(r#"<testcase name="invariant_b()"#), "{stdout}");
    assert!(stdout.contains(r#"<testcase name="handler src/JunitAssertHandler.sol:JunitAssertHandler::alwaysAssert""#), "{stdout}");
    assert!(stdout.contains("[FAIL: panic: assertion failed (0x01)] handler src/JunitAssertHandler.sol:JunitAssertHandler::alwaysAssert"), "{stdout}");
    assert!(stdout.contains("[Sequence] (original: 1, shrunk: 1)"), "{stdout}");
    assert!(stdout.contains("calldata=alwaysAssert() args=[]"), "{stdout}");
    assert!(stdout.contains("Suite result: FAILED. 0 passed; 1 failed; 0 skipped;"), "{stdout}");
});

forgetest_init!(junit_reports_invariant_predicate_counterexamples, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 3;
    });
    prj.add_test(
        "JunitInvariantCounterexample.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract JunitCounterHandler {
    uint256 public counter;

    function inc() external {
        counter++;
    }
}

contract JunitInvariantCounterexampleTest is Test {
    JunitCounterHandler handler;

    function setUp() public {
        handler = new JunitCounterHandler();
        targetContract(address(handler));
    }

    function invariant_breaks() public view {
        require(handler.counter() < 1, "broken");
    }

    function invariant_safe() public pure {}
}
   "#,
    );

    let output = cmd.args(["test", "--junit", "--mt", "invariant_breaks"]).assert_failure();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains(r#"<testcase name="invariant_breaks()"#), "{stdout}");
    assert!(stdout.contains("[FAIL: broken]"), "{stdout}");
    assert!(!stdout.contains(r#"<testcase name="invariant_safe()"#), "{stdout}");
    assert!(stdout.contains("[Sequence] (original:"), "{stdout}");
});

// Tests that check_interval=0 only asserts on the last call of each run.
forgetest_init!(check_interval_zero_only_checks_last_call, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 10;
        config.invariant.check_interval = 0;
    });
    prj.add_test(
        "CheckIntervalTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterHandler {
    uint256 public counter;

    function increment() public {
        counter++;
    }
}

contract CheckIntervalTest is Test {
    CounterHandler handler;

    function setUp() public {
        handler = new CounterHandler();
        targetContract(address(handler));
    }

    // This invariant would fail on intermediate calls (counter 1-9) but passes on call 10
    // With check_interval=0, only the last call is checked, so if depth=10 and counter=10
    // at the end, this should pass even though intermediate states violated the invariant.
    function invariant_counter_multiple_of_depth() public view {
        // Only passes when counter is 0 or 10 (depth). Fails for 1-9.
        require(handler.counter() == 0 || handler.counter() == 10, "not multiple of depth");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_counter"]).assert_success().stdout_eq(str![[r#"
...
[PASS] invariant_counter_multiple_of_depth() (runs: 5, calls: 50, reverts: 0)
...
"#]]);
});

// Tests that check_interval=1 (default) asserts after every call.
forgetest_init!(check_interval_one_checks_every_call, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 10;
        config.invariant.check_interval = 1;
    });
    prj.add_test(
        "CheckIntervalTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterHandler {
    uint256 public counter;

    function increment() public {
        counter++;
    }
}

contract CheckIntervalTest is Test {
    CounterHandler handler;

    function setUp() public {
        handler = new CounterHandler();
        targetContract(address(handler));
    }

    // This invariant fails as soon as counter > 5.
    // With check_interval=1, it should fail on call 6.
    function invariant_counter_le_five() public view {
        require(handler.counter() <= 5, "counter > 5");
    }
}
   "#,
    );

    assert_invariant(cmd.args(["test", "--mt", "invariant_counter"])).failure().stdout_eq(str![[
        r#"
...
[FAIL: counter > 5]
	[SEQUENCE]
...
"#
    ]]);
});

// Tests that check_interval=N checks every N calls AND always on the last call.
forgetest_init!(check_interval_n_checks_every_n_calls, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 20;
        config.invariant.check_interval = 5;
    });
    prj.add_test(
        "CheckIntervalTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterHandler {
    uint256 public counter;

    function increment() public {
        counter++;
    }
}

contract CheckIntervalTest is Test {
    CounterHandler handler;

    function setUp() public {
        handler = new CounterHandler();
        targetContract(address(handler));
    }

    // With check_interval=5 and depth=20, invariant is checked at calls 5,10,15,20.
    // This passes because 5,10,15,20 are all multiples of 5.
    function invariant_counter_multiple_of_five() public view {
        require(handler.counter() % 5 == 0, "not multiple of 5");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_counter"]).assert_success().stdout_eq(str![[r#"
...
[PASS] invariant_counter_multiple_of_five() (runs: 1, calls: 20, reverts: 0)
...
"#]]);
});

// Tests check_interval via inline config annotation.
forgetest_init!(check_interval_inline_config, |prj, cmd| {
    prj.add_test(
        "CheckIntervalInlineTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterHandler {
    uint256 public counter;

    function increment() public {
        counter++;
    }
}

contract CheckIntervalInlineTest is Test {
    CounterHandler handler;

    function setUp() public {
        handler = new CounterHandler();
        targetContract(address(handler));
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 10
    /// forge-config: default.invariant.check_interval = 0
    function invariant_only_last_checked() public view {
        // Only passes when counter is 0 or 10. With check_interval=0, only last call is checked.
        require(handler.counter() == 0 || handler.counter() == 10, "not at boundary");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_only_last_checked"]).assert_success().stdout_eq(str![[
        r#"
...
[PASS] invariant_only_last_checked() (runs: 1, calls: 10, reverts: 0)
...
"#
    ]]);
});

forgetest_init!(invariant_campaign_merges_selected_predicates, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 100;
    });
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public cond;

    function work(uint256 x) public {
        if (x % 2 != 0 && x < 9000) {
            cond++;
        } else {
            revert();
        }
    }
}
   "#,
    );
    prj.add_test(
        "CounterTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function invariant_cond1() public view {
        require(counter.cond() < 10, "condition 1 met");
    }

    function invariant_cond2() public view {
        require(counter.cond() < 15, "condition 2 met");
    }

    function invariant_cond3() public view {
        require(counter.cond() < 5, "condition 3 met");
    }

    function invariant_cond4() public view {
        require(counter.cond() < 111111, "condition 4 met");
    }

    /// forge-config: default.invariant.fail-on-revert = true
    function invariant_cond5() public view {
        require(counter.cond() < 111111, "condition 5 met");
    }
}
   "#,
    );

    // A wide filter that includes the contract's canonical predicate runs one contract-level
    // campaign and reports each selected predicate separately.
    cmd.args(["test", "--mt", "invariant_cond[1-4]"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/CounterTest.t.sol:CounterTest
[FAIL: condition 1 met] invariant_cond1
	[Sequence] (original: [..], shrunk: [..])
...

[FAIL: condition 2 met] invariant_cond2
	[Sequence] (original: [..], shrunk: [..])
...
[FAIL: condition 3 met] invariant_cond3
	[Sequence] (original: [..], shrunk: [..])
...

Invariant/Property Tests: 3/4 invariants broken
[FAIL: condition 1 met] invariant_cond1
...
"#]]);

    // Re-running a single target replays cond3's persisted counterexample and exits without
    // running a fresh campaign — only the primary block, no secondary [FAIL]s, no
    // persisted-failures footer, no `Invariant/Property Tests` roll-up. A stderr warning calls out
    // the other selected predicate with a persisted failure if it would otherwise be included.
    cmd.forge_fuse().args(["test", "--mt", "invariant_cond3"]).assert_failure().stdout_eq(str![[
        r#"
No files changed, compilation skipped
...
Ran 1 test for test/CounterTest.t.sol:CounterTest
[FAIL: condition 3 met]
	[Sequence] (original: 5, shrunk: 5)
...
 invariant_cond3() (runs: 1, calls: 1, reverts: [..])
...
"#
    ]]);
});

forgetest_init!(invariant_campaign_keeps_contract_boundary_with_no_match_test, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 20;
    });
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public cond;

    function inc() public {
        cond++;
    }
}
   "#,
    );
    prj.add_test(
        "FilteredInvariantTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract FilteredInvariantTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        targetContract(address(counter));
    }

    function invariant_a_excluded_by_filter() public view {
        require(counter.cond() < 1, "a broken");
    }

    function invariant_b_checked() public view {
        require(counter.cond() < 2, "b broken");
    }

    function invariant_c_checked() public view {
        require(counter.cond() < 3, "c broken");
    }
}
   "#,
    );

    let output = cmd.args(["test", "--nmt", "invariant_a_excluded_by_filter"]).assert_failure();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(
        stdout.contains("Ran 1 test for test/FilteredInvariantTest.t.sol:FilteredInvariantTest"),
        "{stdout}"
    );
    assert!(stdout.contains("Invariant/Property Tests: 2/2 invariants broken"), "{stdout}");
    assert!(stdout.contains("[FAIL: b broken] invariant_b_checked"), "{stdout}");
    assert!(stdout.contains("[FAIL: c broken] invariant_c_checked"), "{stdout}");
    assert!(!stdout.contains("invariant_a_excluded_by_filter"), "{stdout}");
});

// Verifies that a single selected predicate keeps the output compact: no secondary failure
// blocks and no persisted-failures footer.
forgetest_init!(invariant_campaign_single_predicate_stays_clean, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 50;
    });
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public cond;

    function inc() public {
        cond++;
    }
}
   "#,
    );
    prj.add_test(
        "CounterTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function invariant_breakable() public view {
        require(counter.cond() < 3, "primary broken");
    }

    function invariant_other_breakable() public view {
        require(counter.cond() < 4, "other broken");
    }

    function invariant_safe() public view {
        require(counter.cond() < 1000000, "should never break");
    }
}
   "#,
    );

    // Only the selected predicate runs: a single [FAIL] block, no suite roll-up, and the
    // never-selected `invariant_safe` produces no output.
    let output = cmd.args(["test", "--mt", "invariant_breakable"]).assert_failure();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("Ran 1 test for test/CounterTest.t.sol:CounterTest"), "{stdout}");
    assert!(stdout.contains("[FAIL: primary broken]"), "{stdout}");
    assert!(stdout.contains(" invariant_breakable() (runs:"), "{stdout}");
    assert!(!stdout.contains("Invariant/Property Tests:"), "{stdout}");
    assert!(!stdout.contains("invariant_other_breakable"), "{stdout}");
    assert!(!stdout.contains("invariant_safe"), "{stdout}");

    // The same isolation applies when the selected predicate is not the contract's first
    // invariant function.
    let output =
        cmd.forge_fuse().args(["test", "--mt", "invariant_other_breakable"]).assert_failure();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("[FAIL: other broken]"), "{stdout}");
    assert!(stdout.contains(" invariant_other_breakable() (runs:"), "{stdout}");
    assert!(!stdout.contains("Invariant/Property Tests:"), "{stdout}");
    assert!(!stdout.contains("invariant_breakable"), "{stdout}");
    assert!(!stdout.contains("invariant_safe"), "{stdout}");
});

// A handler `assert(false)` is routed to the dedicated `Assertion Tests:` section instead of
// being attributed to every live invariant. The live invariants stay green; the campaign keeps
// running for its full budget. See also `handler::handler_assertion_routed_to_handler_section`.
forgetest_init!(handler_assertion_is_reported_separately, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 10;
        config.invariant.fail_on_revert = false;
    });
    prj.add_source(
        "AssertHandler.sol",
        r#"
contract AssertHandler {
    uint256 public calls;

    function alwaysAssert() external {
        calls++;
        assert(false);
    }
}
   "#,
    );
    prj.add_test(
        "AssertAllAssertTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {AssertHandler} from "../src/AssertHandler.sol";

contract AssertAllAssertTest is Test {
    AssertHandler handler;

    function setUp() public {
        handler = new AssertHandler();
        targetContract(address(handler));
    }

    function invariant_a() public view {}

    function invariant_b() public view {}
}
   "#,
    );

    let output = cmd.args(["test", "--mt", "invariant_a"]).assert_failure();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("Ran 1 test for test/AssertAllAssertTest.t.sol:AssertAllAssertTest"));
    assert!(stdout.contains("Assertion Tests: 1 assertion bug(s) found"), "{stdout}");
    assert!(
        stdout.contains(
            "[FAIL: panic: assertion failed (0x01)] src/AssertHandler.sol:AssertHandler::alwaysAssert"
        ),
        "{stdout}"
    );
    assert!(stdout.contains(" invariant_a() (runs: 1, calls: 10, reverts: 10)"), "{stdout}");
    assert!(!stdout.contains("Invariant/Property Tests:"), "{stdout}");
    assert!(!stdout.contains("invariant_b"), "{stdout}");
});

// Verifies an explicitly selected optimization invariant runs as its own campaign. Boolean
// predicates in the same contract are not mixed into optimization mode; when unfiltered they run
// as a separate boolean campaign (covered below).
forgetest_init!(selected_optimization_runs_separately, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 5;
    });
    prj.add_source(
        "OptHandler.sol",
        r#"
contract OptHandler {
    uint256 public x;
    function bump(uint256 v) public { x += v % 100; }
}
   "#,
    );
    prj.add_test(
        "OptTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {OptHandler} from "../src/OptHandler.sol";

contract OptTest is Test {
    OptHandler h;
    function setUp() public { h = new OptHandler(); targetContract(address(h)); }

    /// @notice Optimization invariant — primary maximizes int256.
    function invariant_maximize() public view returns (int256) {
        return int256(h.x());
    }

    function invariant_boolean_one() public view {
        require(h.x() < 1000000, "should not exceed 1M");
    }

    function invariant_boolean_two() public view {
        require(h.x() != 42, "magic value not allowed");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_maximize"]).assert_success().stdout_eq(str![[r#"
...
[PASS]
	[Best sequence] [..]
...
 invariant_maximize() (best: [..], runs: 1, calls: 5)
...
"#]]);
});

forgetest_init!(optimization_does_not_suppress_boolean_invariants, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 2;
    });
    prj.add_test(
        "MixedInvariantTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract MixedInvariantHandler {
    uint256 public counter;

    function inc() public {
        counter++;
    }
}

contract MixedInvariantTest is Test {
    MixedInvariantHandler handler;

    function setUp() public {
        handler = new MixedInvariantHandler();
        targetContract(address(handler));
    }

    function invariant_maximize() public view returns (int256) {
        return int256(handler.counter());
    }

    function invariant_boolean() public view {
        require(handler.counter() < 1, "boolean broken");
    }
}
   "#,
    );

    assert_invariant(cmd.args(["test"])).failure().stdout_eq(str![[r#"
...
Ran 2 tests for test/MixedInvariantTest.t.sol:MixedInvariantTest
...
[FAIL: boolean broken]
	[SEQUENCE]
...
 invariant_boolean() ([RUNS])
...
[PASS]
...
 invariant_maximize() (best: [..], runs: 1, calls: 2)
...
Suite result: FAILED. 1 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
});

// Verifies that the `afterInvariant` hook keeps running on later runs even after an earlier
// invariant has already broken.
forgetest_init!(after_invariant_runs_after_earlier_failure, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 20;
    });
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public cond;

    function inc() public {
        cond++;
    }
}
   "#,
    );
    prj.add_test(
        "AfterInvariantTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract AfterInvariantTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        targetContract(address(counter));
    }

    // Breaks early in run 1.
    function invariant_first() public view {
        require(counter.cond() < 2, "first broken");
    }

    // Never breaks; keeps the campaign alive past run 1.
    function invariant_second() public view {
        require(counter.cond() < 1000000, "second broken");
    }

    // Always reverts; only reached on later runs if the hook isn't gated campaign-wide.
    function afterInvariant() public pure {
        require(false, "after_invariant_marker");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: after_invariant_marker]
...
"#]]);
});

// Verifies a stale persisted secondary failure (settings have changed since it was written) is
// not silently dropped from the campaign — the secondary is re-evaluated instead.
forgetest_init!(secondary_persisted_revalidates_on_settings_change, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 50;
        config.invariant.fail_on_revert = false;
    });
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public cond;

    function inc() public {
        cond++;
    }
}
   "#,
    );
    prj.add_test(
        "StaleSecondaryTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract StaleSecondaryTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        targetContract(address(counter));
    }

    function invariant_first() public view {
        require(counter.cond() < 2, "first broken");
    }

    function invariant_second() public view {
        require(counter.cond() < 3, "second broken");
    }
}
   "#,
    );

    // First run: both invariants break in the shared campaign and persist their counterexamples
    // under the current settings (fail_on_revert = false).
    cmd.args(["test", "--mt", "invariant_"]).assert_failure();

    // Flip a tracked InvariantSettings field so the persisted secondary cache is now stale.
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
    });

    // Re-run the shared campaign. With the fix, the stale secondary cache is rejected
    // and `invariant_second` is re-evaluated — the suite roll-up shows 2/2 broken. With the
    // bug, the bare `.exists()` check filtered the secondary out and only the primary block
    // would render (no roll-up).
    cmd.forge_fuse().args(["test", "--mt", "invariant_"]).assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/StaleSecondaryTest.t.sol:StaleSecondaryTest
[FAIL: first broken] invariant_first
	[Sequence] (original: 2, shrunk: 2)
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=inc() args=[]
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=inc() args=[]

[FAIL: second broken] invariant_second
	[Sequence] (original: 3, shrunk: 3)
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=inc() args=[]
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=inc() args=[]
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=inc() args=[]

Invariant/Property Tests: 2/2 invariants broken
[FAIL: first broken] invariant_first
[FAIL: second broken] invariant_second
2 invariant failure(s) persisted to [..]/cache/invariant/failures/StaleSecondaryTest — rerun to shrink
 invariant_first() (runs: 1, calls: 3, reverts: 0)

╭----------+----------+-------+---------+----------╮
| Contract | Selector | Calls | Reverts | Discards |
+==================================================+
| Counter  | inc      | 3     | 0       | 0        |
╰----------+----------+-------+---------+----------╯

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/StaleSecondaryTest.t.sol:StaleSecondaryTest
[FAIL: first broken] invariant_first
	[Sequence] (original: 2, shrunk: 2)
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=inc() args=[]
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=inc() args=[]

[FAIL: second broken] invariant_second
	[Sequence] (original: 3, shrunk: 3)
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=inc() args=[]
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=inc() args=[]
		sender=[..] addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=inc() args=[]

Invariant/Property Tests: 2/2 invariants broken
[FAIL: first broken] invariant_first
[FAIL: second broken] invariant_second
2 invariant failure(s) persisted to [..]/cache/invariant/failures/StaleSecondaryTest — rerun to shrink
 invariant_first() (runs: 1, calls: 3, reverts: 0)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]]);
});

// Verifies that when the campaign anchor passes but another selected predicate fails, the report
// doesn't render a hollow `[FAIL]` header for the primary and the suite roll-up counts only the
// actually-broken invariants.
forgetest_init!(secondary_only_failure_no_hollow_fail, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 50;
    });
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public cond;

    function inc() public {
        cond++;
    }
}
   "#,
    );
    prj.add_test(
        "SecondaryOnlyTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract SecondaryOnlyTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        targetContract(address(counter));
    }

    // Campaign anchor; never breaks.
    function invariant_anchor_safe() public view {
        require(counter.cond() < 1000000, "safe broken");
    }

    // Another selected predicate; breaks within the first run.
    function invariant_secondary_breakable() public view {
        require(counter.cond() < 2, "breakable broken");
    }
}
   "#,
    );

    let output = cmd.args(["test", "--mt", "invariant_"]).assert_failure();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("[FAIL: breakable broken] invariant_secondary_breakable"), "{stdout}");
    assert!(stdout.contains("Invariant/Property Tests: 1/2 invariants broken"), "{stdout}");
    assert!(stdout.contains("[PASS] invariant_anchor_safe"), "{stdout}");
    assert!(stdout.contains(" invariant_anchor_safe() (runs: 5, calls: 250, reverts: 0)"));
    assert!(!stdout.contains("[FAIL: safe broken] invariant_anchor_safe"), "{stdout}");
});

// Verifies `forge test --rerun` records the predicate that actually failed inside a merged
// campaign, not just the campaign anchor. Otherwise a secondary-only failure would be rerun as
// the passing anchor and incorrectly succeed.
forgetest_init!(rerun_replays_non_anchor_invariant_failure, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 50;
    });
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public cond;

    function inc() public {
        cond++;
    }
}
   "#,
    );
    prj.add_test(
        "RerunSecondaryOnlyTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract RerunSecondaryOnlyTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        targetContract(address(counter));
    }

    function invariant_anchor_safe() public view {
        require(counter.cond() < 1000000, "safe broken");
    }

    function invariant_secondary_breakable() public view {
        require(counter.cond() < 2, "breakable broken");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_"]).assert_failure();

    let test_failures = std::fs::read_to_string(prj.root().join("cache/test-failures")).unwrap();
    assert!(test_failures.contains("invariant_secondary_breakable"), "{test_failures}");
    assert!(!test_failures.contains("invariant_anchor_safe"), "{test_failures}");

    let output = cmd.forge_fuse().args(["test", "--rerun"]).assert_failure();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("[FAIL: breakable broken]"), "{stdout}");
    assert!(stdout.contains(" invariant_secondary_breakable() (runs:"), "{stdout}");
    assert!(!stdout.contains("invariant_anchor_safe"), "{stdout}");
});

// Verifies the structured JSON failure event emitted at campaign end attributes the broken
// invariant in declaration order (deterministic) instead of using arbitrary HashMap iteration.
forgetest_init!(failure_event_uses_declaration_order, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 5;
    });
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public cond;

    function inc() public {
        cond++;
    }
}
   "#,
    );
    prj.add_test(
        "FailureEventTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract FailureEventTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        targetContract(address(counter));
    }

    // Declaration-order: a, b, c. All break on the same call.
    function invariant_a() public view {
        require(counter.cond() < 1, "a broken");
    }

    function invariant_b() public view {
        require(counter.cond() < 1, "b broken");
    }

    function invariant_c() public view {
        require(counter.cond() < 1, "c broken");
    }
}
   "#,
    );

    // The event must name `invariant_a` (first declared broken invariant) with its matching
    // reason — not whichever entry HashMap iteration surfaces.
    cmd.args(["test", "--mt", "invariant_"]).assert_failure().stderr_eq(str![[r#"
...
{"timestamp":[..],"event":"failure","invariant":"invariant_a","target":"test/FailureEventTest.t.sol:FailureEventTest","reason":"a broken"}
...
"#]]);
});
