use alloy_primitives::U256;
use foundry_test_utils::{TestCommand, forgetest_init, snapbox::cmd::OutputAssert, str};

mod storage;

fn assert_invariant(cmd: &mut TestCommand) -> OutputAssert {
    cmd.assert_with(|redactions| {
        redactions.extend([
            ("[RUNS]", r"runs: \d+, calls: \d+, reverts: \d+"),
            ("[SEQUENCE]", r"\[Sequence\].*(\n\t\t.*)*"),
        ])
    })
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
[FAIL: `vm.assume` rejected too many inputs (10 allowed)] invariant_balance() (runs: 2, calls: 1000, reverts: 0)
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
[PASS] invariant_counter() (runs: 10, calls: 5000, reverts: [..])

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

[PASS] invariant_counter2() (runs: 10, calls: 5000, reverts: [..])

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

Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

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
		sender=0x00000000000000000000000000000000000014aD addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=increment() args=[]
		sender=0x8ef7F804bAd9183981A366EA618d9D47D3124649 addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=increment() args=[]
		sender=0x00000000000000000000000000000000000016Ac addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=setNumber(uint256) args=[284406551521730736391345481857560031052359183671404042152984097777 [2.844e65]]
 invariant_increment() (runs: 0, calls: 0, reverts: 0)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

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
		vm.prank(0x00000000000000000000000000000000000014aD);
		Counter(0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f).increment();
		vm.prank(0x8ef7F804bAd9183981A366EA618d9D47D3124649);
		Counter(0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f).increment();
		vm.prank(0x00000000000000000000000000000000000016Ac);
		Counter(0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f).setNumber(284406551521730736391345481857560031052359183671404042152984097777);
 invariant_increment() (runs: 0, calls: 0, reverts: 0)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

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
[FAIL: invariant_increment replay failure]
	[Sequence] (original: 3, shrunk: 3)
		sender=0x00000000000000000000000000000000000014aD addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=increment() args=[]
		sender=0x8ef7F804bAd9183981A366EA618d9D47D3124649 addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=increment() args=[]
		sender=0x00000000000000000000000000000000000016Ac addr=[src/Counter.sol:Counter]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=setNumber(uint256) args=[284406551521730736391345481857560031052359183671404042152984097777 [2.844e65]]
 invariant_increment() (runs: 1, calls: 1, reverts: 1)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

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
[FAIL: invariant_never_owner replay failure]
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
    cmd.assert_success().stderr_eq(str![[r#"
...
Warning: Failure from "[..]/invariant/failures/OwnableTest/invariant_never_owner" file was ignored because test contract bytecode has changed.
...
"#]])
    .stdout_eq(str![[r#"
...
[PASS] invariant_never_owner() (runs: 5, calls: 25, reverts: 0)
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

Ran 4 tests for test/InvariantTargetTest.t.sol:InvariantTargetTest
[PASS] invariant_considered_target() (runs: 10, calls: 1000, reverts: 0)

╭---------------------+----------+-------+---------+----------╮
| Contract            | Selector | Calls | Reverts | Discards |
+=============================================================+
| InvariantTargetTest | foo      | 1000  | 0       | 0        |
╰---------------------+----------+-------+---------+----------╯

[PASS] invariant_foo_called() (runs: 10, calls: 1000, reverts: 0)

╭---------------------+----------+-------+---------+----------╮
| Contract            | Selector | Calls | Reverts | Discards |
+=============================================================+
| InvariantTargetTest | foo      | 1000  | 0       | 0        |
╰---------------------+----------+-------+---------+----------╯

[PASS] invariant_setUp_considered_target() (runs: 10, calls: 1000, reverts: 0)

╭---------------------+----------+-------+---------+----------╮
| Contract            | Selector | Calls | Reverts | Discards |
+=============================================================+
| InvariantTargetTest | foo      | 1000  | 0       | 0        |
╰---------------------+----------+-------+---------+----------╯

[PASS] invariant_testSanity_considered_target() (runs: 10, calls: 1000, reverts: 0)

╭---------------------+----------+-------+---------+----------╮
| Contract            | Selector | Calls | Reverts | Discards |
+=============================================================+
| InvariantTargetTest | foo      | 1000  | 0       | 0        |
╰---------------------+----------+-------+---------+----------╯

Suite result: ok. 4 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 4 tests passed, 0 failed, 0 skipped (4 total tests)

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

    assert!(
        prj.root()
            .join("invariant_corpus")
            .join("Counter1Test")
            .join("invariant_counter_called")
            .exists()
    );
    assert!(
        prj.root()
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

forgetest_init!(invariant_with_alias, |prj, cmd| {
    prj.wipe_contracts();
    prj.add_test(
        "InvariantTest1.t.sol",
        r#"
import "forge-std/Test.sol";

contract InvariantBreaker {
    bool public flag0 = true;
    bool public flag1 = true;

    function set0(int256 val) public returns (bool) {
        if (val % 100 == 0) {
            flag0 = false;
        }
        return flag0;
    }

    function set1(int256 val) public returns (bool) {
        if (val % 10 == 0 && !flag0) {
            flag1 = false;
        }
        return flag1;
    }
}

contract InvariantTest is Test {
    InvariantBreaker inv;

    function setUp() public {
        inv = new InvariantBreaker();
    }

    function invariant_neverFalse() public {
        require(inv.flag1(), "false");
    }

    function statefulFuzz_neverFalseWithInvariantAlias() public {
        require(inv.flag1(), "false");
    }
}
   "#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Encountered a total of 2 failing tests, 0 tests succeeded
...
"#]]);
});

// Test basic invariant assume functionality
forgetest_init!(invariant_assume_basic, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 10;
        config.invariant.depth = 10;
    });

    prj.add_test(
        "InvariantAssume.t.sol",
        r#"
import "forge-std/Test.sol";

contract Handler is Test {
    function doSomething(uint256 param) public {
        vm.assume(param == 0);
    }
}

contract InvariantAssume is Test {
    Handler handler;

    function setUp() public {
        handler = new Handler();
    }

    function invariant_dummy() public {}
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_dummy"]).assert_success().stdout_eq(str![[r#""#]]);
});

// Test target contracts filtering
forgetest_init!(invariant_target_contracts, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 10;
    });

    prj.add_test(
        "TargetContracts.t.sol",
        r#"
import "forge-std/Test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract TargetContracts is Test {
    Hello hello1;
    Hello hello2;

    function setUp() public {
        hello1 = new Hello();
        hello2 = new Hello();
    }

    function targetContracts() public returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(hello1);
        return addrs;
    }

    function invariantTrueWorld() public {
        require(hello2.world() == true, "false world");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantTrueWorld"]).assert_success().stdout_eq(str![[r#""#]]);
});

// Test exclude contracts filtering
forgetest_init!(invariant_exclude_contracts, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 10;
    });

    prj.add_test(
        "ExcludeContracts.t.sol",
        r#"
import "forge-std/Test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract ExcludeContracts is Test {
    Hello hello1;
    Hello hello2;

    function setUp() public {
        hello1 = new Hello();
        hello2 = new Hello();
    }

    function excludeContracts() public returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(hello1);
        return addrs;
    }

    function invariantTrueWorld() public {
        require(hello2.world() == true, "false world");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantTrueWorld"]).assert_success().stdout_eq(str![[r#""#]]);
});

// Test custom error decoding in invariants
forgetest_init!(invariant_custom_error, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 10;
        config.invariant.depth = 10;
    });

    prj.add_test(
        "InvariantCustomError.t.sol",
        r#"
import "forge-std/Test.sol";

error InvariantCustomError(uint256 value, string message);

contract Handler is Test {
    function doSomething() public {
        revert InvariantCustomError(111, "custom");
    }
}

contract InvariantCustomError is Test {
    Handler handler;

    function setUp() public {
        handler = new Handler();
    }

    function invariant_decode_error() public {}
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_decode_error"]).assert_failure().stdout_eq(str![[r#""#]]);
});

// Test invariant after invariant hook functionality
forgetest_init!(invariant_after_invariant, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 10;
    });

    prj.add_test(
        "InvariantAfterInvariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract InvariantAfterInvariantTest is Test {
    bool public flag = true;

    function setUp() public {}

    function afterInvariant() public {
        if (!flag) {
            require(false, "afterInvariant failure");
        }
    }

    function invariant_after_invariant_failure() public {
        flag = false;
        require(false, "invariant failure");
    }

    function invariant_failure() public {
        require(false, "invariant failure");
    }

    function invariant_success() public {
        require(true);
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant"]).assert_failure().stdout_eq(str![[r#""#]]);
});

// Test storage mutation detection
forgetest_init!(invariant_storage, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.depth = 100;
        config.fuzz.seed = Some(U256::from(6u32));
        config.invariant.runs = 10;
    });

    prj.add_test(
        "InvariantStorageTest.t.sol",
        r#"
import "forge-std/Test.sol";

contract InvariantStorageTest is Test {
    uint256 public storageUint = 100;
    string public storageString = "test";
    address public storageAddr = address(0x1234);
    uint256[] public uintArray;

    function pushUint(uint256 val) public {
        uintArray.push(val);
        require(uintArray.length <= 3, "pushUint");
    }

    function changeUint(uint256 val) public {
        if (storageUint != val) {
            storageUint = val;
            require(false, "changedUint");
        }
    }

    function changeString(string calldata val) public {
        if (keccak256(abi.encode(storageString)) != keccak256(abi.encode(val))) {
            storageString = val;
            require(false, "changedString");
        }
    }

    function changeAddress(address val) public {
        if (storageAddr != val) {
            storageAddr = val;
            require(false, "changedAddr");
        }
    }

    function invariantChangeUint() public {}
    function invariantChangeString() public {}
    function invariantChangeAddress() public {}
    function invariantPush() public {}
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant"]).assert_failure().stdout_eq(str![[r#""#]]);
});

// Test handler failure scenarios
forgetest_init!(invariant_handler_failure, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 1;
        config.invariant.depth = 10;
    });

    prj.add_test(
        "InvariantHandlerFailure.t.sol",
        r#"
import "forge-std/Test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract Handler is Test {
    function doSomething() public {
        require(false, "failed on revert");
    }
}

