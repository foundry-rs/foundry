use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, util::OutputExt};

use super::symbolic_helpers::z3_available;

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

    assert!(stdout.contains("[PASS] checkSymbolicByteIndex(uint8)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkSymbolicSignextendIndex(uint8)"), "{stdout}");
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

    assert!(stdout.contains("[PASS] checkSymbolicShiftAmount(uint16)"), "{stdout}");
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

    assert!(stdout.contains("[PASS] checkSymbolicExpBase(uint8)"), "{stdout}");
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

    assert!(stdout.contains("[PASS] checkSymbolicExpExponent(uint8)"), "{stdout}");
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

    assert!(stdout.contains("[PASS] checkSymbolicExpWideExponent(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic EXP exponent"), "{stdout}");
});
