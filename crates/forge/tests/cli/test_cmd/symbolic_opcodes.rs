use super::symbolic_helpers::assert_relevant_lines;
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, util::OutputExt};

use super::symbolic_helpers::z3_available;
use crate::skip_unless_z3;

forgetest_init!(symbolic_opcode_byte_and_signextend_accept_symbolic_index, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_opcode_byte_and_signextend_accept_symbolic_index because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicByteSignextend.t.sol",
        r#"
contract SymbolicByteSignextend {
    function checkSymbolicByteIndex(uint8 idx) public pure {
        bytes32 word = hex"000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        uint256 got;
        assembly {
            got := byte(idx, word)
        }
        if (idx < 32) {
            assert(got == idx);
        } else {
            assert(got == 0);
        }
    }

    function checkSymbolicSignextendIndex(uint8 idx) public pure {
        uint256 value = 0x80;
        uint256 got;
        assembly {
            got := signextend(idx, value)
        }
        if (idx == 0) {
            assert(got == type(uint256).max - 0x7f);
        } else {
            assert(got == 0x80);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolic"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicByteIndex(uint8)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicSignextendIndex(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic BYTE index"), "{stdout}");
    assert!(!stdout.contains("symbolic SIGNEXTEND index"), "{stdout}");
});

forgetest_init!(symbolic_shift_opcodes_accept_symbolic_amount, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_shift_opcodes_accept_symbolic_amount because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicShift.t.sol",
        r#"
contract SymbolicShift {
    function checkSymbolicShiftAmount(uint16 shift) public pure {
        uint256 left;
        uint256 right;
        uint256 signed;
        assembly {
            left := shl(shift, 1)
            right := shr(shift, shl(255, 1))
            signed := sar(shift, not(0))
        }

        if (shift == 5) {
            assert(left == 32);
        }
        if (shift >= 256) {
            assert(left == 0);
            assert(right == 0);
            assert(signed == type(uint256).max);
        }
        if (shift == 255) {
            assert(right == 1);
            assert(signed == type(uint256).max);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicShiftAmount"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicShiftAmount(uint16)
"#]],
    );
    assert!(!stdout.contains("symbolic shift amount"), "{stdout}");
});

forgetest_init!(symbolic_exp_accepts_larger_bounded_symbolic_base, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_exp_accepts_larger_bounded_symbolic_base because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExp.t.sol",
        r#"
contract SymbolicExp {
    function checkSymbolicExpBase(uint8 x) public pure {
        uint256 y = uint256(x) ** 16;
        if (x == 2) {
            assert(y == 65536);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicExpBase"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicExpBase(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic EXP base"), "{stdout}");
});

forgetest_init!(symbolic_exp_accepts_bounded_symbolic_exponent, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_exp_accepts_bounded_symbolic_exponent because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpExponent.t.sol",
        r#"
contract SymbolicExpExponent {
    function checkSymbolicExpExponent(uint8 raw) public pure {
        uint256 exponent = uint256(raw) & 7;
        uint256 y = uint256(3) ** exponent;
        if (exponent == 5) {
            assert(y == 243);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicExpExponent"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicExpExponent(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic EXP exponent"), "{stdout}");
});

forgetest_init!(symbolic_exp_accepts_wider_symbolic_exponent_for_concrete_base, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_exp_accepts_wider_symbolic_exponent_for_concrete_base because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpWideExponent.t.sol",
        r#"
contract SymbolicExpWideExponent {
    function checkSymbolicExpWideExponent(uint8 raw) public pure {
        uint256 exponent = uint256(raw) & 63;
        uint256 y = uint256(2) ** exponent;
        if (exponent == 40) {
            assert(y == 1099511627776);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicExpWideExponent"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicExpWideExponent(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic EXP exponent"), "{stdout}");
});

// The engine does not model gas consumption, so `GAS` / `gasleft()` must fail
// closed instead of returning a concrete max value or a symbolic approximation
// that can produce non-replaying counterexamples.
forgetest_init!(symbolic_gasleft_reports_unsupported, |prj, cmd| {
    skip_unless_z3!("symbolic_gasleft_reports_unsupported");

    prj.add_test(
        "SymbolicGasLeftBound.t.sol",
        r#"
contract SymbolicGasLeftBound {
    function checkGasLeftIsBounded() public view {
        assert(gasleft() > 2 ** 200);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkGasLeftIsBounded"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Stuck): unsupported symbolic execution feature: GAS/gasleft() not modeled
"#]],
    );
});

forgetest_init!(symbolic_gas_can_be_used_as_call_operand, |prj, cmd| {
    skip_unless_z3!("symbolic_gas_can_be_used_as_call_operand");

    prj.add_test(
        "SymbolicGasCallOperand.t.sol",
        r#"
contract SymbolicGasCallOperandTarget {
    function ping(uint256 value) external pure returns (uint256) {
        return value + 1;
    }
}

contract SymbolicGasCallOperand {
    SymbolicGasCallOperandTarget target;

    function setUp() public {
        target = new SymbolicGasCallOperandTarget();
    }

    function checkGasOnlyFeedsCall(uint128 raw) public view {
        uint256 value = uint256(raw);
        bytes4 selector = SymbolicGasCallOperandTarget.ping.selector;
        address targetAddress = address(target);
        bool ok;
        uint256 out;
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, selector)
            mstore(add(ptr, 4), value)
            ok := staticcall(gas(), targetAddress, ptr, 36, ptr, 32)
            out := mload(ptr)
        }

        assert(ok);
        assert(out == value + 1);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkGasOnlyFeedsCall"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkGasOnlyFeedsCall(uint128)
"#]],
    );
    assert!(!stdout.contains("GAS/gasleft() not modeled"), "{stdout}");
});

forgetest_init!(symbolic_gas_derived_call_operand_reports_unsupported, |prj, cmd| {
    skip_unless_z3!("symbolic_gas_derived_call_operand_reports_unsupported");

    prj.add_test(
        "SymbolicDerivedGasCallOperand.t.sol",
        r#"
contract SymbolicDerivedGasCallOperandTarget {
    function ping() external pure returns (uint256) {
        return 1;
    }
}

contract SymbolicDerivedGasCallOperand {
    SymbolicDerivedGasCallOperandTarget target;

    function setUp() public {
        target = new SymbolicDerivedGasCallOperandTarget();
    }

    function checkDerivedGasCallOperand() public view {
        bytes4 selector = SymbolicDerivedGasCallOperandTarget.ping.selector;
        address targetAddress = address(target);
        bool ok;
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, selector)
            ok := staticcall(sub(gas(), 1), targetAddress, ptr, 4, ptr, 32)
        }
        assert(ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkDerivedGasCallOperand"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Stuck): unsupported symbolic execution feature: GAS/gasleft() not modeled
"#]],
    );
});

forgetest_init!(symbolic_gas_in_call_calldata_reports_unsupported, |prj, cmd| {
    skip_unless_z3!("symbolic_gas_in_call_calldata_reports_unsupported");

    prj.add_test(
        "SymbolicGasCallData.t.sol",
        r#"
contract SymbolicGasCallDataTarget {
    fallback() external {}
}

contract SymbolicGasCallData {
    SymbolicGasCallDataTarget target;

    function setUp() public {
        target = new SymbolicGasCallDataTarget();
    }

    function checkGasInCallData() public view {
        address targetAddress = address(target);
        bool ok;
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, gas())
            ok := staticcall(gas(), targetAddress, ptr, 32, 0, 0)
        }
        assert(ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkGasInCallData"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Stuck): unsupported symbolic execution feature: GAS/gasleft() not modeled
"#]],
    );
});

forgetest_init!(symbolic_gas_as_call_target_reports_unsupported, |prj, cmd| {
    skip_unless_z3!("symbolic_gas_as_call_target_reports_unsupported");

    prj.add_test(
        "SymbolicGasCallTarget.t.sol",
        r#"
contract SymbolicGasCallTarget {
    function checkGasAsCallTarget() public view {
        bool ok;
        assembly {
            ok := staticcall(gas(), gas(), 0, 0, 0, 0)
        }
        assert(ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkGasAsCallTarget"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Stuck): unsupported symbolic execution feature: GAS/gasleft() not modeled
"#]],
    );
});

forgetest_init!(symbolic_gas_as_call_input_bounds_reports_unsupported, |prj, cmd| {
    skip_unless_z3!("symbolic_gas_as_call_input_bounds_reports_unsupported");

    prj.add_test(
        "SymbolicGasCallInputBounds.t.sol",
        r#"
contract SymbolicGasCallInputBoundsTarget {
    fallback() external {}
}

contract SymbolicGasCallInputBounds {
    SymbolicGasCallInputBoundsTarget target;

    function setUp() public {
        target = new SymbolicGasCallInputBoundsTarget();
    }

    function checkGasAsCallInputOffset() public view {
        address targetAddress = address(target);
        bool ok;
        assembly {
            ok := staticcall(gas(), targetAddress, gas(), 0, 0, 0)
        }
        assert(ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkGasAsCallInputOffset"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Stuck): unsupported symbolic execution feature: GAS/gasleft() not modeled
"#]],
    );
});

// Plan-compliant target behavior for the `GAS` / `gasleft()` opcode: any
// symbolic path that branches on `gasleft()` should taint the result as
// Incomplete (Unsupported), because gas is not modeled symbolically and a
// bounded-symbolic approximation produces non-replaying counterexamples.
//
// The contract below has two branches gated by `gasleft()`:
//   - the `low gas` branch is concretely unreachable under any normal forge transaction gas limit;
//   - the `high gas` branch is the only one a concrete replay can ever take.
//
// A correct symbolic engine should refuse to draw conclusions: rather than
// PASS (if it silently treated `gasleft` as always high) or FAIL with a
// non-replaying counterexample (if it lets Z3 pick `gasleft = 50`), it should
// emit a `[FAIL: incomplete symbolic execution (Stuck): unsupported symbolic
// execution feature: GAS/gasleft() not modeled]` result.
forgetest_init!(symbolic_gasleft_branch_reports_unsupported, |prj, cmd| {
    skip_unless_z3!("symbolic_gasleft_branch_reports_unsupported");

    prj.add_test(
        "SymbolicGasLeftIncomplete.t.sol",
        r#"
contract SymbolicGasLeftIncomplete {
    // The `gasleft() < 100` branch is concretely unreachable under any
    // normal forge transaction gas limit. A correct symbolic engine that
    // does not model gas must not let Z3 pick `gasleft = 50`, take this
    // branch, and report a counterexample that will never replay; the
    // result should taint as Incomplete instead.
    function checkGasGuardedBranch(uint256 input) public view {
        if (gasleft() < 100) {
            assert(input != 0xdead);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkGasGuardedBranch"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Stuck): unsupported symbolic execution feature: GAS/gasleft() not modeled
"#]],
    );
});

// Plan-compliant target behavior for the symbolic Keccak heuristic: any
// result reached on a path whose proof obligation reduces to a Keccak
// property (e.g. `keccak(x) != 0` by uninterpreted-output assumption)
// must surface explicit Keccak/SHA3 vocabulary — either tainted as
// Incomplete or carrying a user-facing warning — because the engine does
// not model SHA3 collision resistance as a real cryptographic proof.
forgetest_init!(symbolic_keccak_dependent_safe_result_must_taint_incomplete, |prj, cmd| {
    skip_unless_z3!("symbolic_keccak_dependent_safe_result_must_taint_incomplete");

    prj.add_test(
        "SymbolicKeccakHeuristic.t.sol",
        r#"
contract SymbolicKeccakHeuristic {
    // The engine treats keccak as an uninterpreted function whose output
    // is never zero by construction, so this assertion holds symbolically.
    // That is a modeling assumption, not a cryptographic proof — the SAFE
    // verdict must surface as Incomplete or carry a Keccak warning.
    function checkKeccakNeverZero(uint256 x) public pure {
        assert(keccak256(abi.encodePacked(x)) != bytes32(0));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkKeccakNeverZero"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Error): solver error: solver model does not satisfy path constraints involving symbolic Keccak heuristic
"#]],
    );

    // Require explicit Keccak/SHA3 vocabulary in the result line; a bare
    // "incomplete" for unrelated reasons (e.g. solver-error) does not
    // satisfy the plan's "heuristic paths are visible, not silently
    // presented as proof-grade" acceptance criterion.
    let lowered = stdout.to_lowercase();
    let has_keccak_signal = lowered.contains("keccak heuristic")
        || lowered.contains("sha3 heuristic")
        || lowered.contains("symbolic keccak")
        || lowered.contains("symbolic sha3")
        || lowered.contains("heuristic keccak");
    assert!(
        has_keccak_signal,
        "expected Keccak-heuristic taint or warning in output, got:\n{stdout}"
    );
});