contract InvariantHandlerFailure is Test {
    bytes4[] internal selectors;

    Handler handler;

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = handler.doSomething.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function setUp() public {
        handler = new Handler();
    }

    function statefulFuzz_BrokenInvariant() public {}
}
   "#,
    );

    cmd.args(["test", "--mt", "statefulFuzz_BrokenInvariant"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
});

// Test inner contract interactions
forgetest_init!(invariant_inner_contract, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 50;
    });

    prj.add_test(
        "InvariantInnerContract.t.sol",
        r#"
import "forge-std/Test.sol";

contract Jesus {
    address fren;
    bool public identity_revealed;

    function create_fren() public {
        fren = address(new Judas());
    }

    function kiss() public {
        require(msg.sender == fren);
        identity_revealed = true;
    }
}

contract Judas {
    Jesus jesus;

    constructor() {
        jesus = Jesus(msg.sender);
    }

    function betray() public {
        jesus.kiss();
    }
}

contract InvariantInnerContract is Test {
    Jesus jesus;

    function setUp() public {
        jesus = new Jesus();
    }

    function invariantHideJesus() public {
        require(jesus.identity_revealed() == false, "jesus betrayed");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantHideJesus"]).assert_failure().stdout_eq(str![[r#""#]]);
});

// Test state preservation between runs
forgetest_init!(invariant_preserve_state, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "InvariantPreserveState.t.sol",
        r#"
import "forge-std/Test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract Handler is Test {
    function thisFunctionReverts() external {
        if (block.number < 10) {} else {
            revert();
        }
    }

    function advanceTime(uint256 blocks) external {
        blocks = blocks % 10;
        vm.roll(block.number + blocks);
        vm.warp(block.timestamp + blocks * 12);
    }
}

contract InvariantPreserveState is Test {
    Handler handler;

    function setUp() public {
        handler = new Handler();
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = handler.thisFunctionReverts.selector;
        selectors[1] = handler.advanceTime.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function invariant_preserve_state() public {
        assertTrue(true);
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_preserve_state"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
});

// Test reentrancy scenarios
forgetest_init!(invariant_reentrancy, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = false;
        config.invariant.call_override = true;
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "InvariantReentrancy.t.sol",
        r#"
import "forge-std/Test.sol";

contract Malicious {
    function world() public {
        // add code so contract is accounted as valid sender
        // see https://github.com/foundry-rs/foundry/issues/4245
        payable(msg.sender).call("");
    }
}

contract Vulnerable {
    bool public open_door = false;
    bool public stolen = false;
    Malicious mal;

    constructor(address _mal) {
        mal = Malicious(_mal);
    }

    function hello() public {
        open_door = true;
        mal.world();
        open_door = false;
    }

    function backdoor() public {
        require(open_door, "");
        stolen = true;
    }
}

contract InvariantReentrancy is Test {
    Vulnerable vuln;
    Malicious mal;

    function setUp() public {
        mal = new Malicious();
        vuln = new Vulnerable(address(mal));
    }

    // do not include `mal` in identified contracts
    // see https://github.com/foundry-rs/foundry/issues/4245
    function targetContracts() public view returns (address[] memory) {
        address[] memory targets = new address[](1);
        targets[0] = address(vuln);
        return targets;
    }

    function invariantNotStolen() public {
        require(vuln.stolen() == false, "stolen");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantNotStolen"]).assert_failure().stdout_eq(str![[r#""#]]);
});

// Test exclude selectors functionality
forgetest_init!(invariant_exclude_selectors, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "ExcludeSelectors.t.sol",
        r#"
import "forge-std/Test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract Hello {
    bool public world = false;

    function change() public {
        world = true;
    }

    function real_change() public {
        world = false;
    }
}

contract ExcludeSelectors is Test {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function excludeSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = Hello.change.selector;
        targets[0] = FuzzSelector(address(hello), selectors);
        return targets;
    }

    function invariantFalseWorld() public {
        require(hello.world() == false, "true world");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantFalseWorld"]).assert_success().stdout_eq(str![[r#""#]]);
});

// Test fuzzed target contracts
forgetest_init!(invariant_fuzzed_target_contracts, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "FuzzedTargetContracts.t.sol",
        r#"
import "forge-std/Test.sol";

// https://github.com/foundry-rs/foundry/issues/5625
// https://github.com/foundry-rs/foundry/issues/6166
// `Target.wrongSelector` is not called when handler added as `targetContract`
// `Target.wrongSelector` is called (and test fails) when no `targetContract` set
contract Target {
    uint256 count;

    function wrongSelector() external {
        revert("wrong target selector called");
    }

    function goodSelector() external {
        count++;
    }
}

contract Handler is Test {
    function increment() public {
        Target(0x6B175474E89094C44Da98b954EedeAC495271d0F).goodSelector();
    }
}

contract ExplicitTargetContract is Test {
    Handler handler;

    function setUp() public {
        Target target = new Target();
        bytes memory targetCode = address(target).code;
        vm.etch(address(0x6B175474E89094C44Da98b954EedeAC495271d0F), targetCode);

        handler = new Handler();
    }

    function targetContracts() public returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(handler);
        return addrs;
    }

    function invariant_explicit_target() public {}
}

contract DynamicTargetContract is Test {
    Handler handler;

    function setUp() public {
        Target target = new Target();
        bytes memory targetCode = address(target).code;
        vm.etch(address(0x6B175474E89094C44Da98b954EedeAC495271d0F), targetCode);

        handler = new Handler();
    }

    function invariant_dynamic_targets() public {}
}
   "#,
    );

    cmd.args(["test", "--mc", "ExplicitTargetContract"]).assert_success().stdout_eq(str![[r#""#]]);
    cmd.forge_fuse()
        .args(["test", "--mc", "DynamicTargetContract"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
});

// Test calldata dictionary with address fixtures
forgetest_init!(invariant_calldata_dictionary, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 50;
    });

    prj.add_test(
        "InvariantCalldataDictionary.t.sol",
        r#"
import "forge-std/Test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

// https://github.com/foundry-rs/foundry/issues/5868
contract Owned {
    address public owner;
    address private ownerCandidate;

    constructor() {
        owner = msg.sender;
    }

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    modifier onlyOwnerCandidate() {
        require(msg.sender == ownerCandidate);
        _;
    }

    function transferOwnership(address candidate) external onlyOwner {
        ownerCandidate = candidate;
    }

    function acceptOwnership() external onlyOwnerCandidate {
        owner = ownerCandidate;
    }
}

contract Handler is Test {
    Owned owned;

    constructor(Owned _owned) {
        owned = _owned;
    }

    function transferOwnership(address sender, address candidate) external {
        vm.assume(sender != address(0));
        vm.prank(sender);
        owned.transferOwnership(candidate);
    }

    function acceptOwnership(address sender) external {
        vm.assume(sender != address(0));
        vm.prank(sender);
        owned.acceptOwnership();
    }
}

contract InvariantCalldataDictionary is Test {
    address owner;
    Owned owned;
    Handler handler;
    address[] actors;

    function setUp() public {
        owner = address(this);
        owned = new Owned();
        handler = new Handler(owned);
        actors.push(owner);
        actors.push(address(777));
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = handler.transferOwnership.selector;
        selectors[1] = handler.acceptOwnership.selector;
        targets[0] = FuzzSelector(address(handler), selectors);
        return targets;
    }

    function fixtureSender() external returns (address[] memory) {
        return actors;
    }

    function fixtureCandidate() external returns (address[] memory) {
        return actors;
    }

    function invariant_owner_never_changes() public {
        assertEq(owned.owner(), owner);
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_owner_never_changes"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
});

// Test fixture functionality
forgetest_init!(invariant_fixtures, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 100;
    });

    prj.add_test(
        "InvariantFixtures.t.sol",
        r#"
import "forge-std/Test.sol";

contract Target {
    bool ownerFound;
    bool amountFound;
    bool magicFound;
    bool keyFound;
    bool backupFound;
    bool extraStringFound;

    function fuzzWithFixtures(
        address owner_,
        uint256 _amount,
        int32 magic,
        bytes32 key,
        bytes memory backup,
        string memory extra
    ) external {
        if (owner_ == address(0x6B175474E89094C44Da98b954EedeAC495271d0F)) {
            ownerFound = true;
        }
        if (_amount == 1122334455) amountFound = true;
        if (magic == -777) magicFound = true;
        if (key == "abcd1234") keyFound = true;
        if (keccak256(backup) == keccak256("qwerty1234")) backupFound = true;
        if (keccak256(abi.encodePacked(extra)) == keccak256(abi.encodePacked("112233aabbccdd"))) {
            extraStringFound = true;
        }
    }

    function isCompromised() public view returns (bool) {
        return ownerFound && amountFound && magicFound && keyFound && backupFound && extraStringFound;
    }
}

/// Try to compromise target contract by finding all accepted values using fixtures.
contract InvariantFixtures is Test {
    Target target;
    address[] public fixture_owner_ = [address(0x6B175474E89094C44Da98b954EedeAC495271d0F)];
    uint256[] public fixture_amount = [1, 2, 1122334455];

    function setUp() public {
        target = new Target();
    }

    function fixtureMagic() external returns (int32[2] memory) {
        int32[2] memory magic;
        magic[0] = -777;
        magic[1] = 777;
        return magic;
    }

    function fixtureKey() external pure returns (bytes32[] memory) {
        bytes32[] memory keyFixture = new bytes32[](1);
        keyFixture[0] = "abcd1234";
        return keyFixture;
    }

    function fixtureBackup() external pure returns (bytes[] memory) {
        bytes[] memory backupFixture = new bytes[](1);
        backupFixture[0] = "qwerty1234";
        return backupFixture;
    }

    function fixtureExtra() external pure returns (string[] memory) {
        string[] memory extraFixture = new string[](1);
        extraFixture[0] = "112233aabbccdd";
        return extraFixture;
    }

    function invariant_target_not_compromised() public {
        assertEq(target.isCompromised(), false);
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_target_not_compromised"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
});

// Test target selectors functionality
forgetest_init!(invariant_target_selectors, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "TargetSelectors.t.sol",
        r#"
import "forge-std/Test.sol";

struct FuzzSelector {
    address addr;
    bytes4[] selectors;
}

contract Hello {
    bool public world = true;

    function change() public {
        world = true;
    }

    function real_change() public {
        world = false;
    }
}

contract TargetSelectors is Test {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function targetSelectors() public returns (FuzzSelector[] memory) {
        FuzzSelector[] memory targets = new FuzzSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = Hello.change.selector;
        targets[0] = FuzzSelector(address(hello), selectors);
        return targets;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantTrueWorld"]).assert_success().stdout_eq(str![[r#""#]]);
});

// Test exclude senders functionality
forgetest_init!(invariant_exclude_senders, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "ExcludeSenders.t.sol",
        r#"
import "forge-std/Test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract ExcludeSenders is Test {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function excludeSenders() public returns (address[] memory) {
        address[] memory senders = new address[](1);
        senders[0] = address(this);
        return senders;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantTrueWorld"]).assert_success().stdout_eq(str![[r#""#]]);
});

// Test target senders functionality
forgetest_init!(invariant_target_senders, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "TargetSenders.t.sol",
        r#"
import "forge-std/Test.sol";

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract TargetSenders is Test {
    Hello hello;

    function setUp() public {
        hello = new Hello();
    }

    function targetSenders() public returns (address[] memory) {
        address[] memory senders = new address[](1);
        senders[0] = address(this);
        return senders;
    }

    function invariantTrueWorld() public {
        require(hello.world() == true, "false world");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantTrueWorld"]).assert_failure().stdout_eq(str![[r#""#]]);
});

// Test excluded senders functionality
forgetest_init!(invariant_excluded_senders, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "InvariantExcludedSenders.t.sol",
        r#"
import "forge-std/Test.sol";

contract InvariantSenders {
    function checkSender() external {
        require(msg.sender != 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, "sender cannot be cheatcode address");
        require(msg.sender != 0x000000000000000000636F6e736F6c652e6c6f67, "sender cannot be console address");
        require(msg.sender != 0x4e59b44847b379578588920cA78FbF26c0B4956C, "sender cannot be CREATE2 deployer");
    }
}

contract InvariantExcludedSendersTest is Test {
    InvariantSenders target;

    function setUp() public {
        target = new InvariantSenders();
    }

    function invariant_check_sender() public view {}
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_check_sender"]).assert_success().stdout_eq(str![[r#""#]]);
});

// Test target artifacts functionality
forgetest_init!(invariant_target_artifacts, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "TargetArtifacts.t.sol",
        r#"
import "forge-std/Test.sol";

contract Targeted {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract Hello {
    bool public world = true;

    function no_change() public {}
}

contract TargetArtifacts is Test {
    Targeted target1;
    Targeted target2;
    Hello hello;

    function setUp() public {
        target1 = new Targeted();
        target2 = new Targeted();
        hello = new Hello();
    }

    function targetArtifacts() public returns (string[] memory) {
        string[] memory abis = new string[](1);
        abis[0] = "default/fuzz/invariant/targetAbi/TargetArtifacts.t.sol:Targeted";
        return abis;
    }

    function invariantShouldPass() public {
        require(target2.world() == true || target1.world() == true || hello.world() == true, "false world");
    }

    function invariantShouldFail() public {
        require(target2.world() == true || target1.world() == true, "false world");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantShouldPass"]).assert_success().stdout_eq(str![[r#""#]]);
    cmd.forge_fuse()
        .args(["test", "--mt", "invariantShouldFail"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
});

// Test fork rolling functionality
forgetest_init!(invariant_roll_fork, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(119u32));
    });

    prj.add_test(
        "InvariantRollFork.t.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "forge-std/Test.sol";

interface IERC20 {
    function totalSupply() external view returns (uint256 supply);
}

contract RollForkHandler is Test {
    uint256 public totalSupply;

    function work() external {
        vm.rollFork(block.number + 1);
        totalSupply = IERC20(0x6B175474E89094C44Da98b954EedeAC495271d0F).totalSupply();
    }
}

contract InvariantRollForkBlockTest is Test {
    RollForkHandler forkHandler;

    function setUp() public {
        vm.createSelectFork("mainnet", 19812632);
        forkHandler = new RollForkHandler();
    }

    /// forge-config: default.invariant.runs = 2
    /// forge-config: default.invariant.depth = 4
    function invariant_fork_handler_block() public {
        require(block.number < 19812634, "too many blocks mined");
    }
}

contract InvariantRollForkStateTest is Test {
    RollForkHandler forkHandler;

    function setUp() public {
        vm.createSelectFork("mainnet", 19812632);
        forkHandler = new RollForkHandler();
    }

    /// forge-config: default.invariant.runs = 1
    function invariant_fork_handler_state() public {
        require(forkHandler.totalSupply() < 3254378807384273078310283461, "wrong supply");
    }
}
   "#,
    );

    cmd.args(["test", "--mc", "InvariantRollForkBlockTest"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
    cmd.forge_fuse()
        .args(["test", "--mc", "InvariantRollForkStateTest"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
});

// Test value scraping from logs and return values
forgetest_init!(invariant_scrape_values, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 50;
        config.invariant.depth = 300;
        config.invariant.fail_on_revert = true;
    });

    prj.add_test(
        "InvariantScrapeValues.t.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "forge-std/Test.sol";

contract FindFromReturnValue {
    bool public found = false;

    function seed() public returns (int256) {
        int256 mystery = 13337;
        return (1337 + mystery);
    }

    function find(int256 i) public {
        int256 mystery = 13337;
        if (i == 1337 + mystery) {
            found = true;
        }
    }
}

contract FindFromReturnValueTest is Test {
    FindFromReturnValue target;

    function setUp() public {
        target = new FindFromReturnValue();
    }

    /// forge-config: default.invariant.runs = 50
    /// forge-config: default.invariant.depth = 300
    /// forge-config: default.invariant.fail-on-revert = true
    function invariant_value_not_found() public view {
        require(!target.found(), "value from return found");
    }
}

contract FindFromLogValue {
    event FindFromLog(int256 indexed mystery, bytes32 rand);

    bool public found = false;

    function seed() public {
        int256 mystery = 13337;
        emit FindFromLog(1337 + mystery, keccak256(abi.encodePacked("mystery")));
    }

    function find(int256 i) public {
        int256 mystery = 13337;
        if (i == 1337 + mystery) {
            found = true;
        }
    }
}

contract FindFromLogValueTest is Test {
    FindFromLogValue target;

    function setUp() public {
        target = new FindFromLogValue();
    }

    /// forge-config: default.invariant.runs = 50
    /// forge-config: default.invariant.depth = 300
    /// forge-config: default.invariant.fail-on-revert = true
    function invariant_value_not_found() public view {
        require(!target.found(), "value from logs found");
    }
}
   "#,
    );

    cmd.args(["test", "--mc", "FindFromReturnValueTest"]).assert_failure().stdout_eq(str![[r#""#]]);
    cmd.forge_fuse()
        .args(["test", "--mc", "FindFromLogValueTest"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
});

// Test shrinking with assert vs require
forgetest_init!(invariant_shrink_with_assert, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(100u32));
        config.invariant.runs = 1;
        config.invariant.depth = 15;
    });

    prj.add_test(
        "InvariantShrinkWithAssert.t.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "forge-std/Test.sol";

contract Counter {
    uint256 public number;

    function increment() public {
        number++;
    }

    function decrement() public {
        number--;
    }
}

contract InvariantShrinkWithAssert is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function invariant_with_assert() public {
        assertTrue(counter.number() < 2, "wrong counter");
    }

    function invariant_with_require() public {
        require(counter.number() < 2, "wrong counter");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_with_assert"]).assert_failure().stdout_eq(str![[r#""#]]);
    cmd.forge_fuse()
        .args(["test", "--mt", "invariant_with_require"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
});

// Test sequences without reverts
forgetest_init!(invariant_sequence_no_reverts, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = false;
        config.invariant.shrink_run_limit = 0;
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "InvariantSequenceNoReverts.t.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "forge-std/Test.sol";

contract SequenceNoReverts {
    uint256 public count;

    function work(uint256 x) public {
        require(x % 2 != 0);
        count++;
    }
}

contract SequenceNoRevertsTest is Test {
    SequenceNoReverts target;

    function setUp() public {
        target = new SequenceNoReverts();
    }

    function invariant_no_reverts() public view {
        require(target.count() < 10, "condition met");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_no_reverts"]).assert_failure().stdout_eq(str![[r#""#]]);
});

// Test exclude artifacts functionality
forgetest_init!(invariant_exclude_artifacts, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "ExcludeArtifacts.t.sol",
        r#"
// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "forge-std/Test.sol";

// Will get automatically excluded. Otherwise it would throw error.
contract NoMutFunctions {
    function no_change() public pure {}
}

contract Excluded {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract Hello {
    bool public world = true;

    function change() public {
        world = false;
    }
}

contract ExcludeArtifacts is Test {
    Excluded excluded;

    function setUp() public {
        excluded = new Excluded();
        new Hello();
        new NoMutFunctions();
    }

    function excludeArtifacts() public returns (string[] memory) {
        string[] memory abis = new string[](1);
        abis[0] = "default/fuzz/invariant/targetAbi/ExcludeArtifacts.t.sol:Excluded";
        return abis;
    }

    function invariantShouldPass() public {
        require(excluded.world() == true, "false world");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantShouldPass"]).assert_success().stdout_eq(str![[r#""#]]);
});

// Test target artifact selectors functionality
forgetest_init!(invariant_target_artifact_selectors, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "TargetArtifactSelectors.t.sol",
        r#"
// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "forge-std/Test.sol";

struct FuzzArtifactSelector {
    string artifact;
    bytes4[] selectors;
}

contract Hi {
    bool public world = true;

    function no_change() public {
        world = true;
    }

    function change() public {
        world = false;
    }
}

contract TargetArtifactSelectors is Test {
    Hi hello;

    function setUp() public {
        hello = new Hi();
    }

    function targetArtifactSelectors() public returns (FuzzArtifactSelector[] memory) {
        FuzzArtifactSelector[] memory targets = new FuzzArtifactSelector[](1);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = Hi.no_change.selector;
        targets[0] =
            FuzzArtifactSelector("default/fuzz/invariant/targetAbi/TargetArtifactSelectors.t.sol:Hi", selectors);
        return targets;
    }

    function invariantShouldPass() public {
        require(hello.world() == true, "false world");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantShouldPass"]).assert_success().stdout_eq(str![[r#""#]]);
});

// Test target artifact selectors 2 functionality
forgetest_init!(invariant_target_artifact_selectors_2, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 10;
        config.invariant.depth = 20;
    });

    prj.add_test(
        "TargetArtifactSelectors2.t.sol",
        r#"
// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "forge-std/Test.sol";

struct FuzzArtifactSelector {
    string artifact;
    bytes4[] selectors;
}

contract Parent {
    bool public should_be_true = true;
    address public child;

    function change() public {
        child = msg.sender;
        should_be_true = false;
    }

    function create() public {
        new Child();
    }
}

contract Child {
    Parent parent;
    bool public changed = false;

    constructor() {
        parent = Parent(msg.sender);
    }

    function change_parent() public {
        parent.change();
    }

    function tracked_change_parent() public {
        parent.change();
    }
}

contract TargetArtifactSelectors2 is Test {
    Parent parent;

    function setUp() public {
        parent = new Parent();
    }

    function targetArtifactSelectors() public returns (FuzzArtifactSelector[] memory) {
        FuzzArtifactSelector[] memory targets = new FuzzArtifactSelector[](2);
        bytes4[] memory selectors_child = new bytes4[](1);

        selectors_child[0] = Child.change_parent.selector;
        targets[0] = FuzzArtifactSelector(
            "default/fuzz/invariant/targetAbi/TargetArtifactSelectors2.t.sol:Child", selectors_child
        );

        bytes4[] memory selectors_parent = new bytes4[](1);
        selectors_parent[0] = Parent.create.selector;
        targets[1] = FuzzArtifactSelector(
            "default/fuzz/invariant/targetAbi/TargetArtifactSelectors2.t.sol:Parent", selectors_parent
        );
        return targets;
    }

    function invariantShouldFail() public {
        if (!parent.should_be_true()) {
            require(!Child(address(parent.child())).changed(), "should have not happened");
        }
        require(parent.should_be_true() == true, "it's false");
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariantShouldFail"]).assert_failure().stdout_eq(str![[r#""#]]);
});

// Test shrink big sequence functionality
forgetest_init!(invariant_shrink_big_sequence, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(119u32));
        config.invariant.runs = 1;
        config.invariant.depth = 1000;
    });

    prj.add_test(
        "InvariantShrinkBigSequence.t.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "forge-std/Test.sol";

contract ShrinkBigSequence {
    uint256 cond;

    function work(uint256 x) public {
        if (x % 2 != 0 && x < 9000) {
            cond++;
        }
    }

    function checkCond() public view {
        require(cond < 77, "condition met");
    }
}

contract ShrinkBigSequenceTest is Test {
    ShrinkBigSequence target;

    function setUp() public {
        target = new ShrinkBigSequence();
    }

    function invariant_shrink_big_sequence() public view {
        target.checkCond();
    }
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_shrink_big_sequence"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
});

// Test shrink fail on revert functionality
forgetest_init!(invariant_shrink_fail_on_revert, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(119u32));
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 1;
        config.invariant.depth = 200;
    });

    prj.add_test(
        "InvariantShrinkFailOnRevert.t.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "forge-std/Test.sol";

contract ShrinkFailOnRevert {
    uint256 cond;

    function work(uint256 x) public {
        if (x % 2 != 0 && x < 9000) {
            cond++;
        }
        require(cond < 10, "condition met");
    }
}

contract ShrinkFailOnRevertTest is Test {
    ShrinkFailOnRevert target;

    function setUp() public {
        target = new ShrinkFailOnRevert();
    }

    function invariant_shrink_fail_on_revert() public view {}
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_shrink_fail_on_revert"])
        .assert_failure()
        .stdout_eq(str![[r#""#]]);
});
