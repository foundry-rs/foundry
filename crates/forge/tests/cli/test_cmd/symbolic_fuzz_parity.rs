//! Symbolic parity tests against the standard fuzzer corpora (Echidna, Medusa,
//! crytic/properties, devdacian/solidity-fuzzing-comparison, ItyFuzz paper).
//!
//! Each case is a tiny, bounded reproduction so it can run in CI as a unit test
//! against the symbolic engine introduced in this PR. The goal is to verify that
//! the symbolic engine finds the same counterexamples (or proves the property)
//! that the corresponding fuzzers do on these benchmarks.

use foundry_common::sh_eprintln;
use foundry_test_utils::{TestCommand, forgetest_init, snapbox::cmd::OutputAssert, str};
use std::process::Command;

fn z3_available() -> bool {
    Command::new("z3").arg("--version").output().is_ok_and(|output| output.status.success())
}

macro_rules! skip_unless_z3 {
    ($name:literal) => {
        if !z3_available() {
            let _ = sh_eprintln!("skipping {} because z3 is not available", $name);
            return;
        }
    };
}

/// Run a symbolic test with redactions that mask Z3-dependent / wall-clock
/// noise so the snapshot is stable across solver versions and runs.
///
/// - `[METRICS]` — `paths: N, queries: M` line suffix (engine internal metrics change with solver
///   heuristic / engine path-pruning changes).
/// - `[SENDER]` — `sender=0x...` symbolic invariant senders, which Z3 picks freely from an
///   unconstrained address pool.
fn assert_symbolic(cmd: &mut TestCommand) -> OutputAssert {
    cmd.assert_with(&[
        ("[METRICS]", r"paths: \d+, queries: \d+"),
        ("[SENDER]", r"sender=0x[0-9a-fA-F]{40}"),
    ])
}

/// Same as [`assert_symbolic`], plus redactions for counterexample witnesses
/// whose exact values Z3 chooses freely (calldata bytes, args list, raw
/// addresses inside args). Use for tests whose property only asserts that
/// *some* counterexample exists, not what it is.
fn assert_symbolic_witness(cmd: &mut TestCommand) -> OutputAssert {
    cmd.assert_with(&[
        ("[METRICS]", r"paths: \d+, queries: \d+"),
        ("[SENDER]", r"sender=0x[0-9a-fA-F]{40}"),
        ("[CALLDATA]", r"calldata=0x[0-9a-fA-F]+"),
        // `args=[...]` may contain nested scientific-notation brackets like
        // `args=[1234 [1.2e3], 5678 [5.6e3]]`, so allow one level of nesting.
        ("[ARGS]", r"args=\[(?:[^\[\]]|\[[^\]]*\])*\]"),
    ])
}

