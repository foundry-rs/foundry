use super::symbolic_helpers::{
    assert_relevant_lines, assert_symbolic, assert_symbolic_engine, assert_symbolic_engine_witness,
};
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str, util::OutputExt};

forgetest_init!(symbolic_invariant_runs_before_fuzz_campaign, |prj, cmd| {
    skip_unless_z3!("symbolic_invariant_runs_before_fuzz_campaign");

    prj.add_test(
        "SymbolicInvariantRuns.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicInvariantTarget {
    uint256 public counter;

    function bump(uint8 amount) external {
        if (amount == 7) {
            counter = 1;
        }
    }
}

contract SymbolicInvariantRuns is Test {
    SymbolicInvariantTarget target;

    function setUp() public {
        target = new SymbolicInvariantTarget();
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 1
    function invariant_counterStaysZero() public view {
        assertEq(target.counter(), 0);
    }
}
"#,
    );

    let stdout = assert_symbolic_engine_witness(cmd.args([
        "test",
        "--symbolic",
        "--fuzz-runs",
        "0",
        "--match-test",
        "invariant_counterStaysZero",
    ]))
    .failure()
    .get_output()
    .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        str![[r#"
Encountered 1 failing test in test/SymbolicInvariantRuns.t.sol:SymbolicInvariantRuns
[FAIL: assertion failed: 1 != 0]
[Sequence] (original: 1, shrunk: 1)
calldata=bump(uint8)
invariant_counterStaysZero()
"#]],
    );
});

forgetest_init!(symbolic_invariant_safe_still_runs_fuzz_campaign, |prj, cmd| {
    skip_unless_z3!("symbolic_invariant_safe_still_runs_fuzz_campaign");

    prj.add_test(
        "SymbolicInvariantSafeRunsFuzz.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicSafeThenFuzzTarget {
    uint256 public count;

    function step() external {
        count++;
    }
}

contract SymbolicInvariantSafeRunsFuzz is Test {
    SymbolicSafeThenFuzzTarget target;

    function setUp() public {
        target = new SymbolicSafeThenFuzzTarget();
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = target.step.selector;
        targetSelector(FuzzSelector({addr: address(target), selectors: selectors}));
        targetContract(address(target));
    }

    // Symbolic checks the one-call prefix; invariant fuzzing must still run the two-call case.
    /// forge-config: default.symbolic.invariant_depth = 1
    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 2
    function invariant_countBelowTwo() public view {
        assertLt(target.count(), 2);
    }
}
"#,
    );

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_countBelowTwo",
    ]))
    .failure()
    .get_output()
    .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        str![[r#"
[FAIL:
invariant_countBelowTwo()
runs: 1, calls: 2, reverts: 0
"#]],
    );
});

