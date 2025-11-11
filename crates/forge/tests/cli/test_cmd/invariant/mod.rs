use alloy_primitives::U256;
use foundry_test_utils::{TestCommand, forgetest_init, snapbox::cmd::OutputAssert, str};

mod common;
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