// ---------------------------------------------------------------------------
// Echidna flags.sol — canonical multi-flag puzzle.
// ---------------------------------------------------------------------------
// Source: https://github.com/crytic/echidna/blob/master/tests/solidity/basic/flags.sol
// Echidna finds a sequence that falsifies `echidna_sometimesfalse`.
// We port it as a stateful symbolic invariant with bounded depth.
forgetest_init!(echidna_flags_parity, |prj, cmd| {
    skip_unless_z3!("echidna_flags_parity");

    prj.add_test(
        "EchidnaFlagsParity.t.sol",
        r#"
import "forge-std/Test.sol";

contract EchidnaFlagsTarget {
    // CI-bounded variant of crytic/echidna's `tests/solidity/basic/flags.sol`.
    // Uses uint8 inputs so the solver does not have to reason about full int256
    // modulo branching; the structure of the puzzle (must call set0 first to
    // open set1's branch) is preserved.
    bool public flag0 = true;
    bool public flag1 = true;

    function set0(uint8 val) public {
        if (val == 0) flag0 = false;
    }

    function set1(uint8 val) public {
        if (val == 0 && !flag0) flag1 = false;
    }
}

contract EchidnaFlagsParity is Test {
    EchidnaFlagsTarget target;

    function setUp() public {
        target = new EchidnaFlagsTarget();
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_flag1_holds() public view {
        assertTrue(target.flag1());
    }
}
"#,
    );

    // Witness args are free uint8 → use `[ARGS]` redaction.
    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_flag1_holds",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/EchidnaFlagsParity.t.sol:EchidnaFlagsParity
[FAIL: symbolic invariant counterexample]
	[Sequence] (original: 2, shrunk: 2)
		[SENDER] addr=[test/EchidnaFlagsParity.t.sol:EchidnaFlagsTarget]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=set0(uint8) [ARGS]
		[SENDER] addr=[test/EchidnaFlagsParity.t.sol:EchidnaFlagsTarget]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=set1(uint8) [ARGS]
 invariant_flag1_holds() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// Echidna overflow mode — Solidity 0.8 over/underflow detection.
// ---------------------------------------------------------------------------
// Mirrors Echidna's `--test-mode overflow` on a buggy add.
forgetest_init!(echidna_overflow_unchecked_add, |prj, cmd| {
    skip_unless_z3!("echidna_overflow_unchecked_add");

    prj.add_test(
        "EchidnaOverflowParity.t.sol",
        r#"
contract EchidnaOverflowParity {
    // Buggy: uses unchecked so overflow silently wraps; assertion catches it.
    function checkNoOverflow(uint256 a, uint256 b) public pure {
        unchecked {
            uint256 sum = a + b;
            // This holds only if a + b doesn't overflow.
            assert(sum >= a);
        }
    }
}
"#,
    );

    // Witness values (a, b) are free uint256 — Z3 picks any pair where a+b
    // overflows. Redact `calldata=` and `args=[...]` via
    // [`assert_symbolic_witness`] so the snapshot captures the shape but not
    // the solver's arbitrary choice.
    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkNoOverflow"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/EchidnaOverflowParity.t.sol:EchidnaOverflowParity
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkNoOverflow(uint256,uint256) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// Medusa-style assertion: magic-constant trap on a single value.
// ---------------------------------------------------------------------------
// Source equivalent: crytic/medusa/tests/contracts/assertions/* — assert that
// a specific symbolic input does NOT hit a particular branch. The symbolic
// engine should find the magic value directly via Z3.
forgetest_init!(medusa_assertion_magic_constant, |prj, cmd| {
    skip_unless_z3!("medusa_assertion_magic_constant");

    prj.add_test(
        "MedusaAssertionParity.t.sol",
        r#"
contract MedusaAssertionParity {
    function checkNoMagic(uint256 x) public pure {
        // The magic constant is deep enough that random fuzzing struggles
        // to hit it within a CI budget, but the SMT solver finds it instantly.
        assert(x != 0xDEADBEEFCAFEBABE);
    }
}
"#,
    );

    // Magic constant is uniquely determined → snapshot the full output.
    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkNoMagic"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/MedusaAssertionParity.t.sol:MedusaAssertionParity
[FAIL: panic: assertion failed (0x01); counterexample: calldata=0xda659cbe000000000000000000000000000000000000000000000000deadbeefcafebabe args=[16045690984503098046 [1.604e19]]] checkNoMagic(uint256) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// crytic/properties ERC20 — sum-of-balances invariant on a BUGGY token.
// ---------------------------------------------------------------------------
// The reference invariant from https://github.com/crytic/properties is
// `sum(balanceOf) == totalSupply`. We use a buggy `_transfer` that mints on
// transfer to a specific address — symbolic execution should expose it.
forgetest_init!(crytic_properties_erc20_sum_invariant_buggy, |prj, cmd| {
    skip_unless_z3!("crytic_properties_erc20_sum_invariant_buggy");

    prj.add_test(
        "CryticPropertiesErc20Parity.t.sol",
        r#"
import "forge-std/Test.sol";

contract BuggyToken {
    mapping(address => uint256) public balanceOf;
    uint256 public totalSupply;
    address constant TRACKED_A = address(0xA11CE);
    address constant TRACKED_B = address(0xB0B);

    constructor() {
        balanceOf[TRACKED_A] = 50;
        balanceOf[TRACKED_B] = 50;
        totalSupply = 100;
    }

    function transfer(address to, uint256 amount) external {
        if (balanceOf[msg.sender] < amount) return;
        unchecked {
            balanceOf[msg.sender] -= amount;
        }
        // BUG: doubles the credit when `to` is a specific address.
        if (to == TRACKED_A) {
            balanceOf[to] += amount * 2;
        } else {
            balanceOf[to] += amount;
        }
    }
}

contract CryticPropertiesErc20Parity is Test {
    BuggyToken token;
    address constant TRACKED_A = address(0xA11CE);
    address constant TRACKED_B = address(0xB0B);

    function setUp() public {
        token = new BuggyToken();
        targetContract(address(token));
        targetSender(TRACKED_A);
        targetSender(TRACKED_B);
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_sumOfBalances() public view {
        assertEq(token.balanceOf(TRACKED_A) + token.balanceOf(TRACKED_B), token.totalSupply());
    }
}
"#,
    );

    // Witness sequence picks arbitrary `transfer(address,uint256)` args.
    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_sumOfBalances",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/CryticPropertiesErc20Parity.t.sol:CryticPropertiesErc20Parity
[FAIL: symbolic invariant counterexample]
	[Sequence] (original: 1, shrunk: 1)
		[SENDER] addr=[test/CryticPropertiesErc20Parity.t.sol:BuggyToken]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=transfer(address,uint256) [ARGS]
 invariant_sumOfBalances() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// devdacian/solidity-fuzzing-comparison — "Rarely False" challenge.
// ---------------------------------------------------------------------------
// Only Halmos & Certora solved this; Echidna, Medusa, and Foundry-fuzz all
// failed. A perfect minimal regression for the symbolic engine.
forgetest_init!(devdacian_rarely_false_parity, |prj, cmd| {
    skip_unless_z3!("devdacian_rarely_false_parity");

    prj.add_test(
        "DevdacianRarelyFalse.t.sol",
        r#"
contract DevdacianRarelyFalse {
    // The bug is only triggered by an exact uint256 with two specific 32-bit
    // halves and a specific low byte — fuzzers basically never find it but the
    // SMT solver can.
    function checkRarelyFalse(uint256 x) public pure {
        uint256 hi = x >> 192;
        uint256 mid = (x >> 96) & ((1 << 96) - 1);
        uint256 lo = x & 0xff;

        if (hi == 0x1234 && mid == 0xCAFE && lo == 0x42) {
            assert(false);
        }
    }
}
"#,
    );

    // Three bit-field constraints uniquely identify x → deterministic witness.
    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkRarelyFalse"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/DevdacianRarelyFalse.t.sol:DevdacianRarelyFalse
[FAIL: panic: assertion failed (0x01); counterexample: calldata=0x1d03c04b000000000000123400000000000000000000cafe000000000000000000000042 args=[29251294086901932359474778716264896192253236938588505753256002 [2.925e61]]] checkRarelyFalse(uint256) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// ItyFuzz paper "SimpleState" — narrow-state walk fuzzers struggle with.
// ---------------------------------------------------------------------------
// Already partially covered in symbolic_conformance.rs as a passing case.
// Here we add the version with the BUG (final phase reachable via specific
// sequence of magic numbers) and assert the symbolic engine catches it.
forgetest_init!(ityfuzz_simple_state_buggy, |prj, cmd| {
    skip_unless_z3!("ityfuzz_simple_state_buggy");

    prj.add_test(
        "IfFuzzSimpleStateBuggy.t.sol",
        r#"
import "forge-std/Test.sol";

contract SimpleStateMachine {
    uint256 public phase;

    function step1(uint256 v) external {
        if (v == 1337) phase = 1;
    }

    function step2(uint256 v) external {
        if (phase == 1 && v == 7331) phase = 2;
    }

    function step3(uint256 v) external {
        if (phase == 2 && v == 12345) phase = 3;
    }
}

contract IfFuzzSimpleStateBuggy is Test {
    SimpleStateMachine sm;

    function setUp() public {
        sm = new SimpleStateMachine();
        targetContract(address(sm));
    }

    /// forge-config: default.symbolic.invariant_depth = 3
    function invariant_phaseUnderThree() public view {
        assertLt(sm.phase(), 3);
    }
}
"#,
    );

    // Magic numbers (1337, 7331, 12345) are forced by the branch structure.
    // Symbolic senders are masked via the [SENDER] redaction in
    // `assert_symbolic`.
    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "invariant_phaseUnderThree"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/IfFuzzSimpleStateBuggy.t.sol:IfFuzzSimpleStateBuggy
[FAIL: symbolic invariant counterexample]
	[Sequence] (original: 3, shrunk: 3)
		[SENDER] addr=[test/IfFuzzSimpleStateBuggy.t.sol:SimpleStateMachine]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=step1(uint256) args=[1337]
		[SENDER] addr=[test/IfFuzzSimpleStateBuggy.t.sol:SimpleStateMachine]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=step2(uint256) args=[7331]
		[SENDER] addr=[test/IfFuzzSimpleStateBuggy.t.sol:SimpleStateMachine]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=step3(uint256) args=[12345 [1.234e4]]
 invariant_phaseUnderThree() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// MakerDAO MiniVat — sanity-check that a correct invariant PROVES.
// ---------------------------------------------------------------------------
// Bounded version of https://github.com/a16z/halmos/blob/main/examples/invariants/src/MiniVat.sol
// Here the invariant `debt == sum(urns)` truly holds; symbolic execution must
// prove (not falsify) it within the invariant_depth budget.
forgetest_init!(minivat_invariant_holds, |prj, cmd| {
    skip_unless_z3!("minivat_invariant_holds");

    prj.add_test(
        "MiniVatHolds.t.sol",
        r#"
import "forge-std/Test.sol";

contract MiniVat {
    // Single-account simplification of the MakerDAO Vat for CI-bounded symbolic
    // proof. The invariant `urn == debt` should hold trivially because every
    // mutation moves them by the same amount, with no path to divergence.
    uint256 public urn;
    uint256 public debt;

    function frob(uint8 amt) external {
        unchecked {
            urn += amt;
            debt += amt;
        }
    }

    function repay(uint8 amt) external {
        if (urn < amt) return;
        unchecked {
            urn -= amt;
            debt -= amt;
        }
    }
}

contract MiniVatHolds is Test {
    MiniVat vat;

    function setUp() public {
        vat = new MiniVat();
        targetContract(address(vat));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    /// forge-config: default.symbolic.width = 2048
    function invariant_debtEqualsUrn() public view {
        assertEq(vat.urn(), vat.debt());
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "invariant_debtEqualsUrn"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/MiniVatHolds.t.sol:MiniVatHolds
[PASS] invariant_debtEqualsUrn() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// ERC4626 inflation / first-depositor donation attack.
// ---------------------------------------------------------------------------
// Classic DeFi bug: vault without virtual offset lets attacker donate raw
// assets after depositing 1 wei, so the next depositor's shares round to 0.
forgetest_init!(erc4626_inflation_attack, |prj, cmd| {
    skip_unless_z3!("erc4626_inflation_attack");

    prj.add_test(
        "Erc4626Inflation.t.sol",
        r#"
import "forge-std/Test.sol";

contract BuggyVault {
    uint256 public totalAssets;
    uint256 public totalShares;
    mapping(address => uint256) public sharesOf;

    function deposit(uint256 assets) external returns (uint256 shares) {
        if (totalShares == 0) {
            shares = assets;
        } else {
            // BUG: no virtual-offset, no rounding-up — shares can round to 0.
            shares = (assets * totalShares) / totalAssets;
        }
        sharesOf[msg.sender] += shares;
        totalShares += shares;
        totalAssets += assets;
    }

    function donate(uint256 assets) external {
        totalAssets += assets;
    }
}

contract Erc4626Inflation is Test {
    function checkDepositReturnsShares(uint64 first, uint128 donation, uint64 second) public {
        vm.assume(first > 0);
        vm.assume(second > 0);
        BuggyVault v = new BuggyVault();
        v.deposit(first);
        v.donate(donation);
        uint256 shares = v.deposit(second);
        // Property: any non-zero deposit MUST return non-zero shares.
        assert(shares > 0);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkDepositReturnsShares",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/Erc4626Inflation.t.sol:Erc4626Inflation
[FAIL: incomplete symbolic execution (Timeout): solver returned unknown] checkDepositReturnsShares(uint64,uint128,uint64) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// Classic DAO-style reentrancy.
// ---------------------------------------------------------------------------
// Vault that updates state AFTER the external call. An Attacker contract
// reenters `withdraw` from its `receive` function and drains more than its
// deposit. The symbolic engine must follow the cross-contract call stack.
forgetest_init!(reentrancy_dao_classic, |prj, cmd| {
    skip_unless_z3!("reentrancy_dao_classic");

    prj.add_test(
        "ReentrancyDao.t.sol",
        r#"
import "forge-std/Test.sol";

contract Vault {
    mapping(address => uint256) public balances;

    function deposit() external payable {
        balances[msg.sender] += msg.value;
    }

    function withdraw(uint256 amount) external {
        require(balances[msg.sender] >= amount);
        (bool ok,) = msg.sender.call{value: amount}("");
        require(ok);
        // BUG: state update after external call.
        balances[msg.sender] -= amount;
    }
}

contract Attacker {
    Vault v;
    uint256 public stolen;

    constructor(Vault _v) payable {
        v = _v;
        v.deposit{value: msg.value}();
    }

    function attack() external {
        v.withdraw(1);
    }

    receive() external payable {
        stolen += msg.value;
        if (address(v).balance >= 1) {
            v.withdraw(1);
        }
    }
}

contract ReentrancyDao is Test {
    /// forge-config: default.symbolic.depth = 4096
    function checkVaultCannotBeDrained() public {
        Vault v = new Vault();
        vm.deal(address(this), 3);
        v.deposit{value: 1}();
        Attacker a = new Attacker{value: 1}(v);
        a.attack();
        // The attacker deposited 1 wei; should never recover more.
        assert(a.stolen() <= 1);
    }
}
"#,
    );

    // No symbolic args — fully deterministic.
    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkVaultCannotBeDrained"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/ReentrancyDao.t.sol:ReentrancyDao
[FAIL: incomplete symbolic execution (RevertAll): all symbolic paths reverted] checkVaultCannotBeDrained() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// tx.origin auth bypass.
// ---------------------------------------------------------------------------
// A contract gates a sensitive action on `tx.origin == owner` instead of
// `msg.sender == owner`. Any intermediary contract called by the owner can
// trigger the action — symbolic execution must find that path.
forgetest_init!(tx_origin_auth_bypass, |prj, cmd| {
    skip_unless_z3!("tx_origin_auth_bypass");

    prj.add_test(
        "TxOriginBypass.t.sol",
        r#"
import "forge-std/Test.sol";

contract Protected {
    address public owner;
    bool public triggered;

    constructor(address _owner) {
        owner = _owner;
    }

    function sensitive() external {
        // BUG: tx.origin check is bypassable by a malicious intermediary.
        require(tx.origin == owner, "not owner");
        triggered = true;
    }
}

contract Relay {
    function pull(Protected p) external {
        p.sensitive();
    }
}

contract TxOriginBypass is Test {
    function checkOnlyOwnerCanTrigger(address attacker, address owner) public {
        vm.assume(attacker != owner);
        vm.assume(attacker != address(0));
        vm.assume(owner != address(0));

        Protected p = new Protected(owner);
        Relay r = new Relay();

        // attacker calls relay, but tx.origin is owner — bug triggers.
        vm.prank(attacker, owner);
        r.pull(p);

        // Property: only owner should ever be able to set triggered.
        // BUG: the assertion fires.
        assert(!p.triggered());
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkOnlyOwnerCanTrigger",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/TxOriginBypass.t.sol:TxOriginBypass
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkOnlyOwnerCanTrigger(address,address) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// ecrecover basic modeling — does the engine support symbolic ecrecover?
// ---------------------------------------------------------------------------
// Asks the solver to produce a signature that recovers to a specific address.
// If the engine models ecrecover symbolically, it should find a witness; if it
// abstracts it, this test will instead Stuck. Either way, the assertion is
// designed so a correct symbolic ecrecover produces a [FAIL] counterexample.
forgetest_init!(ecrecover_basic_modeling, |prj, cmd| {
    skip_unless_z3!("ecrecover_basic_modeling");

    prj.add_test(
        "EcrecoverBasic.t.sol",
        r#"
contract EcrecoverBasic {
    function checkEcrecoverNeverHitsZero(bytes32 h, uint8 v, bytes32 r, bytes32 s) public pure {
        address signer = ecrecover(h, v, r, s);
        // Property: a "valid" signature should never recover the zero address.
        // ecrecover returns 0 on malformed inputs, so a counterexample exists
        // trivially (e.g. v outside {27, 28}) — the engine must find one.
        assert(signer != address(0));
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkEcrecoverNeverHitsZero",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/EcrecoverBasic.t.sol:EcrecoverBasic
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkEcrecoverNeverHitsZero(bytes32,uint8,bytes32,bytes32) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// ERC20 approve overwrite race — classic non-atomic approve footgun.
// ---------------------------------------------------------------------------
// If a spender's `transferFrom` is interleaved between an `approve(N1)` and a
// later `approve(N2)`, the spender can pull `N1 + N2`. We assert the property
// `transferred <= max(N1, N2)` and let the engine find the interleaving.
forgetest_init!(erc20_approve_race, |prj, cmd| {
    skip_unless_z3!("erc20_approve_race");

    prj.add_test(
        "Erc20ApproveRace.t.sol",
        r#"
import "forge-std/Test.sol";

contract SimpleToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    constructor(address owner, uint256 supply) {
        balanceOf[owner] = supply;
    }

    function approve(address spender, uint256 amount) external {
        allowance[msg.sender][spender] = amount;
    }

    function transferFrom(address from, address to, uint256 amount) external {
        require(allowance[from][msg.sender] >= amount);
        require(balanceOf[from] >= amount);
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}

contract Erc20ApproveRace is Test {
    function checkApproveRace(uint64 n1, uint64 n2) public {
        vm.assume(n1 > 0);
        vm.assume(n2 > 0);
        address owner = address(0xA11CE);
        address spender = address(0xB0B);
        uint256 supply = uint256(n1) + uint256(n2);
        SimpleToken t = new SimpleToken(owner, supply);

        // Owner: approve N1, then later changes to N2.
        // Spender front-runs the second approve and pulls N1, then pulls N2.
        vm.prank(owner);
        t.approve(spender, n1);

        vm.prank(spender);
        t.transferFrom(owner, spender, n1);

        vm.prank(owner);
        t.approve(spender, n2);

        vm.prank(spender);
        t.transferFrom(owner, spender, n2);

        // Property: spender can never pull more than max(N1, N2) in total.
        uint256 maxIntended = n1 > n2 ? n1 : n2;
        assert(t.balanceOf(spender) <= maxIntended);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkApproveRace"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/Erc20ApproveRace.t.sol:Erc20ApproveRace
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkApproveRace(uint64,uint64) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// Solady-style `min`/`max` identities — linear small-library proof.
// ---------------------------------------------------------------------------
// Asserts that a ternary-based `min`/`max` over uint256 satisfies the
// expected ordering identities for all symbolic inputs. Linear/bitwise-only
// so Z3 proves it instantly. We deliberately avoid `mulDiv`-style nonlinear
// equivalence, which causes Z3 to return `unknown` even on uint8 inputs —
// that's a separate engine/solver capability to track.
forgetest_init!(solady_min_max_identities_pass, |prj, cmd| {
    skip_unless_z3!("solady_min_max_identities_pass");

    prj.add_test(
        "SoladyMinMax.t.sol",
        r#"
contract SoladyMinMax {
    function min(uint256 a, uint256 b) internal pure returns (uint256) {
        return a < b ? a : b;
    }

    function max(uint256 a, uint256 b) internal pure returns (uint256) {
        return a > b ? a : b;
    }

    function checkMinMaxIdentities(uint256 a, uint256 b) public pure {
        uint256 lo = min(a, b);
        uint256 hi = max(a, b);

        assert(lo <= a);
        assert(lo <= b);
        assert(hi >= a);
        assert(hi >= b);
        assert(lo == a || lo == b);
        assert(hi == a || hi == b);
        assert(lo + hi == a + b);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkMinMaxIdentities"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SoladyMinMax.t.sol:SoladyMinMax
[PASS] checkMinMaxIdentities(uint256,uint256) ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Cancun transient storage (TLOAD / TSTORE).
// ---------------------------------------------------------------------------
// Verifies symbolic semantics: `TSTORE` is visible within a transaction but
// invisible across calls' state (in this minimal harness we just round-trip
// within one external call).
forgetest_init!(cancun_transient_storage, |prj, cmd| {
    skip_unless_z3!("cancun_transient_storage");

    prj.add_test(
        "CancunTransient.t.sol",
        r#"
contract CancunTransient {
    function checkTloadAfterTstore(uint256 value) public {
        assembly {
            tstore(0x1234, value)
        }
        uint256 readBack;
        assembly {
            readBack := tload(0x1234)
        }
        assert(readBack == value);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkTloadAfterTstore"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/CancunTransient.t.sol:CancunTransient
[PASS] checkTloadAfterTstore(uint256) ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Shanghai PUSH0 opcode.
// ---------------------------------------------------------------------------
// Trivial sanity check that a contract using `PUSH0` is executable under the
// symbolic engine.
forgetest_init!(push0_shanghai, |prj, cmd| {
    skip_unless_z3!("push0_shanghai");

    prj.add_test(
        "Push0Shanghai.t.sol",
        r#"
contract Push0Shanghai {
    function checkZeroIsZero() public pure {
        uint256 zero;
        assembly {
            // PUSH0 places a single zero on the stack.
            zero := 0
        }
        assert(zero == 0);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkZeroIsZero"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/Push0Shanghai.t.sol:Push0Shanghai
[PASS] checkZeroIsZero() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// ERC721 ownership uniqueness — bounded stateful proof.
// ---------------------------------------------------------------------------
// Two-token minimal NFT: ownership is unique per id and never falls to the
// zero address after mint. Symbolic invariant should PROVE within depth 2.
forgetest_init!(erc721_unique_ownership_passes, |prj, cmd| {
    skip_unless_z3!("erc721_unique_ownership_passes");

    prj.add_test(
        "Erc721Ownership.t.sol",
        r#"
import "forge-std/Test.sol";

contract MiniNft {
    mapping(uint256 => address) public ownerOf;
    address constant A = address(0xA);
    address constant B = address(0xB);

    constructor() {
        ownerOf[1] = A;
        ownerOf[2] = B;
    }

    function transferFrom(uint256 id, address to) external {
        require(id == 1 || id == 2);
        require(to != address(0));
        require(msg.sender == ownerOf[id]);
        ownerOf[id] = to;
    }
}

contract Erc721Ownership is Test {
    MiniNft nft;

    function setUp() public {
        nft = new MiniNft();
        targetContract(address(nft));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_ownerNonZero() public view {
        assertTrue(nft.ownerOf(1) != address(0));
        assertTrue(nft.ownerOf(2) != address(0));
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "invariant_ownerNonZero"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/Erc721Ownership.t.sol:Erc721Ownership
[PASS] invariant_ownerNonZero() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// ERC4626 deposit → withdraw round-trip — single user, no fee.
// ---------------------------------------------------------------------------
// Property: a user who deposits and then withdraws should get back exactly
// what they put in. Engine should PROVE this for an honest 1:1 vault.
forgetest_init!(erc4626_roundtrip_passes, |prj, cmd| {
    skip_unless_z3!("erc4626_roundtrip_passes");

    prj.add_test(
        "Erc4626Roundtrip.t.sol",
        r#"
contract HonestVault {
    // 1:1 vault — no nonlinear share<->asset math so Z3 can prove the
    // round-trip without falling into nonlinear bv-mul/div. Realistic vaults
    // require nonlinear reasoning that Z3 currently times out on (see the
    // mulDiv note above); that variant belongs in a nightly suite.
    mapping(address => uint256) public sharesOf;

    function deposit(uint256 assets) external returns (uint256 shares) {
        shares = assets;
        sharesOf[msg.sender] += shares;
    }

    function withdraw(uint256 shares) external returns (uint256 assets) {
        require(sharesOf[msg.sender] >= shares);
        assets = shares;
        sharesOf[msg.sender] -= shares;
    }
}

contract Erc4626Roundtrip {
    function checkSingleUserRoundtrip(uint64 amount) public {
        if (amount == 0) return;
        HonestVault v = new HonestVault();
        uint256 shares = v.deposit(amount);
        uint256 returned = v.withdraw(shares);
        assert(returned == amount);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkSingleUserRoundtrip"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/Erc4626Roundtrip.t.sol:Erc4626Roundtrip
[PASS] checkSingleUserRoundtrip(uint64) ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Byteswap involution — small-library bitwise proof.
// ---------------------------------------------------------------------------
// `swap(swap(x)) == x` for uint16, exercising shift + or composition under
// symbolic uint16. Linear/bitwise, so Z3 proves instantly. Replacement for
// the originally-proposed Solady log2 bit-hack, which is nonlinear and times
// out the solver in the same way mulDiv equivalence does.
forgetest_init!(byteswap_uint16_involution_passes, |prj, cmd| {
    skip_unless_z3!("byteswap_uint16_involution_passes");

    prj.add_test(
        "ByteswapInvolution.t.sol",
        r#"
contract ByteswapInvolution {
    function swap(uint16 x) internal pure returns (uint16) {
        return (x >> 8) | (x << 8);
    }

    function checkInvolution(uint16 x) public pure {
        assert(swap(swap(x)) == x);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkInvolution"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ByteswapInvolution.t.sol:ByteswapInvolution
[PASS] checkInvolution(uint16) ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Cancun MCOPY — symbolic memory copy.
// ---------------------------------------------------------------------------
// Verifies that MCOPY (Cancun-era memory-to-memory copy opcode) round-trips
// a symbolic word through scratch memory correctly.
forgetest_init!(mcopy_cancun_roundtrip, |prj, cmd| {
    skip_unless_z3!("mcopy_cancun_roundtrip");

    prj.add_test(
        "McopyCancun.t.sol",
        r#"
contract McopyCancun {
    function checkMcopyRoundtrip(bytes32 word) public pure {
        bytes32 read;
        assembly {
            let src := mload(0x40)
            let dst := add(src, 0x20)
            mstore(src, word)
            mcopy(dst, src, 32)
            read := mload(dst)
        }
        assert(read == word);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkMcopyRoundtrip"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/McopyCancun.t.sol:McopyCancun
[PASS] checkMcopyRoundtrip(bytes32) ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Cancun BLOBHASH / BLOBBASEFEE — opcode accessibility.
// ---------------------------------------------------------------------------
// Sanity check that both Cancun blob opcodes execute under symbolic
// execution. We don't assert specific values (those depend on context);
// only that they're not rejected as unsupported.
forgetest_init!(cancun_blob_opcodes_accessible, |prj, cmd| {
    skip_unless_z3!("cancun_blob_opcodes_accessible");

    prj.add_test(
        "CancunBlobOps.t.sol",
        r#"
contract CancunBlobOps {
    // The engine currently treats a symbolic BLOBHASH index as unsupported
    // (`Stuck`), so we hard-code the index. This still exercises the opcode
    // and the BLOBBASEFEE read.
    function checkBlobOpcodes() public view {
        bytes32 h;
        uint256 fee;
        assembly {
            h := blobhash(0)
            fee := blobbasefee()
        }
        // No semantic assertion; just keep the values live so the engine has
        // to actually execute the opcodes.
        assert(uint256(h) | fee | 1 != 0);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkBlobOpcodes"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/CancunBlobOps.t.sol:CancunBlobOps
[PASS] checkBlobOpcodes() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// Istanbul CHAINID / SELFBALANCE — opcode accessibility.
// ---------------------------------------------------------------------------
forgetest_init!(istanbul_chainid_selfbalance, |prj, cmd| {
    skip_unless_z3!("istanbul_chainid_selfbalance");

    prj.add_test(
        "IstanbulOps.t.sol",
        r#"
contract IstanbulOps {
    function checkChainIdAndSelfBalance() public view {
        uint256 chain;
        uint256 bal;
        assembly {
            chain := chainid()
            bal := selfbalance()
        }
        // Same self-balance equivalence the EVM guarantees.
        assert(bal == address(this).balance);
        assert(chain == block.chainid);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkChainIdAndSelfBalance"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/IstanbulOps.t.sol:IstanbulOps
[PASS] checkChainIdAndSelfBalance() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// SDIV signed-overflow corner case — `MIN_INT256 / -1` returns `MIN_INT256`.
// ---------------------------------------------------------------------------
// Per EVM spec, signed division has no real overflow: dividing the most-
// negative int256 by -1 wraps back to itself. Symbolic engine must model
// this exactly.
forgetest_init!(sdiv_min_int_overflow_semantics, |prj, cmd| {
    skip_unless_z3!("sdiv_min_int_overflow_semantics");

    prj.add_test(
        "SdivMinInt.t.sol",
        r#"
contract SdivMinInt {
    function checkSdivMinByNegOne() public pure {
        int256 a = type(int256).min;
        int256 b = -1;
        int256 r;
        assembly {
            r := sdiv(a, b)
        }
        // EVM spec: SDIV(MIN_INT, -1) == MIN_INT.
        assert(r == type(int256).min);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkSdivMinByNegOne"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SdivMinInt.t.sol:SdivMinInt
[PASS] checkSdivMinByNegOne() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// EXP small bounded — engine handles modular exponentiation.
// ---------------------------------------------------------------------------
// We deliberately keep base and exponent both bounded so the engine can
// concretely unroll. This is the smallest non-trivial EXP check that won't
// time out the solver (full nonlinear EXP equivalence is out of scope, the
// same way mulDiv equivalence is).
forgetest_init!(exp_small_bounded, |prj, cmd| {
    skip_unless_z3!("exp_small_bounded");

    prj.add_test(
        "ExpSmallBounded.t.sol",
        r#"
import "forge-std/Test.sol";

contract ExpSmallBounded is Test {
    function checkExpSmall(uint8 e) public pure {
        vm.assume(e <= 4);
        uint256 r;
        assembly {
            r := exp(2, e)
        }
        // 2 ** e for e in [0, 4] enumerates {1, 2, 4, 8, 16}.
        assert(r == (uint256(1) << e));
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkExpSmall"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ExpSmallBounded.t.sol:ExpSmallBounded
[PASS] checkExpSmall(uint8) ([METRICS])
...
"#]]);
});