forgetest_init!(symbolic_invariant_replays_setup_arbitrary_storage, |prj, cmd| {
    skip_unless_z3!("symbolic_invariant_replays_setup_arbitrary_storage");

    prj.add_test(
        "SymbolicInvariantSetupStorage.t.sol",
        r#"
import "forge-std/Test.sol";

interface ArbitraryStorageVm {
    function setArbitraryStorage(address target) external;
}

contract ExternalStore {
    uint256 public value;
}

contract StorageBackedTarget {
    ExternalStore store;
    bool public hit;

    constructor(ExternalStore store_) {
        store = store_;
    }

    function useStore() external {
        require(store.value() == 42);
        hit = true;
    }
}

contract SymbolicInvariantSetupStorage is Test {
    ExternalStore store;
    StorageBackedTarget target;

    function setUp() public {
        store = new ExternalStore();
        target = new StorageBackedTarget(store);
        ArbitraryStorageVm(address(vm)).setArbitraryStorage(address(store));
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 1
    function invariant_notHit() public view {
        require(!target.hit(), "hit");
    }
}
"#,
    );

    let stdout = assert_symbolic_engine_witness(cmd.args([
        "test",
        "--symbolic",
        "--fuzz-runs",
        "0",
        "--match-test",
        "invariant_notHit",
    ]))
    .failure()
    .get_output()
    .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        str![[r#"
[FAIL: hit]
"#]],
    );
    assert_relevant_lines(
        &stdout,
        str![[r#"
calldata=useStore()
"#]],
    );
    assert!(!stdout.contains("symbolic invariant counterexample did not replay"), "{stdout}");
});

forgetest_init!(symbolic_invariant_honors_execution_timeout, |prj, cmd| {
    skip_unless_z3!("symbolic_invariant_honors_execution_timeout");

    prj.add_test(
        "SymbolicInvariantTimeout.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicTimeoutTarget {
    uint256 scratch;

    function spin() external {
        while (true) {
            unchecked {
                scratch += 1;
            }
        }
    }
}

contract SymbolicInvariantTimeout is Test {
    SymbolicTimeoutTarget target;

    function setUp() public {
        target = new SymbolicTimeoutTarget();
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 1
    /// forge-config: default.symbolic.timeout = 1
    // Keep the depth guard out of this fixture's stop condition; it should fail
    // because the one-second symbolic timeout is reached first.
    /// forge-config: default.symbolic.max_depth = 100000000
    function invariant_alwaysTrue() public pure {}
}
"#,
    );

    assert_symbolic_engine(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_alwaysTrue",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicInvariantTimeout.t.sol:SymbolicInvariantTimeout
[FAIL: incomplete symbolic execution (Timeout): symbolic execution timeout exceeded (1s)] invariant_alwaysTrue() ([METRICS])
...
"#]]);
});

// EIP-1153 transient storage is per-transaction scratch space. The symbolic
// invariant runner must clear `state.world.transient_storage` at the boundary
// of every top-level sequence step. The target below has two entry points:
// `poke` writes a symbolic sentinel into transient slot 0, and `peek` reverts
// if transient slot 0 is non-zero. Because each call is a fresh top-level
// transaction, `peek` must always observe zero — regardless of how many
// `poke(sentinel)` calls preceded it.
forgetest_init!(symbolic_transient_storage_clears_between_sequence_steps, |prj, cmd| {
    skip_unless_z3!("symbolic_transient_storage_clears_between_sequence_steps");

    prj.add_test(
        "SymbolicTransientStorageInvariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract TransientTarget {
    function poke(uint256 sentinel) external {
        assembly { tstore(0, sentinel) }
    }

    function peek() external view {
        uint256 v;
        assembly { v := tload(0) }
        // Must hold at the start of every top-level call: transient storage
        // from a prior step must have been cleared.
        require(v == 0, "transient storage leaked across top-level steps");
    }
}

contract SymbolicTransientStorageInvariant is Test {
    TransientTarget target;

    function setUp() public {
        target = new TransientTarget();
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_transientClearsBetweenSteps() public view {
        target.peek();
    }
}
"#,
    );

    assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_transientClearsBetweenSteps",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicTransientStorageInvariant.t.sol:SymbolicTransientStorageInvariant
[PASS] invariant_transientClearsBetweenSteps() ([METRICS])
...
"#]]);
});

// A target function that branches symbolically into a revert path and a
// state-mutating path. With `fail_on_revert = false` and `invariant_depth = 2`,
// the engine must continue exploring non-reverting symbolic branches even when
// other branches of the same function revert; otherwise it would silently
// under-approximate and miss the counter increment below.
forgetest_init!(symbolic_revert_branches_do_not_swallow_non_revert_paths, |prj, cmd| {
    skip_unless_z3!("symbolic_revert_branches_do_not_swallow_non_revert_paths");

    prj.add_test(
        "SymbolicRevertBranchInvariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract RevertBranchTarget {
    uint256 public counter;

    function step(uint8 mode) external {
        if (mode == 0) {
            revert("symbolic revert branch");
        }
        unchecked { counter += 1; }
    }
}

contract SymbolicRevertBranchInvariant is Test {
    RevertBranchTarget target;

    function setUp() public {
        target = new RevertBranchTarget();
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    /// forge-config: default.invariant.fail_on_revert = false
    function invariant_counterStaysZero() public view {
        assertEq(target.counter(), 0);
    }
}
"#,
    );

    assert_symbolic_engine_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_counterStaysZero",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/SymbolicRevertBranchInvariant.t.sol:SymbolicRevertBranchInvariant
[FAIL: assertion failed: 1 != 0]
...
 invariant_counterStaysZero() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

forgetest_init!(symbolic_invariant_does_not_inherit_prank_into_nested_call, |prj, cmd| {
    skip_unless_z3!("symbolic_invariant_does_not_inherit_prank_into_nested_call");

    prj.add_test(
        "SymbolicSetupMappingStorageInvariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract SetupToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function setBalance(address account, uint256 amount) external {
        balanceOf[account] = amount;
    }

    function approve(address spender, uint256 amount) external {
        allowance[msg.sender][spender] = amount;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(allowance[from][msg.sender] >= amount, "insufficient allowance");
        require(balanceOf[from] >= amount, "insufficient balance");

        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

contract SetupSpender {
    function pull(SetupToken token, address from, uint8 amount) external {
        token.transferFrom(from, address(this), amount);
    }
}

contract SymbolicSetupMappingStorageInvariant is Test {
    SetupToken token;
    SetupSpender spender;

    function setUp() public {
        token = new SetupToken();
        spender = new SetupSpender();
        token.setBalance(address(this), 10);
        token.approve(address(spender), type(uint256).max);
        targetContract(address(this));
    }

    function pull(uint8 amount) external {
        if (amount == 7) {
            vm.startPrank(address(this));
            spender.pull(token, address(this), amount);
            vm.stopPrank();
        }
    }

    /// forge-config: default.symbolic.invariant_depth = 1
    function invariant_balanceStaysInitialized() public view {
        assertEq(token.balanceOf(address(this)), 10);
    }
}
"#,
    );

    assert_symbolic_engine_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_balanceStaysInitialized",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/SymbolicSetupMappingStorageInvariant.t.sol:SymbolicSetupMappingStorageInvariant
[FAIL: assertion failed: 3 != 10]
	[Sequence] (original: 1, shrunk: 1)
		[SENDER] [SENDER] calldata=pull(uint8) [ARGS]
 invariant_balanceStaysInitialized() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

forgetest_init!(symbolic_invariant_solves_multicall_hard_arithmetic, |prj, cmd| {
    skip_unless_z3!("symbolic_invariant_solves_multicall_hard_arithmetic");

    prj.add_test(
        "SymbolicInvariantHardArithmetic.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicHardArithmeticMarket {
    bool public borrowed;
    uint256 public supplyAssets;
    uint256 public supplyShares;
    uint256 public collateralAssets;
    uint256 public borrowAssets;
    uint256 public borrowShares;

    function supply(uint8 assets) external {
        require(assets != 0, "zero supply");
        uint256 shares = mulDivDown(assets, supplyShares + 1_000_000, supplyAssets + 1);
        supplyAssets += assets;
        supplyShares += shares;
    }

    function supplyCollateral(uint8 assets) external {
        require(assets != 0, "zero collateral");
        collateralAssets += assets;
    }

    function borrow(uint8 assets) external {
        require(assets != 0, "zero borrow");
        uint256 shares = mulDivUp(assets, borrowShares + 1_000_000, borrowAssets + 1);
        borrowAssets += assets;
        borrowShares += shares;
        require(borrowAssets <= supplyAssets, "insufficient liquidity");
        require(borrowAssets <= collateralAssets, "insufficient collateral");
        borrowed = true;
    }

    function mulDivDown(uint256 x, uint256 y, uint256 d) internal pure returns (uint256) {
        return (x * y) / d;
    }

    function mulDivUp(uint256 x, uint256 y, uint256 d) internal pure returns (uint256) {
        return (x * y + (d - 1)) / d;
    }
}

contract SymbolicInvariantHardArithmetic is Test {
    SymbolicHardArithmeticMarket target;

    function setUp() public {
        target = new SymbolicHardArithmeticMarket();
        bytes4[] memory selectors = new bytes4[](3);
        selectors[0] = target.supply.selector;
        selectors[1] = target.supplyCollateral.selector;
        selectors[2] = target.borrow.selector;
        targetSelector(FuzzSelector({addr: address(target), selectors: selectors}));
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 3
    function invariant_notBorrowed() public view {
        assertEq(target.borrowed(), false);
    }
}
"#,
    );

    assert_symbolic_engine_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_notBorrowed",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/SymbolicInvariantHardArithmetic.t.sol:SymbolicInvariantHardArithmetic
[FAIL: assertion failed: true != false]
	[Sequence] (original: 3, shrunk: 3)
		[SENDER] [SENDER] calldata=supply(uint8) [ARGS]
		[SENDER] [SENDER] calldata=supplyCollateral(uint8) [ARGS]
		[SENDER] [SENDER] calldata=borrow(uint8) [ARGS]
 invariant_notBorrowed() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// Top-level invariant sequence calls must look up code through the symbolic
// world overlay so that prior-step `vm.etch` writes are visible. The target
// below deploys a `Counter` whose `value()` returns 0, then exposes an
// `etchCounter` step that overwrites that address with bytecode that returns
// 42 for any call. If the engine fetched code from the backend instead of the
// overlay, the etch effect would be invisible at the next step and the
// (intentionally false) invariant would silently hold.
forgetest_init!(symbolic_invariant_sees_etched_code_via_overlay, |prj, cmd| {
    skip_unless_z3!("symbolic_invariant_sees_etched_code_via_overlay");

    prj.add_test(
        "SymbolicOverlayCodeInvariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract Counter {
    function value() external pure returns (uint256) {
        return 0;
    }
}

contract AlwaysReturns42 {
    fallback() external {
        assembly {
            mstore(0, 42)
            return(0, 32)
        }
    }
}

contract OverlayTarget is Test {
    Counter public c;
    address etched;

    constructor() {
        c = new Counter();
        etched = address(new AlwaysReturns42());
    }

    function etchCounter() external {
        vm.etch(address(c), etched.code);
    }

    function callCounter() external view returns (uint256) {
        return c.value();
    }
}

contract SymbolicOverlayCodeInvariant is Test {
    OverlayTarget t;

    function setUp() public {
        t = new OverlayTarget();
        targetContract(address(t));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_counterAlwaysReturnsZero() public view {
        assertEq(t.callCounter(), 0);
    }
}
"#,
    );

    assert_symbolic_engine_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_counterAlwaysReturnsZero",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/SymbolicOverlayCodeInvariant.t.sol:SymbolicOverlayCodeInvariant
[FAIL: assertion failed: 42 != 0]
	[Sequence] (original: 1, shrunk: 1)
		[SENDER] [SENDER] calldata=etchCounter() [ARGS]
 invariant_counterAlwaysReturnsZero() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
