use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, str, util::OutputExt};
use std::process::Command;

use super::symbolic_helpers::assert_symbolic;

fn z3_available() -> bool {
    Command::new("z3").arg("--version").output().is_ok_and(|output| output.status.success())
}

forgetest_init!(symbolic_tests_are_ignored_without_flag, |prj, cmd| {
    prj.add_test(
        "SymbolicIgnored.t.sol",
        r#"
contract SymbolicIgnored {
    function checkWouldFail(uint256 x) public pure {
        assert(x != 42);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--match-test", "checkWouldFail"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("No tests found") || stdout.contains("0 tests"));
});

forgetest_init!(symbolic_passes_scalar_test, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_passes_scalar_test because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicPass.t.sol",
        r#"
contract SymbolicPass {
    function checkNoop(uint256) public pure {}
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkNoop"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkNoop(uint256)"));
    assert!(stdout.contains("(paths:"));
});

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

forgetest_init!(symbolic_calldataload_accepts_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_calldataload_accepts_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCalldataLoad.t.sol",
        r#"
contract SymbolicCalldataLoad {
    function checkSymbolicCalldataLoad(uint16 offset, uint256 marker) public pure {
        uint256 loaded;
        assembly {
            loaded := calldataload(offset)
        }

        if (offset == 36) {
            assert(loaded == marker);
        }
        if (offset >= msg.data.length) {
            assert(loaded == 0);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCalldataLoad"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicCalldataLoad(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CALLDATALOAD offset"), "{stdout}");
});

forgetest_init!(symbolic_calldatacopy_accepts_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_calldatacopy_accepts_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCalldataCopy.t.sol",
        r#"
contract SymbolicCalldataCopy {
    function checkSymbolicCalldataCopy(uint16 offset, uint256 marker) public pure {
        uint256 copied;
        assembly {
            calldatacopy(0, offset, 32)
            copied := mload(0)
        }

        if (offset == 36) {
            assert(copied == marker);
        }
        if (offset >= msg.data.length) {
            assert(copied == 0);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCalldataCopy"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicCalldataCopy(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CALLDATACOPY offset"), "{stdout}");
});

forgetest_init!(symbolic_calldatacopy_accepts_symbolic_dest, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_calldatacopy_accepts_symbolic_dest because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCalldataCopyDest.t.sol",
        r#"
contract SymbolicCalldataCopyDest {
    function checkSymbolicCalldataCopyDest(uint16 dest, uint256 marker) public pure {
        uint256 copied;
        assembly {
            calldatacopy(dest, 36, 32)
            copied := mload(0x80)
        }

        if (dest == 0x80) {
            assert(copied == marker);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCalldataCopyDest"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicCalldataCopyDest(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CALLDATACOPY dest"), "{stdout}");
});

forgetest_init!(symbolic_calldatacopy_accepts_bounded_symbolic_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_calldatacopy_accepts_bounded_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCalldataCopySize.t.sol",
        r#"
contract SymbolicCalldataCopySize {
    function checkSymbolicCalldataCopySize(uint8 n) public pure {
        uint256 size = uint256(n & 3);
        bytes memory copied = hex"aaaaaaaa";
        bytes4 selector = bytes4(keccak256("checkSymbolicCalldataCopySize(uint8)"));

        assembly {
            calldatacopy(add(copied, 0x20), 0, size)
        }

        if (size == 0) assert(copied[0] == bytes1(0xaa));
        if (size > 0) assert(copied[0] == selector[0]);
        if (size <= 1) assert(copied[1] == bytes1(0xaa));
        if (size > 1) assert(copied[1] == selector[1]);
        if (size <= 2) assert(copied[2] == bytes1(0xaa));
        if (size > 2) assert(copied[2] == selector[2]);
        assert(copied[3] == bytes1(0xaa));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCalldataCopySize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicCalldataCopySize(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic CALLDATACOPY size"), "{stdout}");
});

forgetest_init!(symbolic_calldatacopy_accepts_symbolic_dest_and_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_calldatacopy_accepts_symbolic_dest_and_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCalldataCopyDestAndSize.t.sol",
        r#"
contract SymbolicCalldataCopyDestAndSize {
    function checkCalldataCopyDestAndSize(uint8 rawDest, uint8 rawSize, bytes32 marker) public {
        uint256 dest = 0x80 + uint256(rawDest);
        uint256 size = uint256(rawSize);
        require(size <= 32);

        bytes32 copied;
        assembly {
            calldatacopy(dest, 68, size)
            copied := mload(0xa0)
        }

        if (dest == 0xa0 && size == 32) {
            assert(copied == marker);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCalldataCopyDestAndSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(
        stdout.contains("[PASS] checkCalldataCopyDestAndSize(uint8,uint8,bytes32)"),
        "{stdout}"
    );
    assert!(!stdout.contains("symbolic CALLDATACOPY dest"), "{stdout}");
    assert!(!stdout.contains("symbolic CALLDATACOPY size"), "{stdout}");
});

forgetest_init!(symbolic_mload_accepts_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_mload_accepts_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMload.t.sol",
        r#"
contract SymbolicMload {
    function checkSymbolicMload(uint16 offset, uint256 marker) public pure {
        uint256 loaded;
        assembly {
            mstore(0x80, marker)
            loaded := mload(offset)
        }

        if (offset == 0x80) {
            assert(loaded == marker);
        }
        if (offset >= 0xa0) {
            assert(loaded == 0);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicMload"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicMload(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic MLOAD offset"), "{stdout}");
});

forgetest_init!(symbolic_mstore_accepts_constrained_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_mstore_accepts_constrained_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMstoreConstrained.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicMstoreConstrained is Test {
    function checkConstrainedMstore(uint16 offset, uint256 marker) public {
        vm.assume(offset == 0x80);

        uint256 loaded;
        assembly {
            mstore(offset, marker)
            loaded := mload(0x80)
        }

        assertEq(loaded, marker);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkConstrainedMstore"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkConstrainedMstore(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic MSTORE offset"), "{stdout}");
});

forgetest_init!(symbolic_mstore_accepts_unconstrained_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_mstore_accepts_unconstrained_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMstoreUnconstrained.t.sol",
        r#"
contract SymbolicMstoreUnconstrained {
    function checkSymbolicMstore(uint16 offset, uint256 marker) public pure {
        uint256 loaded;
        assembly {
            mstore(offset, marker)
            loaded := mload(0x80)
        }

        assert(loaded != 0x42);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicMstore"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL: panic: assertion failed"), "{stdout}");
    assert!(!stdout.contains("symbolic MSTORE offset"), "{stdout}");
});

forgetest_init!(symbolic_mstore8_accepts_unconstrained_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_mstore8_accepts_unconstrained_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMstore8Unconstrained.t.sol",
        r#"
contract SymbolicMstore8Unconstrained {
    function checkSymbolicMstore8(uint16 offset, uint256 marker) public pure {
        uint256 loaded;
        assembly {
            mstore8(offset, marker)
            loaded := byte(0, mload(0x80))
        }

        assert(loaded != 0xab);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicMstore8"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL: panic: assertion failed"), "{stdout}");
    assert!(!stdout.contains("symbolic MSTORE8 offset"), "{stdout}");
});

forgetest_init!(symbolic_msize_after_symbolic_write_is_modeled, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_msize_after_symbolic_write_is_modeled because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMsizeAfterWrite.t.sol",
        r#"
contract SymbolicMsizeAfterWrite {
    function checkSymbolicMsize(uint16 offset, uint256 marker) public pure {
        uint256 size;
        assembly {
            mstore(offset, marker)
            size := msize()
        }

        assert(size != 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicMsize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicMsize(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic MSIZE after symbolic memory write"), "{stdout}");
});

forgetest_init!(symbolic_sha3_accepts_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_sha3_accepts_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSha3.t.sol",
        r#"
contract SymbolicSha3 {
    function checkSymbolicSha3(uint16 offset, uint256 marker) public pure {
        bytes32 digest;
        bytes32 expected;
        assembly {
            mstore(0x80, marker)
            digest := keccak256(offset, 32)
            expected := keccak256(0x80, 32)
        }

        if (offset == 0x80) {
            assert(digest == expected);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicSha3"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicSha3(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic SHA3 offset"), "{stdout}");
});

forgetest_init!(symbolic_sha3_accepts_constrained_symbolic_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_sha3_accepts_constrained_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSha3ConstrainedSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicSha3ConstrainedSize is Test {
    function checkConstrainedSha3Size(uint16 size, uint256 marker) public {
        vm.assume(size == 32);

        bytes32 digest;
        bytes32 expected;
        assembly {
            mstore(0x80, marker)
            digest := keccak256(0x80, size)
            expected := keccak256(0x80, 32)
        }

        assertEq(digest, expected);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkConstrainedSha3Size"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkConstrainedSha3Size(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic SHA3 size"), "{stdout}");
});

forgetest_init!(symbolic_sha3_accepts_bounded_symbolic_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_sha3_accepts_bounded_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSha3BoundedSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicSha3BoundedSize is Test {
    function checkBoundedSha3Size(uint8 rawSize, uint256 marker) public {
        uint256 size = uint256(rawSize & 32);

        bytes32 digest;
        bytes32 sameDigest;
        assembly {
            mstore(0x80, marker)
            digest := keccak256(0x80, size)
            sameDigest := keccak256(0x80, size)
        }

        assertEq(digest, sameDigest);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkBoundedSha3Size"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkBoundedSha3Size(uint8,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic SHA3 size"), "{stdout}");
});

forgetest_init!(symbolic_log_accepts_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_log_accepts_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicLogOffset.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicLogOffset is Test {
    function checkSymbolicLogOffset(uint16 offset, uint256 marker) public {
        vm.recordLogs();
        assembly {
            mstore(0x80, marker)
            log1(offset, 32, 0x1234)
        }

        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertEq(logs.length, 1);
        if (offset == 0x80) {
            assertEq(abi.decode(logs[0].data, (uint256)), marker);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicLogOffset"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicLogOffset(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic LOG offset"), "{stdout}");
});

forgetest_init!(symbolic_log_accepts_bounded_symbolic_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_log_accepts_bounded_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicLogSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicLogSize is Test {
    function checkSymbolicLogSize(uint8 rawSize) public {
        uint256 size = uint256(rawSize & 3);

        assembly {
            mstore(0x80, shl(232, 0x010203))
            log1(0x80, size, 0x1234)
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicLogSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicLogSize(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic LOG size"), "{stdout}");
});

forgetest_init!(symbolic_returndatacopy_accepts_constrained_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_returndatacopy_accepts_constrained_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicReturndataCopyConstrained.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicReturndataCopyHelper {
    function pair(uint256 marker) external pure returns (uint256, uint256) {
        return (11, marker);
    }
}

contract SymbolicReturndataCopyConstrained is Test {
    SymbolicReturndataCopyHelper helper;

    function setUp() public {
        helper = new SymbolicReturndataCopyHelper();
    }

    function checkConstrainedReturndataCopy(uint16 offset, uint256 marker) public {
        vm.assume(offset == 32);

        bytes4 selector = SymbolicReturndataCopyHelper.pair.selector;
        address target = address(helper);
        bool ok;
        uint256 copied;
        assembly {
            mstore(0x80, selector)
            mstore(0x84, marker)
            ok := staticcall(gas(), target, 0x80, 36, 0, 0)
            returndatacopy(0xa0, offset, 32)
            copied := mload(0xa0)
        }

        assertTrue(ok);
        assertEq(copied, marker);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkConstrainedReturndataCopy"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkConstrainedReturndataCopy(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic RETURNDATACOPY offset"), "{stdout}");
});

forgetest_init!(symbolic_returndatacopy_accepts_bounded_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_returndatacopy_accepts_bounded_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicReturndataCopyOffset.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicReturndataCopyOffsetHelper {
    function pair(uint256 marker) external pure returns (uint256, uint256) {
        return (11, marker);
    }
}

contract SymbolicReturndataCopyOffset is Test {
    SymbolicReturndataCopyOffsetHelper helper;

    function setUp() public {
        helper = new SymbolicReturndataCopyOffsetHelper();
    }

    function checkSymbolicReturndataCopyOffset(uint8 rawOffset, uint256 marker) public {
        uint256 offset = uint256(rawOffset);
        vm.assume(offset <= 32);

        bytes4 selector = SymbolicReturndataCopyOffsetHelper.pair.selector;
        address target = address(helper);
        bool ok;
        uint256 copied;
        assembly {
            mstore(0x80, selector)
            mstore(0x84, marker)
            ok := staticcall(gas(), target, 0x80, 36, 0, 0)
            returndatacopy(0xa0, offset, 32)
            copied := mload(0xa0)
        }

        assertTrue(ok);
        if (offset == 32) {
            assertEq(copied, marker);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicReturndataCopyOffset"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicReturndataCopyOffset(uint8,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic RETURNDATACOPY offset"), "{stdout}");
});

forgetest_init!(symbolic_returndatacopy_accepts_symbolic_dest, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_returndatacopy_accepts_symbolic_dest because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicReturndataCopyDest.t.sol",
        r#"
contract SymbolicReturndataCopyDestHelper {
    function echo(uint256 marker) external pure returns (uint256) {
        return marker;
    }
}

contract SymbolicReturndataCopyDest {
    SymbolicReturndataCopyDestHelper helper = new SymbolicReturndataCopyDestHelper();

    function checkSymbolicReturndataCopyDest(uint16 dest, uint256 marker) public {
        (bool ok,) = address(helper).call(
            abi.encodeWithSelector(SymbolicReturndataCopyDestHelper.echo.selector, marker)
        );
        require(ok);

        uint256 copied;
        assembly {
            returndatacopy(dest, 0, 32)
            copied := mload(0x80)
        }

        if (dest == 0x80) {
            assert(copied == marker);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicReturndataCopyDest"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicReturndataCopyDest(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic RETURNDATACOPY dest"), "{stdout}");
});

forgetest_init!(symbolic_returndatacopy_accepts_bounded_symbolic_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_returndatacopy_accepts_bounded_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicReturndataCopySize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicReturndataCopySizeHelper {
    function pair(uint256 marker) external pure returns (uint256, uint256) {
        return (11, marker);
    }
}

contract SymbolicReturndataCopySize is Test {
    SymbolicReturndataCopySizeHelper helper;

    function setUp() public {
        helper = new SymbolicReturndataCopySizeHelper();
    }

    function checkSymbolicReturndataCopySize(uint8 rawSize, uint256 marker) public {
        uint256 size = uint256(rawSize);
        vm.assume(size <= 32);

        bytes4 selector = SymbolicReturndataCopySizeHelper.pair.selector;
        address target = address(helper);
        bool ok;
        uint256 copied;
        assembly {
            mstore(0x80, selector)
            mstore(0x84, marker)
            ok := staticcall(gas(), target, 0x80, 36, 0, 0)
            returndatacopy(0xa0, 32, size)
            copied := mload(0xa0)
        }

        assertTrue(ok);
        if (size == 32) {
            assertEq(copied, marker);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicReturndataCopySize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicReturndataCopySize(uint8,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic RETURNDATACOPY size"), "{stdout}");
});

forgetest_init!(symbolic_return_revert_accept_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_return_revert_accept_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicReturnRevertOffset.t.sol",
        r#"
contract SymbolicReturnRevertHelper {
    function ret(uint16 offset, uint256 marker) external pure returns (uint256) {
        assembly {
            mstore(0x80, marker)
            return(offset, 32)
        }
    }

    function rev(uint16 offset, uint256 marker) external pure {
        assembly {
            mstore(0x80, marker)
            revert(offset, 32)
        }
    }
}

contract SymbolicReturnRevertOffset {
    SymbolicReturnRevertHelper helper;

    function setUp() public {
        helper = new SymbolicReturnRevertHelper();
    }

    function checkSymbolicReturnOffset(uint16 offset, uint256 marker) public view {
        uint256 value = helper.ret(offset, marker);
        if (offset == 0x80) {
            assert(value == marker);
        }
    }

    function checkSymbolicRevertOffset(uint16 offset, uint256 marker) public view {
        (bool ok, bytes memory data) =
            address(helper).staticcall(abi.encodeCall(SymbolicReturnRevertHelper.rev, (offset, marker)));
        assert(!ok);
        if (offset == 0x80) {
            assert(abi.decode(data, (uint256)) == marker);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicReturnRevertOffset"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicReturnOffset(uint16,uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkSymbolicRevertOffset(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic RETURN offset"), "{stdout}");
    assert!(!stdout.contains("symbolic REVERT offset"), "{stdout}");
});

forgetest_init!(symbolic_mcopy_accepts_symbolic_source_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_mcopy_accepts_symbolic_source_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMcopy.t.sol",
        r#"
contract SymbolicMcopy {
    function checkSymbolicMcopy(uint16 src, uint256 marker) public pure {
        uint256 copied;
        assembly {
            mstore(0x80, marker)
            mcopy(0, src, 32)
            copied := mload(0)
        }

        if (src == 0x80) {
            assert(copied == marker);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicMcopy"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicMcopy(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic MCOPY src"), "{stdout}");
});

forgetest_init!(symbolic_call_accepts_symbolic_input_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_call_accepts_symbolic_input_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCallInputOffset.t.sol",
        r#"
contract SymbolicCallInputOffsetHelper {
    function echo(uint256 value) external pure returns (uint256) {
        return value;
    }
}

contract SymbolicCallInputOffset {
    SymbolicCallInputOffsetHelper helper;

    function setUp() public {
        helper = new SymbolicCallInputOffsetHelper();
    }

    function checkSymbolicCallInputOffset(uint16 offset, uint256 marker) public view {
        bytes4 selector = SymbolicCallInputOffsetHelper.echo.selector;
        bool ok;
        uint256 out;
        address target = address(helper);
        assembly {
            mstore(0x80, selector)
            mstore(0x84, marker)
            ok := staticcall(gas(), target, offset, 36, 0, 32)
            out := mload(0)
        }

        if (offset == 0x80) {
            assert(ok);
            assert(out == marker);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCallInputOffset"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicCallInputOffset(uint16,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CALL input offset"), "{stdout}");
});

forgetest_init!(symbolic_call_accepts_bounded_symbolic_output_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_call_accepts_bounded_symbolic_output_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCallOutputSize.t.sol",
        r#"
contract SymbolicCallOutputSizeHelper {
    function marker() external pure returns (uint256) {
        return 0x1234;
    }
}

contract SymbolicCallOutputSize {
    SymbolicCallOutputSizeHelper helper;

    function setUp() public {
        helper = new SymbolicCallOutputSizeHelper();
    }

    function checkSymbolicCallOutputSize(uint8 rawSize) public view {
        bytes4 selector = SymbolicCallOutputSizeHelper.marker.selector;
        uint256 size = uint256(rawSize & 32);
        bool ok;
        uint256 out;
        address target = address(helper);
        assembly {
            mstore(0x80, selector)
            ok := staticcall(gas(), target, 0x80, 4, 0xa0, size)
            out := mload(0xa0)
        }

        assert(ok);
        if (size == 32) {
            assert(out == 0x1234);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCallOutputSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicCallOutputSize(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic CALL output size"), "{stdout}");
});

forgetest_init!(symbolic_call_accepts_bounded_symbolic_input_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_call_accepts_bounded_symbolic_input_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCallInputSize.t.sol",
        r#"
contract SymbolicCallInputSizeHelper {
    fallback() external {
        assembly {
            mstore(0, calldatasize())
            return(0, 32)
        }
    }
}

contract SymbolicCallInputSize {
    SymbolicCallInputSizeHelper helper;

    function setUp() public {
        helper = new SymbolicCallInputSizeHelper();
    }

    function checkSymbolicCallInputSize(uint8 rawSize, uint256 marker) public view {
        uint256 size = uint256(rawSize & 32);
        bool ok;
        uint256 out;
        address target = address(helper);
        assembly {
            mstore(0x80, marker)
            ok := staticcall(gas(), target, 0x80, size, 0xa0, 32)
            out := mload(0xa0)
        }

        assert(ok);
        assert(out == size);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCallInputSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicCallInputSize(uint8,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CALL input size"), "{stdout}");
});

forgetest_init!(symbolic_return_accepts_bounded_symbolic_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_return_accepts_bounded_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicReturnSize.t.sol",
        r#"
contract SymbolicReturnSizeHelper {
    fallback() external {
        assembly {
            calldatacopy(0x00, 0x00, calldatasize())
            return(0x00, calldatasize())
        }
    }
}

contract SymbolicReturnSize {
    SymbolicReturnSizeHelper helper;

    function setUp() public {
        helper = new SymbolicReturnSizeHelper();
    }

    function checkSymbolicReturnSize(uint8 rawSize, uint256 marker) public view {
        uint256 size = uint256(rawSize & 32);
        bool ok;
        uint256 returnedSize;
        address target = address(helper);
        assembly {
            mstore(0x80, marker)
            ok := staticcall(gas(), target, 0x80, size, 0x00, 0x00)
            returnedSize := returndatasize()
        }

        assert(ok);
        assert(returnedSize == size);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicReturnSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicReturnSize(uint8,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic RETURN size"), "{stdout}");
});

forgetest_init!(symbolic_revert_accepts_bounded_symbolic_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_revert_accepts_bounded_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRevertSize.t.sol",
        r#"
contract SymbolicRevertSizeHelper {
    fallback() external {
        assembly {
            calldatacopy(0x00, 0x00, calldatasize())
            revert(0x00, calldatasize())
        }
    }
}

contract SymbolicRevertSize {
    SymbolicRevertSizeHelper helper;

    function setUp() public {
        helper = new SymbolicRevertSizeHelper();
    }

    function checkSymbolicRevertSize(uint8 rawSize, uint256 marker) public view {
        uint256 size = uint256(rawSize & 32);
        bool ok;
        uint256 returnedSize;
        address target = address(helper);
        assembly {
            mstore(0x80, marker)
            ok := staticcall(gas(), target, 0x80, size, 0x00, 0x00)
            returnedSize := returndatasize()
        }

        assert(!ok);
        assert(returnedSize == size);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicRevertSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicRevertSize(uint8,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic REVERT size"), "{stdout}");
});

forgetest_init!(symbolic_loop_bound_limits_symbolic_unrolling, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_loop_bound_limits_symbolic_unrolling because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicLoopBound.t.sol",
        r#"
contract SymbolicLoopBound {
    /// forge-config: default.symbolic.loop = 2
    function checkLoopBound(uint8 n) public pure {
        uint256 i;
        while (i < n) {
            ++i;
        }
        assert(i <= 2);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkLoopBound"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkLoopBound(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic depth limit exceeded"), "{stdout}");
});

forgetest_init!(symbolic_finds_assert_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_finds_assert_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicAssert.t.sol",
        r#"
contract SymbolicAssert {
    function checkRejectsFortyTwo(uint256 x) public pure {
        assert(x != 42);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkRejectsFortyTwo"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"));
    assert!(stdout.contains("panic: assertion failed"));
    assert!(stdout.contains("checkRejectsFortyTwo(uint256)"));
    assert!(stdout.contains("args=[42]"));
});

forgetest_init!(symbolic_finds_wrapping_arithmetic_riddle_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_finds_wrapping_arithmetic_riddle_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRiddle.t.sol",
        r#"
contract SymbolicRiddle {
    function check_riddle(uint256 x) external pure {
        uint256 msgSender = uint160(0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38);

        unchecked {
            require(x * x < msgSender);
        }

        require(x > msgSender);
        require(x & 0x800 != 0);
        require(x & 0x10000 == 0);

        assert(false);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "check_riddle"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("panic: assertion failed"), "{stdout}");
    assert!(stdout.contains("check_riddle(uint256)"), "{stdout}");
    assert!(!stdout.contains("unsupported symbolic execution feature"), "{stdout}");
});

forgetest_init!(symbolic_ignores_plain_require_revert, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_ignores_plain_require_revert because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRequire.t.sol",
        r#"
contract SymbolicRequire {
    function checkRequire(uint256 x) public pure {
        require(x != 42, "hit");
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkRequire"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkRequire(uint256)"));
    assert!(stdout.contains("(paths:"));
});

forgetest_init!(symbolic_vm_assume_prunes_paths, |prj, cmd| {
    if !z3_available() {
        let _ =
            sh_eprintln!("skipping symbolic_vm_assume_prunes_paths because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicAssume.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicAssume is Test {
    function checkAssume(uint256 x) public {
        vm.assume(x != 42);
        assert(x != 42);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkAssume"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkAssume(uint256)"));
    assert!(stdout.contains("(paths:"));
});

forgetest_init!(symbolic_finds_bytes_counterexample_with_native_inline_config, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_finds_bytes_counterexample_with_native_inline_config because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBytes.t.sol",
        r#"
contract SymbolicBytes {
    /// forge-config: default.symbolic.array_lengths = [3]
    function checkBytes(bytes memory data) public pure {
        if (data[1] == 0x42) {
            assert(false);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkBytes"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkBytes(bytes)"), "{stdout}");
});

forgetest_init!(symbolic_replays_string_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_replays_string_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicString.t.sol",
        r#"
contract SymbolicString {
    /// forge-config: default.symbolic.array_lengths = [3]
    function checkString(string memory value) public pure {
        bytes memory data = bytes(value);
        if (data[0] == bytes1(uint8(0x41))) {
            assert(false);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkString"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkString(string)"), "{stdout}");
});

forgetest_init!(symbolic_uses_native_array_lengths, |prj, cmd| {
    if !z3_available() {
        let _ =
            sh_eprintln!("skipping symbolic_uses_native_array_lengths because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicNativeArrayLengths.t.sol",
        r#"
contract SymbolicNativeArrayLengths {
    /// forge-config: default.symbolic.array_lengths = [3]
    function checkArray(uint256[] memory values) public pure {
        assert(values.length == 3);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkArray"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkArray(uint256[])"), "{stdout}");
});

forgetest_init!(symbolic_uses_legacy_halmos_array_lengths, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_uses_legacy_halmos_array_lengths because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicHalmosLengths.t.sol",
        r#"
contract SymbolicHalmosLengths {
    /// @custom:halmos --array-lengths 3
    function checkArray(uint256[] memory values) public pure {
        assert(values.length == 3);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkArray"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkArray(uint256[])"), "{stdout}");
});

forgetest_init!(symbolic_handles_nested_struct_dynamic_input, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_handles_nested_struct_dynamic_input because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicNestedStruct.t.sol",
        r#"
contract SymbolicNestedStruct {
    struct Payload {
        uint256[] values;
        bytes note;
    }

    /// forge-config: default.symbolic.array_lengths = [2, 3]
    function checkStruct(Payload memory payload) public pure {
        assert(payload.values.length == 2);
        assert(payload.note.length == 3);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkStruct"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkStruct((uint256[],bytes))"), "{stdout}");
});

forgetest_init!(symbolic_allows_shorter_variants_with_positional_inner_lengths, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_allows_shorter_variants_with_positional_inner_lengths because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMixedLengthSets.t.sol",
        r#"
contract SymbolicMixedLengthSets {
    /// forge-config: default.symbolic.default_array_lengths = [1, 2]
    /// forge-config: default.symbolic.array_lengths = [4, 4]
    function checkBatch(bytes[] memory items) public pure {
        assert(items.length == 1 || items.length == 2);
        for (uint256 i; i < items.length; i++) {
            assert(items[i].length == 4);
        }
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkBatch"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicMixedLengthSets.t.sol:SymbolicMixedLengthSets
[PASS] checkBatch(bytes[]) ([METRICS])
...
"#]]);
});

forgetest_init!(symbolic_reports_calldata_variant_width_exhaustion, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_reports_calldata_variant_width_exhaustion because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicVariantLimit.t.sol",
        r#"
contract SymbolicVariantLimit {
    /// forge-config: default.symbolic.width = 2
    /// forge-config: default.symbolic.default_array_lengths = [1, 2]
    /// forge-config: default.symbolic.default_bytes_lengths = [1, 2]
    function checkVariants(bytes[] memory items) public pure {
        items;
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkVariants"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicVariantLimit.t.sol:SymbolicVariantLimit
[FAIL: incomplete symbolic execution (Stuck): symbolic calldata variant limit exceeded (2)] checkVariants(bytes[]) ([METRICS])
...
"#]]);
});

forgetest_init!(symbolic_rejects_malformed_halmos_array_lengths, |prj, cmd| {
    prj.add_test(
        "SymbolicMalformedHalmos.t.sol",
        r#"
contract SymbolicMalformedHalmos {
    /// forge-config: default.symbolic.default_dynamic_length = 2
    /// @custom:halmos --array-lengths nope
    function checkBytes(bytes memory data) public pure {
        data;
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--match-test", "checkBytes"])
        .assert_failure()
        .get_output()
        .clone();
    let stderr = output.stderr_lossy();

    assert!(stderr.contains("invalid @custom:halmos annotation"), "{stderr}");
    assert!(stderr.contains("invalid length `nope`"), "{stderr}");
});

forgetest_init!(symbolic_executes_typed_external_call, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_executes_typed_external_call because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExternalCall.t.sol",
        r#"
contract Helper {
    function inc(uint256 x) external returns (uint256) {
        return x + 1;
    }
}

contract SymbolicExternalCall {
    Helper helper;

    function setUp() public {
        helper = new Helper();
    }

    function checkExternal(uint256 x) public {
        assert(helper.inc(x) != 43);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExternal"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkExternal(uint256)"), "{stdout}");
    assert!(!stdout.contains("unsupported symbolic execution feature: external CALL"), "{stdout}");
});

forgetest_init!(symbolic_executes_low_level_external_call, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_executes_low_level_external_call because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicLowLevelCall.t.sol",
        r#"
contract Helper {
    function inc(uint256 x) external returns (uint256) {
        return x + 1;
    }
}

contract SymbolicLowLevelCall {
    Helper helper;

    function setUp() public {
        helper = new Helper();
    }

    function checkLowLevel(uint256 x) public {
        (bool ok, bytes memory ret) =
            address(helper).call(abi.encodeWithSelector(Helper.inc.selector, x));
        require(ok);
        uint256 value = abi.decode(ret, (uint256));
        assert(value != 7);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkLowLevel"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkLowLevel(uint256)"), "{stdout}");
    assert!(!stdout.contains("unsupported symbolic execution feature: external CALL"), "{stdout}");
});

forgetest_init!(symbolic_external_call_with_symbolic_selector_finds_backdoor, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_external_call_with_symbolic_selector_finds_backdoor because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSelectorBackdoor.t.sol",
        r#"
contract SelectorTarget {
    function innocent(uint256) external pure returns (bool) {
        return false;
    }

    function backdoor(uint256 x) external pure returns (bool) {
        return x == 7;
    }
}

contract SymbolicSelectorBackdoor {
    SelectorTarget target;

    function setUp() public {
        target = new SelectorTarget();
    }

    function checkNoBackdoor(bytes4 selector, uint256 x) public {
        (bool ok, bytes memory ret) = address(target).call(
            abi.encodeWithSelector(selector, x)
        );
        if (ok && abi.decode(ret, (bool))) {
            assert(false);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkNoBackdoor"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkNoBackdoor(bytes4,uint256)"), "{stdout}");
    assert!(stdout.contains("args="), "{stdout}");
    assert!(!stdout.contains("symbolic external CALL selector"), "{stdout}");
});

forgetest_init!(symbolic_external_call_with_symbolic_target_finds_backdoor, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_external_call_with_symbolic_target_finds_backdoor because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicTargetBackdoor.t.sol",
        r#"
import "forge-std/Test.sol";

contract GoodTarget {
    function check(uint256) external pure returns (bool) {
        return true;
    }
}

contract BadTarget {
    function check(uint256 x) external pure returns (bool) {
        return x != 7;
    }
}

contract SymbolicTargetBackdoor is Test {
    GoodTarget good;
    BadTarget bad;

    function setUp() public {
        good = new GoodTarget();
        bad = new BadTarget();
    }

    /// forge-config: default.symbolic.symbolic_call_targets = true
    function checkNoBadTarget(address target, uint256 x) public {
        vm.assume(target == address(good) || target == address(bad));
        (bool ok, bytes memory ret) = target.call(
            abi.encodeWithSignature("check(uint256)", x)
        );
        require(ok);
        assert(abi.decode(ret, (bool)));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkNoBadTarget"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkNoBadTarget(address,uint256)"), "{stdout}");
    assert!(stdout.contains("args="), "{stdout}");
    assert!(!stdout.contains("symbolic CALL target outside known contracts"), "{stdout}");
});

forgetest_init!(symbolic_external_call_with_single_known_target_auto_expands, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_external_call_with_single_known_target_auto_expands because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicTargetDefaultAuto.t.sol",
        r#"
import "forge-std/Test.sol";

contract OnlyTarget {
    function check(uint256) external pure returns (bool) {
        return true;
    }
}

contract SymbolicTargetDefaultAuto is Test {
    OnlyTarget onlyTarget;

    function setUp() public {
        onlyTarget = new OnlyTarget();
    }

    function checkTarget(address target, uint256 x) public {
        vm.assume(target == address(onlyTarget));
        (bool ok, bytes memory ret) = target.call(
            abi.encodeWithSignature("check(uint256)", x)
        );
        require(ok);
        assert(abi.decode(ret, (bool)));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkTarget"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkTarget(address,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CALL target"), "{stdout}");
});

forgetest_init!(
    symbolic_external_call_with_unbounded_symbolic_target_requires_config,
    |prj, cmd| {
        if !z3_available() {
            let _ = sh_eprintln!(
                "skipping symbolic_external_call_with_unbounded_symbolic_target_requires_config because z3 is not available"
            );
            return;
        }

        prj.add_test(
            "SymbolicTargetDefaultOff.t.sol",
            r#"
contract SymbolicTargetDefaultOff {
    function checkTarget(address target) public {
        (bool ok,) = target.call("");
        require(ok);
    }
}
"#,
        );

        let stdout = cmd
            .args(["test", "--symbolic", "--match-test", "checkTarget"])
            .assert_failure()
            .get_output()
            .stdout_lossy();

        assert!(stdout.contains("symbolic CALL target"), "{stdout}");
    }
);

forgetest_init!(symbolic_external_call_with_empty_unknown_target_is_modeled, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_external_call_with_empty_unknown_target_is_modeled because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicUnboundedTarget.t.sol",
        r#"
contract SymbolicUnboundedTarget {
    /// forge-config: default.symbolic.symbolic_call_targets = true
    function checkUnbounded(address target) public {
        if (uint160(target) <= 9 || target == address(this)) {
            return;
        }
        bool ok;
        assembly {
            ok := call(gas(), target, 0, 0, 0, 0, 0)
        }
        require(ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkUnbounded"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkUnbounded(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic CALL target outside known contracts"), "{stdout}");
});

forgetest_init!(symbolic_delegatecall_with_symbolic_target_executes, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_delegatecall_with_symbolic_target_executes because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicDelegateTarget.t.sol",
        r#"
import "forge-std/Test.sol";

contract SafeDelegateTarget {
    function ok(uint256) external pure returns (bool) {
        return true;
    }
}

contract OtherDelegateTarget {
    function ok(uint256) external pure returns (bool) {
        return true;
    }
}

contract SymbolicDelegateTarget is Test {
    SafeDelegateTarget safe;
    OtherDelegateTarget other;

    function setUp() public {
        safe = new SafeDelegateTarget();
        other = new OtherDelegateTarget();
    }

    /// forge-config: default.symbolic.symbolic_call_targets = true
    function checkDelegateTarget(address target, uint256 x) public {
        vm.assume(target == address(safe) || target == address(other));
        (bool ok, bytes memory ret) = target.delegatecall(
            abi.encodeWithSignature("ok(uint256)", x)
        );
        require(ok);
        assert(abi.decode(ret, (bool)));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkDelegateTarget"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkDelegateTarget(address,uint256)"), "{stdout}");
    assert!(stdout.contains("checkDelegateTarget(address,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CALL target"), "{stdout}");
});

forgetest_init!(symbolic_external_unknown_selector_returns_call_failure, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_external_unknown_selector_returns_call_failure because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicUnknownSelector.t.sol",
        r#"
contract OneSelectorTarget {
    function ping() external pure returns (uint256) {
        return 1;
    }
}

contract SymbolicUnknownSelector {
    OneSelectorTarget target;

    function setUp() public {
        target = new OneSelectorTarget();
    }

    function checkUnknownSelector(bytes4 selector) public {
        (bool ok,) = address(target).call(abi.encodeWithSelector(selector));
        if (selector == OneSelectorTarget.ping.selector) {
            assert(ok);
        } else {
            assert(!ok);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkUnknownSelector"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkUnknownSelector(bytes4)"), "{stdout}");
    assert!(!stdout.contains("symbolic external CALL selector"), "{stdout}");
});

forgetest_init!(symbolic_svm_create_bytes4_can_drive_selector_dispatch, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_svm_create_bytes4_can_drive_selector_dispatch because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSvmBytes4Selector.t.sol",
        r#"
interface Svm {
    function createBytes4(string calldata name) external returns (bytes4);
}

contract OneSelectorTarget {
    function ping() external pure returns (uint256) {
        return 1;
    }
}

contract SymbolicSvmBytes4Selector {
    address constant SVM_ADDRESS = address(0xF3993A62377BCd56AE39D773740A5390411E8BC9);
    OneSelectorTarget target;

    function setUp() public {
        target = new OneSelectorTarget();
    }

    function checkSvmBytes4Selector() public {
        bytes4 selector = Svm(SVM_ADDRESS).createBytes4("selector");
        (bool ok,) = address(target).call(abi.encodeWithSelector(selector));
        if (selector == OneSelectorTarget.ping.selector) {
            assert(ok);
        } else {
            assert(!ok);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSvmBytes4Selector"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSvmBytes4Selector()"), "{stdout}");
    assert!(!stdout.contains("symbolic external CALL selector"), "{stdout}");
});

forgetest_init!(symbolic_svm_create_calldata_generates_bounded_dispatch, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_svm_create_calldata_generates_bounded_dispatch because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSvmCreateCalldata.t.sol",
        r#"
interface Svm {
    function createCalldata(string calldata name) external returns (bytes memory);
}

contract OneSelectorTarget {
    function ping() external pure returns (uint256) {
        return 1;
    }
}

contract SymbolicSvmCreateCalldata {
    address constant SVM_ADDRESS = address(0xF3993A62377BCd56AE39D773740A5390411E8BC9);
    OneSelectorTarget target;

    function setUp() public {
        target = new OneSelectorTarget();
    }

    /// forge-config: default.symbolic.max_calldata_bytes = 4
    function checkSvmCreateCalldata() public {
        bytes memory data = Svm(SVM_ADDRESS).createCalldata("data");
        assert(data.length <= 4);
        (bool ok,) = address(target).call(data);

        bytes4 selector;
        assembly {
            selector := mload(add(data, 0x20))
        }
        if (selector == OneSelectorTarget.ping.selector) {
            assert(ok);
        } else {
            assert(!ok);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSvmCreateCalldata"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSvmCreateCalldata()"), "{stdout}");
    assert!(!stdout.contains("symbolic external CALL selector"), "{stdout}");
});

forgetest_init!(symbolic_external_require_is_call_failure, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_external_require_is_call_failure because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExternalRequire.t.sol",
        r#"
contract Helper {
    function rejectFortyTwo(uint256 x) external pure returns (bool) {
        require(x != 42, "hit");
        return true;
    }
}

contract SymbolicExternalRequire {
    Helper helper;

    function setUp() public {
        helper = new Helper();
    }

    function checkRequireCall(uint256 x) public {
        (bool ok,) = address(helper).call(
            abi.encodeWithSelector(Helper.rejectFortyTwo.selector, x)
        );
        if (x == 42) {
            assert(!ok);
        } else {
            assert(ok);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkRequireCall"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkRequireCall(uint256)"), "{stdout}");
    assert!(!stdout.contains("unsupported symbolic execution feature: external CALL"), "{stdout}");
});

forgetest_init!(symbolic_staticcall_rejects_storage_write, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_staticcall_rejects_storage_write because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicStaticCall.t.sol",
        r#"
contract Writer {
    uint256 value;

    function set(uint256 x) external {
        value = x;
    }
}

contract SymbolicStaticCall {
    Writer writer;

    function setUp() public {
        writer = new Writer();
    }

    function checkStatic(uint256 x) public view {
        (bool ok,) = address(writer).staticcall(
            abi.encodeWithSelector(Writer.set.selector, x)
        );
        assert(!ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkStatic"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkStatic(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_delegatecall_writes_caller_storage, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_delegatecall_writes_caller_storage because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicDelegateCall.t.sol",
        r#"
contract DelegateTarget {
    uint256 public value;

    function set(uint256 x) external {
        value = x;
    }
}

contract SymbolicDelegateCall {
    uint256 public value;
    DelegateTarget target;

    function setUp() public {
        target = new DelegateTarget();
    }

    function checkDelegate(uint256 x) public {
        (bool ok,) = address(target).delegatecall(
            abi.encodeWithSelector(DelegateTarget.set.selector, x)
        );
        require(ok);
        assert(value == x);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkDelegate"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkDelegate(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_call_transfers_value_and_checks_balance, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_call_transfers_value_and_checks_balance because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicValueCall.t.sol",
        r#"
import "forge-std/Test.sol";

contract Sink {
    receive() external payable {}
}

contract SymbolicValueCall is Test {
    Sink sink;

    function setUp() public {
        sink = new Sink();
    }

    function checkValueTransfer() public {
        vm.deal(address(this), 1);
        (bool ok,) = address(sink).call{value: 1}("");
        assert(ok);
        assert(address(this).balance == 0);
        assert(address(sink).balance == 1);

        (bool second,) = address(sink).call{value: 2}("");
        assert(!second);
        assert(address(sink).balance == 1);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkValueTransfer"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkValueTransfer()"), "{stdout}");
});

forgetest_init!(symbolic_call_accepts_symbolic_value, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_call_accepts_symbolic_value because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicValueCall.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicValueReceiver {
    uint256 public last;

    function receiveValue() external payable returns (uint256) {
        last = msg.value;
        return msg.value;
    }
}

contract SymbolicValueCall is Test {
    SymbolicValueReceiver receiver;

    function setUp() public {
        receiver = new SymbolicValueReceiver();
    }

    function checkSymbolicValueTransfer(uint256 amount) public {
        vm.assume(amount <= 5);
        vm.deal(address(this), 5);

        (bool ok, bytes memory ret) = address(receiver).call{value: amount}(
            abi.encodeWithSelector(SymbolicValueReceiver.receiveValue.selector)
        );

        assert(ok);
        assertEq(abi.decode(ret, (uint256)), amount);
        assertEq(receiver.last(), amount);
        assertEq(address(receiver).balance, amount);
        assertEq(address(this).balance, 5 - amount);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicValueTransfer"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicValueTransfer(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic external CALL value"), "{stdout}");
});

forgetest_init!(symbolic_call_splits_symbolic_insufficient_value, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_call_splits_symbolic_insufficient_value because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInsufficientValueCall.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicValueSink {
    receive() external payable {}
}

contract SymbolicInsufficientValueCall is Test {
    SymbolicValueSink sink;

    function setUp() public {
        sink = new SymbolicValueSink();
    }

    function checkSymbolicInsufficientValue(uint256 amount) public {
        vm.assume(amount <= 2);
        vm.deal(address(this), 1);

        (bool ok,) = address(sink).call{value: amount}("");

        assert(ok == (amount <= 1));
        assertEq(address(sink).balance, ok ? amount : 0);
        assertEq(address(this).balance, ok ? 1 - amount : 1);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicInsufficientValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicInsufficientValue(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic external CALL value"), "{stdout}");
});

forgetest_init!(symbolic_callcode_accepts_symbolic_value, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_callcode_accepts_symbolic_value because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCallcodeValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCallcodeValueTarget {
    function echoValue() external payable returns (uint256) {
        return msg.value;
    }
}

contract SymbolicCallcodeValue is Test {
    SymbolicCallcodeValueTarget target;

    function setUp() public {
        target = new SymbolicCallcodeValueTarget();
    }

    function checkSymbolicCallcodeValue(uint256 amount) public {
        vm.assume(amount <= 7);
        vm.deal(address(this), 7);

        bytes memory input = abi.encodeWithSelector(SymbolicCallcodeValueTarget.echoValue.selector);
        uint256 echoed;
        bool ok;
        address callTarget = address(target);
        assembly {
            ok := callcode(gas(), callTarget, amount, add(input, 0x20), mload(input), 0x80, 0x20)
            echoed := mload(0x80)
        }

        assert(ok);
        assertEq(echoed, amount);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCallcodeValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicCallcodeValue(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CALLCODE value"), "{stdout}");
});

forgetest_init!(symbolic_cheatcodes_accept_symbolic_address_targets, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_cheatcodes_accept_symbolic_address_targets because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicAddressCheatcodes.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicAddressCheatcodes is Test {
    function checkSymbolicDealStoreLoadAndNonce(address who, bytes32 value) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.deal(who, 11);
        assertEq(who.balance, 11);

        bytes32 slot = bytes32(uint256(1));
        vm.store(who, slot, value);
        assertEq(vm.load(who, slot), value);

        assertEq(vm.getNonce(who), 0);
        vm.setNonceUnsafe(who, 5);
        assertEq(vm.getNonce(who), 5);
        vm.resetNonce(who);
        assertEq(vm.getNonce(who), 0);
    }

    function checkSymbolicEtch(address who) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.etch(who, hex"00");
        assertEq(who.code.length, 1);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicAddressCheatcodes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(
        stdout.contains("[PASS] checkSymbolicDealStoreLoadAndNonce(address,bytes32)"),
        "{stdout}"
    );
    assert!(stdout.contains("[PASS] checkSymbolicEtch(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.deal target"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.store target"), "{stdout}");
    assert!(!stdout.contains("symbolic EXTCODESIZE target"), "{stdout}");
});

forgetest_init!(symbolic_prank_accepts_symbolic_sender, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_prank_accepts_symbolic_sender because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPrankSender.t.sol",
        r#"
import "forge-std/Test.sol";

contract SenderTarget {
    function sender() external view returns (address) {
        return msg.sender;
    }

    function origin() external view returns (address) {
        return tx.origin;
    }

    function context() external view returns (address, address) {
        return (msg.sender, tx.origin);
    }
}

contract SymbolicPrankSender is Test {
    SenderTarget target;

    function setUp() public {
        target = new SenderTarget();
    }

    function checkSymbolicPrank(address who) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.prank(who);
        assertEq(target.sender(), who);
    }

    function checkSymbolicStartPrank(address who) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.startPrank(who);
        assertEq(target.sender(), who);
        assertEq(target.sender(), who);
        vm.stopPrank();
    }

    function checkSymbolicPrankOrigin(address who, address origin) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.prank(who, origin);
        (address actualSender, address actualOrigin) = target.context();
        assertEq(actualSender, who);
        assertEq(actualOrigin, origin);
    }

    function checkSymbolicStartPrankOrigin(address who, address origin) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.startPrank(who, origin);
        assertEq(target.sender(), who);
        assertEq(target.origin(), origin);
        assertEq(target.sender(), who);
        assertEq(target.origin(), origin);
        vm.stopPrank();
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicPrankSender"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicPrank(address)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkSymbolicStartPrank(address)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkSymbolicPrankOrigin(address,address)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkSymbolicStartPrankOrigin(address,address)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.prank"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.startPrank"), "{stdout}");
});

forgetest_init!(symbolic_balance_accepts_symbolic_target, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_balance_accepts_symbolic_target because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBalance.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBalance is Test {
    function checkSymbolicBalance(address who) public {
        address funded = address(0xBEEF);
        vm.deal(funded, 123);

        uint256 expected = who == funded ? 123 : 0;
        assertEq(who.balance, expected);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicBalance"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicBalance(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic BALANCE target"), "{stdout}");
});

forgetest_init!(symbolic_extcodesize_accepts_symbolic_target, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodesize_accepts_symbolic_target because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeSize is Test {
    function checkSymbolicCodeLength(address who) public {
        address coded = address(0xC0DE);
        vm.etch(coded, hex"60006000");

        uint256 expected = who == coded ? 4 : 0;
        assertEq(who.code.length, expected);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicExtcodeSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicCodeLength(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic EXTCODESIZE target"), "{stdout}");
});

forgetest_init!(symbolic_extcodehash_accepts_symbolic_target, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodehash_accepts_symbolic_target because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeHash.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeHash is Test {
    function checkSymbolicCodeHash(address who) public {
        address coded = address(0xC0DE);
        vm.etch(coded, hex"60006000");

        bytes32 expected = who == coded ? keccak256(hex"60006000") : bytes32(0);
        assertEq(who.codehash, expected);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicExtcodeHash"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicCodeHash(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic EXTCODEHASH target"), "{stdout}");
});

forgetest_init!(symbolic_extcodecopy_accepts_symbolic_target, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodecopy_accepts_symbolic_target because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeCopy.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeCopy is Test {
    function checkSymbolicExtcodeCopy(address who) public {
        address coded = address(0xC0DE);
        vm.etch(coded, hex"60016002");

        bytes32 copied;
        assembly {
            extcodecopy(who, 0x80, 0, 4)
            copied := mload(0x80)
        }

        bytes32 expected = who == coded ? bytes32(hex"6001600200000000000000000000000000000000000000000000000000000000") : bytes32(0);
        assertEq(copied, expected);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicExtcodeCopy"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicExtcodeCopy(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic EXTCODECOPY target"), "{stdout}");
});

forgetest_init!(symbolic_create_deploys_and_calls_helper, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_deploys_and_calls_helper because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreate.t.sol",
        r#"
contract CreatedHelper {
    function inc(uint256 x) external pure returns (uint256) {
        return x + 1;
    }
}

contract SymbolicCreate {
    function checkCreate(uint256 x) public {
        CreatedHelper helper = new CreatedHelper();
        assert(helper.inc(x) != 9);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkCreate(uint256)"), "{stdout}");
    assert!(!stdout.contains("unsupported opcode: 0xf0"), "{stdout}");
});

forgetest_init!(symbolic_create_preserves_symbolic_constructor_args, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_preserves_symbolic_constructor_args because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreateConstructorArgs.t.sol",
        r#"
contract CreatedStore {
    uint256 public value;

    constructor(uint256 x) {
        value = x;
    }
}

contract SymbolicCreateConstructorArgs {
    function checkCreateConstructorArg(uint256 x) public {
        CreatedStore store = new CreatedStore(x);
        assert(store.value() == x);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateConstructorArg"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreateConstructorArg(uint256)"), "{stdout}");
    assert!(
        !stdout.contains("unsupported symbolic execution feature: symbolic CREATE initcode"),
        "{stdout}"
    );
});

forgetest_init!(symbolic_create_accepts_constrained_symbolic_initcode_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_accepts_constrained_symbolic_initcode_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreateInitcodeOffset.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCreateInitcodeOffset is Test {
    function checkCreateInitcodeOffset(uint16 offset) public {
        vm.assume(offset == 0x80);

        address created;
        uint256 size;
        assembly {
            mstore(0x80, 0x6001600c60003960016000f30000000000000000000000000000000000000000)
            created := create(0, offset, 13)
            size := extcodesize(created)
        }

        assert(created != address(0));
        assert(size == 1);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateInitcodeOffset"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreateInitcodeOffset(uint16)"), "{stdout}");
    assert!(!stdout.contains("symbolic CREATE initcode offset"), "{stdout}");
    assert!(!stdout.contains("symbolic bytecode opcode"), "{stdout}");
});

forgetest_init!(symbolic_create2_deploys_and_calls_helper, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create2_deploys_and_calls_helper because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreate2.t.sol",
        r#"
contract CreatedHelper {
    function inc(uint256 x) external pure returns (uint256) {
        return x + 1;
    }
}

contract SymbolicCreate2 {
    function checkCreate2(uint256 x) public {
        CreatedHelper helper = new CreatedHelper{salt: bytes32(uint256(123))}();
        assert(helper.inc(x) != 11);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate2"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkCreate2(uint256)"), "{stdout}");
    assert!(!stdout.contains("unsupported opcode: 0xf5"), "{stdout}");
});

forgetest_init!(symbolic_create2_preserves_symbolic_constructor_args, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create2_preserves_symbolic_constructor_args because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreate2Args.t.sol",
        r#"
contract CreatedImmutable {
    uint256 immutable value;

    constructor(uint256 value_) {
        value = value_;
    }

    function get() external view returns (uint256) {
        return value;
    }
}

contract SymbolicCreate2Args {
    function checkCreate2ConstructorArg(uint256 x) public {
        CreatedImmutable created = new CreatedImmutable{salt: bytes32(uint256(7))}(x);
        assert(created.get() == x);
        assert(address(created).code.length > 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate2ConstructorArg"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreate2ConstructorArg(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CREATE2 initcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_create_matches_and_reports_missing, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_create_matches_and_reports_missing because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectCreate.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedCreateTarget {
    function ping() external pure returns (uint256) {
        return 1;
    }
}

contract SymbolicExpectCreate is Test {
    function checkCreateExpectation(uint256) public {
        vm.expectCreate(type(SymbolicExpectedCreateTarget).runtimeCode, address(this));
        SymbolicExpectedCreateTarget target = new SymbolicExpectedCreateTarget();
        assertEq(target.ping(), 1);
    }

    function checkCreate2Expectation(uint256) public {
        vm.expectCreate2(type(SymbolicExpectedCreateTarget).runtimeCode, address(this));
        SymbolicExpectedCreateTarget target = new SymbolicExpectedCreateTarget{salt: bytes32(uint256(99))}();
        assertEq(target.ping(), 1);
    }

    function checkSymbolicCreateExpectation(address deployer) public {
        vm.assume(deployer == address(this));
        vm.expectCreate(type(SymbolicExpectedCreateTarget).runtimeCode, deployer);
        SymbolicExpectedCreateTarget target = new SymbolicExpectedCreateTarget();
        assertEq(target.ping(), 1);
    }

    function checkMismatchedSymbolicCreateExpectation(address deployer) public {
        vm.assume(deployer != address(this));
        vm.expectCreate(type(SymbolicExpectedCreateTarget).runtimeCode, deployer);
        new SymbolicExpectedCreateTarget();
    }

    function checkMissingCreateExpectation(uint256) public {
        vm.expectCreate(type(SymbolicExpectedCreateTarget).runtimeCode, address(this));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateExpectation"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreateExpectation(uint256)"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkCreate2Expectation"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreate2Expectation(uint256)"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCreateExpectation"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicCreateExpectation(address)"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMismatchedSymbolicCreateExpectation"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkMismatchedSymbolicCreateExpectation(address)"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMissingCreateExpectation"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkMissingCreateExpectation(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_create2_supports_symbolic_salt_and_self_address, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create2_supports_symbolic_salt_and_self_address because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreate2SelfAddress.t.sol",
        r#"
contract CreatedSelfAddress {
    address public constructorSelf;

    constructor() {
        constructorSelf = address(this);
    }

    function runtimeSelf() external view returns (address) {
        return address(this);
    }
}

contract SymbolicCreate2SelfAddress {
    function checkCreate2SelfAddress(uint256 salt) public {
        CreatedSelfAddress created = new CreatedSelfAddress{salt: bytes32(salt)}();
        assert(created.constructorSelf() == address(created));
        assert(created.runtimeSelf() == address(created));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate2SelfAddress"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreate2SelfAddress(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CREATE2 salt"), "{stdout}");
    assert!(!stdout.contains("symbolic CALL target"), "{stdout}");
});

forgetest_init!(symbolic_create2_collision_returns_zero, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create2_collision_returns_zero because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreate2Collision.t.sol",
        r#"
import "forge-std/Test.sol";

contract CreatedHelper {}

contract SymbolicCreate2Collision is Test {
    function checkCreate2Collision() public {
        uint64 beforeNonce = vm.getNonce(address(this));
        bytes memory code = type(CreatedHelper).creationCode;
        address first;
        address second;
        assembly {
            first := create2(0, add(code, 0x20), mload(code), 1)
            second := create2(0, add(code, 0x20), mload(code), 1)
        }
        assert(first != address(0));
        assert(second == address(0));
        assert(vm.getNonce(address(this)) == beforeNonce + 2);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate2Collision"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreate2Collision()"), "{stdout}");
});

forgetest_init!(symbolic_create_failure_bumps_creator_nonce, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_failure_bumps_creator_nonce because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreateFailureNonce.t.sol",
        r#"
import "forge-std/Test.sol";

contract RevertingCreate {
    constructor() {
        revert();
    }
}

contract SymbolicCreateFailureNonce is Test {
    function checkCreateFailureNonce(uint256) public {
        uint64 beforeNonce = vm.getNonce(address(this));
        try new RevertingCreate() {
            assert(false);
        } catch {}
        assert(vm.getNonce(address(this)) == beforeNonce + 1);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateFailureNonce"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreateFailureNonce(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_compute_create_address_cheatcodes, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_compute_create_address_cheatcodes because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicComputeCreateAddresses.t.sol",
        r#"
import "forge-std/Test.sol";

contract CreatedHelper {}

contract SymbolicComputeCreateAddresses is Test {
    address constant DEFAULT_CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

    function checkComputeCreateAddress(uint256) public {
        uint64 nonce = vm.getNonce(address(this));
        address expected = vm.computeCreateAddress(address(this), nonce);
        CreatedHelper created = new CreatedHelper();
        assert(address(created) == expected);
    }

    function checkSymbolicComputeCreateAddress(uint64 nonce) public {
        address first = vm.computeCreateAddress(address(this), nonce);
        address second = vm.computeCreateAddress(address(this), nonce);

        assert(first == second);
        assert(uint160(first) <= type(uint160).max);
    }

    function checkSymbolicComputeCreateAddressDeployer(address deployer, uint64 nonce) public {
        address first = vm.computeCreateAddress(deployer, nonce);
        address second = vm.computeCreateAddress(deployer, nonce);

        assert(first == second);
        assert(uint160(first) <= type(uint160).max);
    }

    function checkComputeCreate2Address(uint256 saltValue) public {
        bytes32 salt = bytes32(saltValue);
        bytes memory code = type(CreatedHelper).creationCode;
        address expected = vm.computeCreate2Address(salt, keccak256(code), address(this));
        address created;
        assembly {
            created := create2(0, add(code, 0x20), mload(code), salt)
        }
        assert(created == expected);
    }

    function checkSymbolicComputeCreate2Address(bytes32 salt, bytes32 initCodeHash) public {
        address first = vm.computeCreate2Address(salt, initCodeHash, address(this));
        address second = vm.computeCreate2Address(salt, initCodeHash, address(this));

        assert(first == second);
        assert(uint160(first) <= type(uint160).max);
    }

    function checkSymbolicComputeCreate2AddressDeployer(
        address deployer,
        bytes32 salt,
        bytes32 initCodeHash
    ) public {
        address first = vm.computeCreate2Address(salt, initCodeHash, deployer);
        address second = vm.computeCreate2Address(salt, initCodeHash, deployer);

        assert(first == second);
        assert(uint160(first) <= type(uint160).max);
    }

    function checkComputeCreate2DefaultDeployer() public {
        bytes memory code = type(CreatedHelper).creationCode;
        bytes32 salt = bytes32(uint256(1));
        bytes32 initCodeHash = keccak256(code);
        address expected = vm.computeCreate2Address(salt, initCodeHash);
        address manual = address(uint160(uint256(keccak256(abi.encodePacked(
            bytes1(0xff), DEFAULT_CREATE2_DEPLOYER, salt, initCodeHash
        )))));
        assert(expected == manual);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicComputeCreateAddresses"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkComputeCreateAddress(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkSymbolicComputeCreateAddress(uint64)"), "{stdout}");
    assert!(
        stdout.contains("[PASS] checkSymbolicComputeCreateAddressDeployer(address,uint64)"),
        "{stdout}"
    );
    assert!(stdout.contains("[PASS] checkComputeCreate2Address(uint256)"), "{stdout}");
    assert!(
        stdout.contains("[PASS] checkSymbolicComputeCreate2Address(bytes32,bytes32)"),
        "{stdout}"
    );
    assert!(
        stdout
            .contains("[PASS] checkSymbolicComputeCreate2AddressDeployer(address,bytes32,bytes32)"),
        "{stdout}"
    );
    assert!(stdout.contains("[PASS] checkComputeCreate2DefaultDeployer()"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.computeCreateAddress nonce"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.computeCreate2Address init code hash"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.computeCreateAddress deployer"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.computeCreate2Address deployer"), "{stdout}");
});

forgetest_init!(symbolic_vm_nonce_cheatcodes, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_vm_nonce_cheatcodes because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicNonceCheatcodes.t.sol",
        r#"
import "forge-std/Test.sol";

contract NonceTarget {}

contract SymbolicNonceCheatcodes is Test {
    function checkSetNonceCheatcodes(uint256) public {
        address account = address(0x1234);
        assertEq(vm.getNonce(account), 0);

        vm.setNonce(account, 7);
        assertEq(vm.getNonce(account), 7);

        vm.setNonceUnsafe(account, 2);
        assertEq(vm.getNonce(account), 2);
    }

    function checkResetNonceCheatcode(uint256) public {
        address account = address(0xbeef);
        vm.setNonce(account, 3);
        vm.resetNonce(account);
        assertEq(vm.getNonce(account), 0);

        NonceTarget target = new NonceTarget();
        vm.setNonce(address(target), 9);
        vm.resetNonce(address(target));
        assertEq(vm.getNonce(address(target)), 1);
    }

}
"#,
    );

    let stdout = cmd
        .args([
            "test",
            "--symbolic",
            "--match-test",
            "checkSetNonceCheatcodes|checkResetNonceCheatcode",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSetNonceCheatcodes(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkResetNonceCheatcode(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_vm_set_nonce_rejects_decrement, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_set_nonce_rejects_decrement because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSetNonceRejectsDecrement.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicSetNonceRejectsDecrement is Test {
    function checkSetNonceRejectsDecrement(uint256) public {
        address account = address(0xcafe);
        vm.setNonce(account, 4);
        vm.setNonce(account, 3);
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--match-test", "checkSetNonceRejectsDecrement"])
        .assert_failure()
        .get_output()
        .clone();
    let output = format!("{}{}", output.stdout_lossy(), output.stderr_lossy());

    assert!(output.contains("[FAIL"), "{output}");
    assert!(output.contains("checkSetNonceRejectsDecrement(uint256)"), "{output}");
});

forgetest_init!(symbolic_create_transfers_value_and_checks_balance, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_transfers_value_and_checks_balance because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreateValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract PayableCreated {
    constructor() payable {}
}

contract SymbolicCreateValue is Test {
    function checkCreateValue() public {
        vm.deal(address(this), 1);
        bytes memory code = type(PayableCreated).creationCode;
        address first;
        address second;
        assembly {
            first := create(1, add(code, 0x20), mload(code))
            second := create(2, add(code, 0x20), mload(code))
        }
        assert(first != address(0));
        assert(first.balance == 1);
        assert(second == address(0));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreateValue()"), "{stdout}");
});

forgetest_init!(symbolic_create_accepts_symbolic_value, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_accepts_symbolic_value because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreateSymbolicValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract PayableCreatedWithValue {
    uint256 public paid;

    constructor() payable {
        paid = msg.value;
    }
}

contract SymbolicCreateSymbolicValue is Test {
    function checkCreateSymbolicValue(uint256 amount) public {
        vm.assume(amount <= 5);
        vm.deal(address(this), 5);

        PayableCreatedWithValue created = new PayableCreatedWithValue{value: amount}();

        assertEq(created.paid(), amount);
        assertEq(address(created).balance, amount);
        assertEq(address(this).balance, 5 - amount);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateSymbolicValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreateSymbolicValue(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CREATE value"), "{stdout}");
});

forgetest_init!(symbolic_create_splits_symbolic_insufficient_value, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_splits_symbolic_insufficient_value because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreateInsufficientValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract PayableCreatedForInsufficient {
    constructor() payable {}
}

contract SymbolicCreateInsufficientValue is Test {
    function checkCreateInsufficientValue(uint256 amount) public {
        vm.assume(amount <= 2);
        vm.deal(address(this), 1);
        bytes memory code = type(PayableCreatedForInsufficient).creationCode;

        address created;
        assembly {
            created := create(amount, add(code, 0x20), mload(code))
        }

        bool ok = created != address(0);
        assertEq(ok, amount <= 1);
        assertEq(address(this).balance, ok ? 1 - amount : 1);
        if (ok) {
            assertEq(created.balance, amount);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateInsufficientValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreateInsufficientValue(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CREATE value"), "{stdout}");
    assert!(!stdout.contains("symbolic CREATE balance"), "{stdout}");
});

forgetest_init!(symbolic_create2_accepts_symbolic_value, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create2_accepts_symbolic_value because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreate2SymbolicValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract PayableCreate2WithValue {
    uint256 public paid;

    constructor() payable {
        paid = msg.value;
    }
}

contract SymbolicCreate2SymbolicValue is Test {
    function checkCreate2SymbolicValue(uint256 amount) public {
        vm.assume(amount <= 5);
        vm.deal(address(this), 5);

        PayableCreate2WithValue created =
            new PayableCreate2WithValue{salt: bytes32(uint256(0x1234)), value: amount}();

        assertEq(created.paid(), amount);
        assertEq(address(created).balance, amount);
        assertEq(address(this).balance, 5 - amount);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate2SymbolicValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreate2SymbolicValue(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CREATE value"), "{stdout}");
});

forgetest_init!(symbolic_create_accepts_bounded_symbolic_initcode_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_accepts_bounded_symbolic_initcode_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreateInitcodeSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCreateInitcodeSize is Test {
    function checkCreateInitcodeSize(uint256 size) public {
        vm.assume(size == 0 || size == 13);
        bytes memory code = hex"6001600c60003960016000f300";

        address created;
        assembly {
            created := create(0, add(code, 0x20), size)
        }

        assert(created != address(0));
        assertEq(created.code.length, size == 13 ? 1 : 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateInitcodeSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCreateInitcodeSize(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic CREATE initcode size"), "{stdout}");
    assert!(!stdout.contains("symbolic bytecode opcode"), "{stdout}");
});

forgetest_init!(symbolic_staticcall_rejects_create, |prj, cmd| {
    if !z3_available() {
        let _ =
            sh_eprintln!("skipping symbolic_staticcall_rejects_create because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicStaticCreate.t.sol",
        r#"
contract CreatedHelper {}

contract Creator {
    function deploy() external returns (address created) {
        bytes memory code = type(CreatedHelper).creationCode;
        assembly {
            created := create(0, add(code, 0x20), mload(code))
        }
    }
}

contract SymbolicStaticCreate {
    Creator creator;

    function setUp() public {
        creator = new Creator();
    }

    function checkStaticCreate() public view {
        (bool ok,) = address(creator).staticcall(abi.encodeWithSelector(Creator.deploy.selector));
        assert(!ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkStaticCreate"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkStaticCreate()"), "{stdout}");
});

forgetest_init!(symbolic_mapping_storage_finds_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_mapping_storage_finds_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMappingStorage.t.sol",
        r#"
contract SymbolicMappingStorage {
    mapping(address => uint256) values;

    function checkMapping(address who, uint256 value) public {
        values[who] = value;
        if (values[who] == 7) {
            assert(false);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMapping"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkMapping(address,uint256)"), "{stdout}");
    assert!(stdout.contains("args=["), "{stdout}");
    assert!(!stdout.contains("symbolic SHA3"), "{stdout}");
    assert!(!stdout.contains("symbolic SSTORE key"), "{stdout}");
    assert!(!stdout.contains("symbolic SLOAD key"), "{stdout}");
});

forgetest_init!(symbolic_nested_mapping_storage_round_trips, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_nested_mapping_storage_round_trips because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicNestedMappingStorage.t.sol",
        r#"
contract SymbolicNestedMappingStorage {
    mapping(address => mapping(address => uint256)) allowances;

    function checkNestedMapping(address owner, address spender, uint256 value) public {
        allowances[owner][spender] = value;
        assert(allowances[owner][spender] == value);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkNestedMapping"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkNestedMapping(address,address,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic SHA3"), "{stdout}");
});

forgetest_init!(symbolic_vm_store_load_accepts_symbolic_slot, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_store_load_accepts_symbolic_slot because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicVmStoreLoadSlot.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicVmStoreLoadSlot is Test {
    function checkStoreLoad(bytes32 slot, bytes32 value) public {
        vm.store(address(this), slot, value);
        assert(vm.load(address(this), slot) == value);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkStoreLoad"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkStoreLoad(bytes32,bytes32)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.store slot"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.load slot"), "{stdout}");
});

forgetest_init!(symbolic_mapping_dynamic_array_storage_round_trips, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_mapping_dynamic_array_storage_round_trips because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMappingDynamicArrayStorage.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicMappingDynamicArrayStorage is Test {
    mapping(address => uint256[]) values;

    function checkMappingArray(address owner, uint256 index, uint256 value) public {
        values[owner].push(0);
        values[owner].push(0);
        values[owner].push(0);

        vm.assume(index < values[owner].length);
        values[owner][index] = value;
        assert(values[owner][index] == value);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMappingArray"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkMappingArray(address,uint256,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic SHA3"), "{stdout}");
    assert!(!stdout.contains("symbolic SSTORE key"), "{stdout}");
    assert!(!stdout.contains("symbolic SLOAD key"), "{stdout}");
});

forgetest_init!(symbolic_packed_storage_round_trips, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_packed_storage_round_trips because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPackedStorage.t.sol",
        r#"
contract SymbolicPackedStorage {
    uint128 left;
    uint128 right;
    bool flag;
    address owner;

    function checkPacked(uint128 a, uint128 b, bool enabled, address who) public {
        left = a;
        right = b;
        flag = enabled;
        owner = who;

        assert(left == a);
        assert(right == b);
        assert(flag == enabled);
        assert(owner == who);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkPacked"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkPacked(uint128,uint128,bool,address)"), "{stdout}");
});

forgetest_init!(symbolic_erc20_storage_paths_round_trip, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_erc20_storage_paths_round_trip because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicErc20Storage.t.sol",
        r#"
contract SymbolicErc20Storage {
    mapping(address => uint256) balanceOf;
    mapping(address => mapping(address => uint256)) allowance;

    function checkErc20Storage(address owner, address spender, uint256 amount) public {
        balanceOf[owner] = amount;
        allowance[owner][spender] = amount;

        assert(balanceOf[owner] == amount);
        assert(allowance[owner][spender] == amount);
        assert(balanceOf[owner] == allowance[owner][spender]);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkErc20Storage"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkErc20Storage(address,address,uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic SHA3"), "{stdout}");
    assert!(!stdout.contains("symbolic SSTORE key"), "{stdout}");
    assert!(!stdout.contains("symbolic SLOAD key"), "{stdout}");
});

forgetest_init!(symbolic_erc20_transfer_from_storage_paths_do_not_alias, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_erc20_transfer_from_storage_paths_do_not_alias because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicErc20TransferFromStorage.t.sol",
        r#"
contract SymbolicErc20TransferFromStorage {
    mapping(address => uint256) balanceOf;
    mapping(address => mapping(address => uint256)) allowance;

    function checkTransferFromStorage(
        address owner,
        address spender,
        address recipient,
        uint96 balance,
        uint96 approval,
        uint96 amount
    ) public {
        balanceOf[owner] = balance;
        allowance[owner][spender] = approval;

        uint256 beforeTotal = balanceOf[owner] + balanceOf[recipient];
        if (owner != recipient && amount <= balanceOf[owner] && amount <= allowance[owner][spender]) {
            allowance[owner][spender] -= amount;
            balanceOf[owner] -= amount;
            balanceOf[recipient] += amount;

            assert(balanceOf[owner] + balanceOf[recipient] == beforeTotal);
            assert(allowance[owner][spender] == uint256(approval) - amount);
        }
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkTransferFromStorage"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicErc20TransferFromStorage.t.sol:SymbolicErc20TransferFromStorage
[PASS] checkTransferFromStorage(address,address,address,uint96,uint96,uint96) ([METRICS])
...
"#]]);
});

forgetest_init!(symbolic_svm_storage_helpers_are_supported, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_svm_storage_helpers_are_supported because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSvmStorageHelpers.t.sol",
        r#"
interface Svm {
    function enableSymbolicStorage(address target) external;
    function setArbitraryStorage(address target) external;
    function snapshotStorage(address target) external returns (uint256);
    function snapshotState() external returns (uint256);
}

contract SymbolicSvmStorageHelpers {
    address constant SVM_ADDRESS = address(0xF3993A62377BCd56AE39D773740A5390411E8BC9);

    function checkSvmStorageHelpers(bytes32 slot, bytes32 value) public {
        Svm(SVM_ADDRESS).enableSymbolicStorage(address(this));
        Svm(SVM_ADDRESS).setArbitraryStorage(address(this));
        uint256 stateSnapshot = Svm(SVM_ADDRESS).snapshotState();
        uint256 storageSnapshot = Svm(SVM_ADDRESS).snapshotStorage(address(this));

        bytes32 loaded;
        assembly {
            sstore(slot, value)
            loaded := sload(slot)
        }

        assert(loaded == value);
        assert(storageSnapshot == stateSnapshot + 1);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSvmStorageHelpers"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSvmStorageHelpers(bytes32,bytes32)"), "{stdout}");
    assert!(!stdout.contains("symbolic Halmos compatibility cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_generic_storage_exposes_arbitrary_uninitialized_reads, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_generic_storage_exposes_arbitrary_uninitialized_reads because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicGenericStorage.t.sol",
        r#"
interface Svm {
    function setArbitraryStorage(address target) external;
}

contract SymbolicGenericStorage {
    address constant SVM_ADDRESS = address(0xF3993A62377BCd56AE39D773740A5390411E8BC9);

    mapping(address => uint256) balanceOf;

    /// forge-config: default.symbolic.storage_layout = "generic"
    function checkNativeGenericStorage(address owner) public view {
        assert(balanceOf[owner] == 0);
    }

    function checkSvmArbitraryStorage(address owner) public {
        Svm(SVM_ADDRESS).setArbitraryStorage(address(this));
        assert(balanceOf[owner] == 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicGenericStorage"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkNativeGenericStorage(address)"), "{stdout}");
    assert!(stdout.contains("checkSvmArbitraryStorage(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic SLOAD key"), "{stdout}");
    assert!(!stdout.contains("symbolic Halmos compatibility cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_prank_propagates_callers, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_prank_propagates_callers because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPrank.t.sol",
        r#"
import "forge-std/Test.sol";

contract CallerProbe {
    function callers() external view returns (address, address) {
        return (msg.sender, tx.origin);
    }
}

contract SymbolicPrank is Test {
    CallerProbe probe;

    function setUp() public {
        probe = new CallerProbe();
    }

    function checkPrank(uint256) public {
        address alice = address(0xA11CE);
        address bob = address(0xB0B);

        vm.prank(alice);
        (address sender,) = probe.callers();
        assertEq(sender, alice);

        (sender,) = probe.callers();
        assertEq(sender, address(this));

        vm.startPrank(alice, bob);
        address origin;
        (sender, origin) = probe.callers();
        assertEq(sender, alice);
        assertEq(origin, bob);

        vm.stopPrank();
        (sender,) = probe.callers();
        assertEq(sender, address(this));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkPrank"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkPrank(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_assert_cheatcodes_find_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_assert_cheatcodes_find_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicVmAssert.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicVmAssert is Test {
    function checkVmAssert(uint256 x) public {
        vm.assertNotEq(x, uint256(42));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkVmAssert"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkVmAssert(uint256)"), "{stdout}");
    assert!(stdout.contains("args=[42]"), "{stdout}");
    assert!(!stdout.contains("counterexample did not replay"), "{stdout}");
});

forgetest_init!(symbolic_vm_recorded_logs_round_trip, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_recorded_logs_round_trip because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRecordedLogs.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicLogEmitter {
    event Helper(uint256 indexed topic, bytes data);

    function ok(uint256 topic, bytes memory data) external {
        emit Helper(topic, data);
    }

    function fail(uint256 topic, bytes memory data) external {
        emit Helper(topic, data);
        revert();
    }
}

contract SymbolicRecordedLogs is Test {
    event Local(uint256 indexed topic, bytes data);

    SymbolicLogEmitter emitter;

    function setUp() public {
        emitter = new SymbolicLogEmitter();
    }

    /// forge-config: default.symbolic.array_lengths = [2]
    function checkRecordedLogs(uint256 topic, bytes memory data) public {
        vm.recordLogs();

        emit Local(topic, data);
        emitter.ok(topic + 1, data);
        try emitter.fail(topic + 2, data) {} catch {}

        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertEq(logs.length, 2);

        assertEq(logs[0].topics.length, 2);
        assertEq(logs[0].topics[0], keccak256("Local(uint256,bytes)"));
        assertEq(logs[0].topics[1], bytes32(topic));
        assertEq(logs[0].emitter, address(this));

        bytes memory localData = abi.decode(logs[0].data, (bytes));
        assert(keccak256(localData) == keccak256(data));

        assertEq(logs[1].topics.length, 2);
        assertEq(logs[1].topics[0], keccak256("Helper(uint256,bytes)"));
        assertEq(logs[1].topics[1], bytes32(topic + 1));
        assertEq(logs[1].emitter, address(emitter));

        Vm.Log[] memory drained = vm.getRecordedLogs();
        assertEq(drained.length, 0);
    }

    /// forge-config: default.symbolic.array_lengths = [2]
    function checkRecordedLogsJson(uint256 topic, bytes memory data) public {
        vm.recordLogs();
        emit Local(topic, data);

        string memory json = vm.getRecordedLogsJson();
        assert(bytes(json).length > 0);

        Vm.Log[] memory drained = vm.getRecordedLogs();
        assertEq(drained.length, 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicRecordedLogs"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkRecordedLogs(uint256,bytes)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkRecordedLogsJson(uint256,bytes)"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.getRecordedLogsJson"), "{stdout}");
});

forgetest_init!(symbolic_vm_env_crypto_and_console_helpers, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_env_crypto_and_console_helpers because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicEnvCryptoConsole.t.sol",
        r#"
import "forge-std/Test.sol";
import "forge-std/console2.sol";

contract SymbolicEnvCryptoConsole is Test {
    function checkEnvCryptoConsole(uint256 x) public {
        assertTrue(vm.envExists("FOUNDRY_SYMBOLIC_ENV_PRESENT"));
        assertEq(vm.envUint("FOUNDRY_SYMBOLIC_ENV_UINT"), 42);
        assertEq(vm.envOr("FOUNDRY_SYMBOLIC_ENV_MISSING", uint256(7)), 7);
        assertEq(vm.envString("FOUNDRY_SYMBOLIC_ENV_STRING"), "hello");

        uint256[] memory values = vm.envUint("FOUNDRY_SYMBOLIC_ENV_UINTS", ",");
        assertEq(values.length, 3);
        assertEq(values[0], 1);
        assertEq(values[2], 3);

        string[] memory words = vm.envString("FOUNDRY_SYMBOLIC_ENV_STRINGS", ",");
        assertEq(words.length, 2);
        assertEq(words[1], "beta");

        bytes[] memory blobs = vm.envBytes("FOUNDRY_SYMBOLIC_ENV_BYTES_ARRAY", ",");
        assertEq(blobs.length, 2);
        assertEq(blobs[1], hex"cafe");

        uint256[] memory defaultValues = new uint256[](2);
        defaultValues[0] = 5;
        defaultValues[1] = 6;
        uint256[] memory missing = vm.envOr("FOUNDRY_SYMBOLIC_ENV_MISSING_ARRAY", ",", defaultValues);
        assertEq(missing.length, 2);
        assertEq(missing[1], 6);

        address keyAddress = vm.addr(1);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(1, keccak256("foundry-symbolic"));
        assertTrue(keyAddress != address(0));
        assertTrue(v == 27 || v == 28);
        assertTrue(r != bytes32(0));
        assertTrue(s != bytes32(0));

        console2.log("symbolic", x);
    }

    function checkKeyUtilities() public {
        address keyAddress = vm.addr(1);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(1, keccak256("foundry-symbolic"));
        assertTrue(keyAddress != address(0));
        assertTrue(v == 27 || v == 28);
        assertTrue(r != bytes32(0));
        assertTrue(s != bytes32(0));
        (bytes32 compactR, bytes32 vs) = vm.signCompact(1, keccak256("foundry-symbolic"));
        assertEq(compactR, r);
        assertTrue(vs != bytes32(0));
        address remembered = vm.rememberKey(2);
        assertEq(remembered, vm.addr(2));
        address[] memory wallets = vm.getWallets();
        assertEq(wallets.length, 1);
        assertEq(wallets[0], remembered);
        uint256 derived = vm.deriveKey("test test test test test test test test test test test junk", uint32(0));
        assertEq(vm.addr(derived), 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
        address[] memory derivedWallets = vm.rememberKeys(
            "test test test test test test test test test test test junk",
            "m/44'/60'/0'/0/",
            uint32(3)
        );
        assertEq(derivedWallets.length, 3);
        assertEq(derivedWallets[0], vm.addr(derived));
    }

    function checkBase64Utilities() public {
        assertEq(vm.toBase64(bytes("hello")), "aGVsbG8=");
        assertEq(vm.toBase64URL(hex"ffff"), "__8=");
    }

    function checkParseToStringUtilities() public {
        assertEq(vm.parseBytes("0x1234"), hex"1234");
        assertEq(vm.parseAddress(vm.toString(address(0xBEEF))), address(0xBEEF));
        assertEq(vm.parseUint(vm.toString(uint256(123))), 123);
        assertEq(vm.parseInt(vm.toString(int256(-5))), -5);
        assertEq(vm.parseBytes32(vm.toString(bytes32(uint256(0x12)))), bytes32(uint256(0x12)));
        assertTrue(vm.parseBool(vm.toString(true)));
    }

    function checkStringUtilities() public {
        assertEq(vm.toLowercase("AbC"), "abc");
        assertEq(vm.toUppercase("AbC"), "ABC");
        assertEq(vm.trim("  foundry  "), "foundry");
        assertEq(vm.replace("hello forge", "forge", "symbolic"), "hello symbolic");
        string[] memory parts = vm.split("a,b,c", ",");
        assertEq(parts.length, 3);
        assertEq(parts[1], "b");
        assertEq(vm.indexOf("foundry", "dry"), 4);
        assertTrue(vm.contains("foundry", "ound"));
    }
}
"#,
    );

    cmd.env("FOUNDRY_SYMBOLIC_ENV_PRESENT", "1");
    cmd.env("FOUNDRY_SYMBOLIC_ENV_UINT", "42");
    cmd.env("FOUNDRY_SYMBOLIC_ENV_STRING", "hello");
    cmd.env("FOUNDRY_SYMBOLIC_ENV_UINTS", "1,2,3");
    cmd.env("FOUNDRY_SYMBOLIC_ENV_STRINGS", "alpha,beta");
    cmd.env("FOUNDRY_SYMBOLIC_ENV_BYTES_ARRAY", "12,cafe");

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicEnvCryptoConsole"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkEnvCryptoConsole(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkKeyUtilities()"), "{stdout}");
    assert!(stdout.contains("[PASS] checkBase64Utilities()"), "{stdout}");
    assert!(stdout.contains("[PASS] checkParseToStringUtilities()"), "{stdout}");
    assert!(stdout.contains("[PASS] checkStringUtilities()"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_ffi_is_config_gated, |prj, cmd| {
    if !z3_available() {
        let _ =
            sh_eprintln!("skipping symbolic_vm_ffi_is_config_gated because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicFfiDisabled.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicFfiDisabled is Test {
    function checkFfiDisabled(uint256) public {
        string[] memory input = new string[](1);
        input[0] = "true";
        vm.ffi(input);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkFfiDisabled"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("symbolic ffi disabled"), "{stdout}");
});

forgetest_init!(symbolic_vm_ffi_success_when_enabled, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_ffi_success_when_enabled because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicFfiEnabled.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicFfiEnabled is Test {
    function checkFfiEnabled(uint256) public {
        string[] memory input = new string[](3);
        input[0] = "sh";
        input[1] = "-c";
        input[2] = "printf 0x1234";

        bytes memory output = vm.ffi(input);
        assertEq(output.length, 2);
        assertEq(uint8(output[0]), 0x12);
        assertEq(uint8(output[1]), 0x34);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--ffi", "--match-test", "checkFfiEnabled"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkFfiEnabled(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic ffi disabled"), "{stdout}");
});

forgetest_init!(symbolic_vm_etch_and_get_deployed_code, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_etch_and_get_deployed_code because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicEtch.t.sol",
        r#"
import "forge-std/Test.sol";

interface IEtchedHelper {
    function value() external pure returns (uint256);
}

contract EtchedHelper {
    function value() external pure returns (uint256) {
        return 99;
    }
}

contract SymbolicEtch is Test {
    function checkEtch(uint256) public {
        address target = address(0xBEEF);
        bytes memory code = vm.getDeployedCode("SymbolicEtch.t.sol:EtchedHelper");
        vm.etch(target, code);

        assertGt(target.code.length, 0);
        assertEq(IEtchedHelper(target).value(), 99);
    }

    function checkEtchSymbolicBytes(uint8 value) public {
        address target = address(0xCAFE);
        bytes memory code = abi.encodePacked(bytes1(0x60), bytes1(value), bytes1(0x00));
        vm.etch(target, code);

        assertEq(target.code.length, 3);

        bytes memory copied = new bytes(3);
        assembly {
            extcodecopy(target, add(copied, 0x20), 0, 3)
        }

        assertEq(copied[0], bytes1(0x60));
        assertEq(copied[1], bytes1(value));
        assertEq(copied[2], bytes1(0x00));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicEtch"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkEtch(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkEtchSymbolicBytes(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.etch"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.getCode artifact"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_extcodehash_distinguishes_empty_existing_account, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodehash_distinguishes_empty_existing_account because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCodeHash.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCodeHash is Test {
    function checkCodeHash(uint256) public {
        address target = address(0xBEEF);
        vm.etch(target, hex"");

        bytes32 emptyHash = keccak256(new bytes(0));
        assert(target.code.length == 0);
        assert(target.codehash == emptyHash);
        assert(address(0xCAFE).codehash == bytes32(0));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCodeHash"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCodeHash(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_extcodecopy_pads_partial_code_ranges, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodecopy_pads_partial_code_ranges because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeCopy.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeCopy is Test {
    function checkExtcodeCopy(uint256) public {
        address target = address(0xBEEF);
        vm.etch(target, hex"010203");

        bytes memory copied = new bytes(5);
        assembly {
            extcodecopy(target, add(copied, 0x20), 1, 5)
        }

        assert(uint8(copied[0]) == 2);
        assert(uint8(copied[1]) == 3);
        assert(uint8(copied[2]) == 0);
        assert(uint8(copied[3]) == 0);
        assert(uint8(copied[4]) == 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExtcodeCopy"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkExtcodeCopy(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_codecopy_accepts_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_codecopy_accepts_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCodeCopy.t.sol",
        r#"
contract SymbolicCodeCopy {
    function checkCodeCopy(uint16 offset) public pure {
        uint256 copied;
        uint256 size;
        assembly {
            size := codesize()
            codecopy(0, offset, 1)
            copied := mload(0)
        }

        if (offset >= size) {
            assert(copied == 0);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCodeCopy"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCodeCopy(uint16)"), "{stdout}");
    assert!(!stdout.contains("symbolic CODECOPY offset"), "{stdout}");
});

forgetest_init!(symbolic_extcodecopy_accepts_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodecopy_accepts_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeCopyOffset.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeCopyOffset is Test {
    function checkExtcodeCopyOffset(uint16 offset) public {
        address target = address(0xBEEF);
        vm.etch(target, hex"010203");

        bytes memory copied = new bytes(1);
        assembly {
            extcodecopy(target, add(copied, 0x20), offset, 1)
        }

        if (offset == 1) {
            assert(uint8(copied[0]) == 2);
        }
        if (offset >= 3) {
            assert(uint8(copied[0]) == 0);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExtcodeCopyOffset"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkExtcodeCopyOffset(uint16)"), "{stdout}");
    assert!(!stdout.contains("symbolic EXTCODECOPY offset"), "{stdout}");
});

forgetest_init!(symbolic_codecopy_accepts_bounded_symbolic_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_codecopy_accepts_bounded_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCodeCopySize.t.sol",
        r#"
contract SymbolicCodeCopySize {
    function checkCodeCopySize(uint8 rawSize) public pure {
        uint256 size = uint256(rawSize & 1);
        uint256 first;
        uint256 copied;
        assembly {
            codecopy(0x80, 0, 1)
            first := byte(0, mload(0x80))
            codecopy(0xa0, 0, size)
            copied := byte(0, mload(0xa0))
        }

        if (size == 1) {
            assert(copied == first);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCodeCopySize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkCodeCopySize(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic CODECOPY size"), "{stdout}");
});

forgetest_init!(symbolic_extcodecopy_accepts_bounded_symbolic_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodecopy_accepts_bounded_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeCopySize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeCopySize is Test {
    function checkExtcodeCopySize(uint8 rawSize) public {
        address target = address(0xBEEF);
        vm.etch(target, hex"010203");
        uint256 size = uint256(rawSize & 3);

        bytes memory copied = new bytes(3);
        assembly {
            extcodecopy(target, add(copied, 0x20), 0, size)
        }

        if (size == 3) {
            assertEq(uint8(copied[0]), 1);
            assertEq(uint8(copied[1]), 2);
            assertEq(uint8(copied[2]), 3);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExtcodeCopySize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkExtcodeCopySize(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic EXTCODECOPY size"), "{stdout}");
});

forgetest_init!(symbolic_selfdestruct_updates_account_overlay, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_selfdestruct_updates_account_overlay because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSelfdestruct.t.sol",
        r#"
import "forge-std/Test.sol";

contract Killable {
    receive() external payable {}

    function die(address payable beneficiary) external {
        selfdestruct(beneficiary);
    }
}

contract SymbolicSelfdestruct is Test {
    Killable killable;
    address payable beneficiary = payable(address(0xB0B));

    function setUp() public {
        killable = new Killable();
    }

    function checkSelfdestruct(uint256) public {
        vm.deal(address(killable), 7);

        killable.die(beneficiary);

        assert(address(killable).balance == 0);
        assert(beneficiary.balance == 7);
        assert(address(killable).code.length == 0);
        assert(address(killable).codehash == bytes32(0));
        assert(vm.getNonce(address(killable)) == 1);
    }
}
"#,
    );

    let stdout = cmd
        .args([
            "test",
            "--symbolic",
            "--evm-version",
            "shanghai",
            "--match-test",
            "checkSelfdestruct",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSelfdestruct(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_selfdestruct_accepts_symbolic_beneficiary, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_selfdestruct_accepts_symbolic_beneficiary because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSelfdestructBeneficiary.t.sol",
        r#"
import "forge-std/Test.sol";

contract Killable {
    receive() external payable {}

    function die(address payable beneficiary) external {
        selfdestruct(beneficiary);
    }
}

contract SymbolicSelfdestructBeneficiary is Test {
    Killable killable;

    function setUp() public {
        killable = new Killable();
    }

    function checkSelfdestructBeneficiary(address payable beneficiary) public {
        vm.assume(beneficiary != address(killable));
        vm.deal(address(killable), 7);

        killable.die(beneficiary);

        assert(address(killable).balance == 0);
        assert(beneficiary.balance == 7);
    }
}
"#,
    );

    let stdout = cmd
        .args([
            "test",
            "--symbolic",
            "--evm-version",
            "shanghai",
            "--match-test",
            "checkSelfdestructBeneficiary",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSelfdestructBeneficiary(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic SELFDESTRUCT beneficiary"), "{stdout}");
    assert!(!stdout.contains("symbolic BALANCE target"), "{stdout}");
});

forgetest_init!(symbolic_selfdestruct_cancun_reports_incomplete, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_selfdestruct_cancun_reports_incomplete because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSelfdestructCancun.t.sol",
        r#"
import "forge-std/Test.sol";

/// forge-config: default.evm_version = "cancun"

contract SymbolicSelfdestructCancun is Test {
    function checkSelfdestructCancun(address payable beneficiary) public {
        selfdestruct(beneficiary);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSelfdestructCancun"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("SELFDESTRUCT/EIP-6780 not modeled"), "{stdout}");
});

forgetest_init!(symbolic_precompiles_execute_concrete_inputs, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_precompiles_execute_concrete_inputs because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPrecompiles.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicPrecompiles is Test {
    function checkHashPrecompiles(uint256) public {
        assertEq(
            sha256(bytes("abc")),
            0xba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        );
        assertEq(ripemd160(bytes("abc")), bytes20(hex"8eb208f7e05d987a9b044a8e98c6b087f15a0bfc"));
    }

    function checkIdentityPrecompile(uint256) public {
        (bool ok, bytes memory out) = address(4).staticcall(hex"0102030405");
        assert(ok);
        assertEq(out, hex"0102030405");
    }

    function checkModexpPrecompile(uint256) public {
        bytes memory input = abi.encodePacked(uint256(1), uint256(1), uint256(1), bytes1(0x02), bytes1(0x05), bytes1(0x0d));
        (bool ok, bytes memory out) = address(5).staticcall(input);
        assert(ok);
        assertEq(out, hex"06");
    }

    function checkBn254Precompiles(uint256) public {
        bytes memory emptyPointPair = new bytes(128);
        (bool okAdd, bytes memory addOut) = address(6).staticcall(emptyPointPair);
        assert(okAdd);
        assertEq(addOut.length, 64);
        assertEq(keccak256(addOut), keccak256(new bytes(64)));

        bytes memory emptyMul = new bytes(96);
        (bool okMul, bytes memory mulOut) = address(7).staticcall(emptyMul);
        assert(okMul);
        assertEq(mulOut.length, 64);
        assertEq(keccak256(mulOut), keccak256(new bytes(64)));

        (bool okPair, bytes memory pairOut) = address(8).staticcall("");
        assert(okPair);
        assertEq(pairOut, abi.encode(uint256(1)));
    }

    function checkBlake2fPrecompile(uint256) public {
        bytes memory input = new bytes(213);
        (bool ok, bytes memory out) = address(9).staticcall(input);
        assert(ok);
        assertEq(out.length, 64);
        assertEq(keccak256(out), 0x209b39a06473bccc5b67b4f26619611f672409e4480527289d64c51b9a382bb6);

        (bool invalidOk, bytes memory invalidOut) = address(9).staticcall(hex"00");
        assert(!invalidOk);
        assertEq(invalidOut.length, 0);
    }

    function checkEcrecoverPrecompile(uint256) public {
        bytes32 digest = keccak256("foundry-symbolic-precompile");
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(1, digest);
        assertEq(ecrecover(digest, v, r, s), vm.addr(1));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicPrecompiles"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkHashPrecompiles(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkIdentityPrecompile(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkModexpPrecompile(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkBn254Precompiles(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkBlake2fPrecompile(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkEcrecoverPrecompile(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_hash_precompiles_accept_symbolic_input, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_hash_precompiles_accept_symbolic_input because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPrecompileInput.t.sol",
        r#"
contract SymbolicPrecompileInput {
    /// forge-config: default.symbolic.array_lengths = [2]
    function checkSymbolicHashDeterminism(bytes memory data) public {
        bytes32 shaA = sha256(data);
        bytes32 shaB = sha256(data);
        assert(shaA == shaB);

        bytes20 ripemdA = ripemd160(data);
        bytes20 ripemdB = ripemd160(data);
        assert(ripemdA == ripemdB);
    }

    function checkSymbolicEcrecoverDeterminism(
        bytes32 digest,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        address first = ecrecover(digest, v, r, s);
        address second = ecrecover(digest, v, r, s);

        assert(first == second);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicPrecompileInput"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicHashDeterminism(bytes)"), "{stdout}");
    assert!(
        stdout.contains("[PASS] checkSymbolicEcrecoverDeterminism(bytes32,uint8,bytes32,bytes32)"),
        "{stdout}"
    );
    assert!(!stdout.contains("symbolic precompile input"), "{stdout}");
});

forgetest_init!(symbolic_identity_precompile_accepts_symbolic_input, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_identity_precompile_accepts_symbolic_input because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicIdentityPrecompileInput.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicIdentityPrecompileInput is Test {
    /// forge-config: default.symbolic.array_lengths = [2]
    function checkSymbolicIdentity(bytes memory data) public {
        (bool ok, bytes memory out) = address(4).staticcall(data);
        assert(ok);
        assert(out.length == data.length);
        for (uint256 i; i < data.length; ++i) {
            assert(out[i] == data[i]);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicIdentity"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicIdentity(bytes)"), "{stdout}");
    assert!(!stdout.contains("symbolic precompile input"), "{stdout}");
});

forgetest_init!(symbolic_advanced_precompiles_accept_symbolic_payloads, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_advanced_precompiles_accept_symbolic_payloads because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicAdvancedPrecompileInput.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicAdvancedPrecompileInput is Test {
    function checkSymbolicModexp(bytes1 base) public {
        bytes memory input = abi.encodePacked(
            uint256(1),
            uint256(1),
            uint256(1),
            base,
            bytes1(0x05),
            bytes1(0x0d)
        );
        (bool okA, bytes memory outA) = address(5).staticcall(input);
        (bool okB, bytes memory outB) = address(5).staticcall(input);

        assert(okA);
        assert(okB);
        assertEq(outA.length, 1);
        assertEq(outA.length, outB.length);
        assertEq(keccak256(outA), keccak256(outB));
    }

    function checkSymbolicBlake2f(bytes1 tweak) public {
        bytes memory input = new bytes(213);
        input[10] = tweak;

        (bool okA, bytes memory outA) = address(9).staticcall(input);
        (bool okB, bytes memory outB) = address(9).staticcall(input);

        assert(okA);
        assert(okB);
        assertEq(outA.length, 64);
        assertEq(outA.length, outB.length);
        assertEq(keccak256(outA), keccak256(outB));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicAdvancedPrecompileInput"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicModexp(bytes1)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkSymbolicBlake2f(bytes1)"), "{stdout}");
    assert!(!stdout.contains("symbolic precompile input"), "{stdout}");
    assert!(!stdout.contains("symbolic precompile length header"), "{stdout}");
});

forgetest_init!(symbolic_precompiles_accept_symbolic_input_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_precompiles_accept_symbolic_input_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPrecompileInputSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicPrecompileInputSize is Test {
    function checkSymbolicIdentityInputSize(uint256 size) public {
        vm.assume(size <= 4);

        bool ok;
        uint256 rdsize;
        bytes32 word;
        assembly {
            let inPtr := mload(0x40)
            let outPtr := add(inPtr, 0x40)
            mstore(inPtr, 0x0102030400000000000000000000000000000000000000000000000000000000)
            mstore(outPtr, 0)
            ok := staticcall(gas(), 4, inPtr, size, outPtr, 4)
            rdsize := returndatasize()
            word := mload(outPtr)
        }

        bytes32 expected;
        if (size > 0) expected |= bytes32(uint256(0x01) << 248);
        if (size > 1) expected |= bytes32(uint256(0x02) << 240);
        if (size > 2) expected |= bytes32(uint256(0x03) << 232);
        if (size > 3) expected |= bytes32(uint256(0x04) << 224);

        assert(ok);
        assertEq(rdsize, size);
        assertEq(word, expected);
    }

    function checkSymbolicShaInputSize(uint256 size) public {
        vm.assume(size <= 4);

        bool okA;
        bool okB;
        bytes32 outA;
        bytes32 outB;
        assembly {
            let inPtr := mload(0x40)
            let outPtrA := add(inPtr, 0x40)
            let outPtrB := add(inPtr, 0x80)
            mstore(inPtr, 0x0102030400000000000000000000000000000000000000000000000000000000)
            okA := staticcall(gas(), 2, inPtr, size, outPtrA, 32)
            okB := staticcall(gas(), 2, inPtr, size, outPtrB, 32)
            outA := mload(outPtrA)
            outB := mload(outPtrB)
        }

        assert(okA);
        assert(okB);
        assertEq(outA, outB);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicPrecompileInputSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicIdentityInputSize(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkSymbolicShaInputSize(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic precompile CALL input size"), "{stdout}");
});

forgetest_init!(symbolic_vm_set_blockhash, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_vm_set_blockhash because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicBlockhash.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBlockhash is Test {
    function checkSetBlockhash(uint256) public {
        bytes32 previousHash = bytes32(uint256(0x1234));
        vm.roll(300);
        vm.setBlockhash(299, previousHash);
        vm.setBlockhash(300, bytes32(uint256(0xdead)));
        vm.setBlockhash(43, bytes32(uint256(0xbeef)));

        assertEq(blockhash(299), previousHash);
        assertEq(blockhash(300), bytes32(0));
        assertEq(blockhash(43), bytes32(0));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSetBlockhash"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSetBlockhash(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_blockhash_accepts_symbolic_number, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_blockhash_accepts_symbolic_number because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBlockhashNumber.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBlockhashNumber is Test {
    function checkSymbolicBlockhashNumber(uint256 blockNumber) public {
        bytes32 previousHash = bytes32(uint256(0x1234));
        vm.roll(300);
        vm.setBlockhash(299, previousHash);

        bytes32 hash = blockhash(blockNumber);
        if (hash == previousHash) {
            assertEq(blockNumber, 299);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicBlockhashNumber"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicBlockhashNumber(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic BLOCKHASH number"), "{stdout}");
});

forgetest_init!(symbolic_vm_set_blockhash_accepts_symbolic_hash, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_set_blockhash_accepts_symbolic_hash because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBlockhashValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBlockhashValue is Test {
    function checkSymbolicBlockhashValue(bytes32 blockHash) public {
        vm.roll(300);
        vm.setBlockhash(299, blockHash);

        assertEq(blockhash(299), blockHash);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicBlockhashValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicBlockhashValue(bytes32)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.setBlockhash hash"), "{stdout}");
});

forgetest_init!(symbolic_vm_block_environment_breadth, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_block_environment_breadth because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBlockEnvironment.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBlockEnvironment is Test {
    function checkBlockEnvironment(uint256) public {
        bytes32 randomness = bytes32(uint256(0xabc));

        vm.warp(123);
        assertEq(vm.getBlockTimestamp(), 123);
        assertEq(block.timestamp, 123);

        vm.txGasPrice(7);
        assertEq(tx.gasprice, 7);

        vm.prevrandao(randomness);
        assertEq(block.prevrandao, uint256(randomness));

        vm.blobBaseFee(11);
        assertEq(block.blobbasefee, 11);
        assertEq(vm.getBlobBaseFee(), 11);

        bytes32[] memory hashes = new bytes32[](2);
        hashes[0] = bytes32(uint256(0x1111));
        hashes[1] = bytes32(uint256(0x2222));
        vm.blobhashes(hashes);

        assertEq(blobhash(0), hashes[0]);
        assertEq(blobhash(1), hashes[1]);
        assertEq(blobhash(2), bytes32(0));

        bytes32[] memory got = vm.getBlobhashes();
        assertEq(got.length, 2);
        assertEq(got[0], hashes[0]);
        assertEq(got[1], hashes[1]);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkBlockEnvironment"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkBlockEnvironment(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_uses_prepared_executor_environment, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_uses_prepared_executor_environment because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPreparedEnvironment.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicPreparedEnvironment is Test {
    function setUp() public {
        vm.chainId(424242);
        vm.roll(12345);
        vm.warp(67890);
        vm.fee(77);
        vm.prevrandao(bytes32(uint256(99)));
        vm.coinbase(address(0xBEEF));
        vm.txGasPrice(66);
    }

    function checkPreparedEnvironment(uint256 x) public {
        if (x > 1) return;

        assertEq(block.chainid, 424242);
        assertEq(block.number, 12345);
        assertEq(block.timestamp, 67890);
        assertEq(block.basefee, 77);
        assertEq(block.prevrandao, 99);
        assertEq(block.coinbase, address(0xBEEF));
        assertEq(tx.gasprice, 66);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkPreparedEnvironment"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkPreparedEnvironment(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_vm_state_snapshots, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_vm_state_snapshots because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicStateSnapshots.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicStateSnapshots is Test {
    uint256 value;

    function checkStateSnapshots(uint256) public {
        value = 1;
        vm.deal(address(this), 5 ether);

        uint256 snapshotId = vm.snapshotState();
        value = 2;
        vm.deal(address(this), 8 ether);

        assertTrue(vm.revertToState(snapshotId));
        assertEq(value, 1);
        assertEq(address(this).balance, 5 ether);

        assertTrue(vm.deleteStateSnapshot(snapshotId));
        assertFalse(vm.revertToState(snapshotId));

        uint256 legacySnapshot = vm.snapshot();
        value = 3;

        assertTrue(vm.revertToAndDelete(legacySnapshot));
        assertEq(value, 1);
        assertFalse(vm.revertTo(legacySnapshot));

        uint256 deletedSnapshot = vm.snapshotState();
        assertTrue(vm.deleteSnapshot(deletedSnapshot));
        assertFalse(vm.revertToState(deletedSnapshot));

        uint256 clearedSnapshot = vm.snapshotState();
        vm.deleteStateSnapshots();
        assertFalse(vm.revertToState(clearedSnapshot));
    }

    receive() external payable {}
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkStateSnapshots"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkStateSnapshots(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_vm_random_bytes, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_vm_random_bytes because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicRandomBytes.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicRandomBytes is Test {
    function checkRandomBytes(uint256) public {
        bytes memory data = vm.randomBytes(3);

        assertEq(data.length, 3);
        vm.assume(data[0] == bytes1(0x11));
        vm.assume(data[1] == bytes1(0x22));
        vm.assume(data[2] == bytes1(0x33));

        assertTrue(data[0] == bytes1(0x11));
        assertTrue(data[1] == bytes1(0x22));
        assertTrue(data[2] == bytes1(0x33));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkRandomBytes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkRandomBytes(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_vm_random_bytes_accepts_bounded_symbolic_length, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_random_bytes_accepts_bounded_symbolic_length because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRandomBytesLength.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicRandomBytesLength is Test {
    function checkRandomBytesSymbolicLength(uint8 n) public {
        uint256 len = uint256(n);
        vm.assume(len <= 3);

        bytes memory data = vm.randomBytes(len);

        assertEq(data.length, len);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkRandomBytesSymbolicLength"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkRandomBytesSymbolicLength(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic randomBytes len"), "{stdout}");
    assert!(!stdout.contains("symbolic randomBytes length"), "{stdout}");
});

forgetest_init!(symbolic_cheatcodes_accept_constrained_scalar_args, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_cheatcodes_accept_constrained_scalar_args because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicConstrainedCheatcodes.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicConstrainedCheatcodes is Test {
    function checkConstrainedDeal(address target, uint256 amount) public {
        vm.assume(target == address(0xbeef));
        vm.assume(amount == 7);

        vm.deal(target, amount);

        assertEq(address(0xbeef).balance, 7);
    }

    function checkConstrainedRandomBytes(uint16 len) public {
        vm.assume(len == 3);

        bytes memory data = vm.randomBytes(len);

        assertEq(data.length, 3);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicConstrainedCheatcodes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkConstrainedDeal(address,uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkConstrainedRandomBytes(uint16)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.deal target"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.deal value"), "{stdout}");
    assert!(!stdout.contains("symbolic randomBytes len"), "{stdout}");
});

forgetest_init!(symbolic_cheatcodes_accept_bounded_symbolic_input_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_cheatcodes_accept_bounded_symbolic_input_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCheatcodeInputSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCheatcodeInputSize is Test {
    function checkLowLevelAssumeSize(uint256 size, bool condition) public {
        vm.assume(size >= 36);
        vm.assume(size <= 68);

        bytes memory data = abi.encodeWithSelector(bytes4(keccak256("assume(bool)")), condition);
        address cheatcode = address(vm);
        bool ok;
        assembly {
            ok := call(gas(), cheatcode, 0, add(data, 32), size, 0, 0)
        }

        assert(ok);
        assertTrue(condition);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkLowLevelAssumeSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkLowLevelAssumeSize(uint256,bool)"), "{stdout}");
    assert!(!stdout.contains("symbolic cheatcode CALL input size"), "{stdout}");
});

forgetest_init!(symbolic_svm_creator_breadth, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_svm_creator_breadth because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicSvmCreators.t.sol",
        r#"
interface Svm {
    function createUint8(string calldata name) external returns (uint8);
    function createInt16(string calldata name) external returns (int16);
    function createBytes2(string calldata name) external returns (bytes2);
    function createBytes(string calldata name) external returns (bytes memory);
    function createBytes(uint256 len, string calldata name) external returns (bytes memory);
    function createString(string calldata name) external returns (string memory);
    function createString(uint256 len, string calldata name) external returns (string memory);
}

contract SymbolicSvmCreators {
    address constant SVM_ADDRESS = address(0xF3993A62377BCd56AE39D773740A5390411E8BC9);

    function checkSvmCreators(uint256) public {
        uint8 small = Svm(SVM_ADDRESS).createUint8("small");
        int16 signed = Svm(SVM_ADDRESS).createInt16("signed");
        bytes2 fixedBytes = Svm(SVM_ADDRESS).createBytes2("fixedBytes");
        bytes memory data = Svm(SVM_ADDRESS).createBytes("data");
        bytes memory sizedData = Svm(SVM_ADDRESS).createBytes(5, "sizedData");
        string memory text = Svm(SVM_ADDRESS).createString("text");
        string memory sizedText = Svm(SVM_ADDRESS).createString(3, "sizedText");

        assert(uint256(small) < 256);
        assert(signed == signed);
        assert(data.length == 2);
        assert(sizedData.length == 5);
        assert(bytes(text).length == 2);
        assert(bytes(sizedText).length == 3);
        assert(fixedBytes == fixedBytes);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSvmCreators"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSvmCreators(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic Halmos compatibility cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_invariant_finds_single_step_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_invariant_finds_single_step_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInvariantSingle.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCounterTarget {
    uint256 public value;

    function set(uint256 x) external {
        if (x == 7) {
            value = 11;
        }
    }
}

contract SymbolicInvariantSingle is Test {
    SymbolicCounterTarget target;

    function setUp() public {
        target = new SymbolicCounterTarget();
        targetContract(address(target));
    }

    function invariant_counterNeverEleven() public view {
        assert(target.value() != 11);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "invariant_counterNeverEleven"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("invariant_counterNeverEleven()"), "{stdout}");
    assert!(stdout.contains("set(uint256)"), "{stdout}");
    assert!(stdout.contains("args=[7]"), "{stdout}");
    assert!(!stdout.contains("No contracts to fuzz"), "{stdout}");
});

forgetest_init!(symbolic_invariant_respects_sequence_depth, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_invariant_respects_sequence_depth because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInvariantDepth.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicDepthTarget {
    uint256 public value;

    function arm(uint256 x) external {
        if (x == 1) {
            value = 1;
        }
    }

    function trip(uint256 x) external {
        if (value == 1 && x == 2) {
            value = 2;
        }
    }
}

contract SymbolicInvariantDepth is Test {
    SymbolicDepthTarget target;

    function setUp() public {
        target = new SymbolicDepthTarget();
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_valueBelowTwo() public view {
        assert(target.value() < 2);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "invariant_valueBelowTwo"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("arm(uint256)"), "{stdout}");
    assert!(stdout.contains("trip(uint256)"), "{stdout}");
    assert!(stdout.contains("args=[1]"), "{stdout}");
    assert!(stdout.contains("args=[2]"), "{stdout}");
});

forgetest_init!(symbolic_invariant_uses_target_sender, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_invariant_uses_target_sender because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInvariantSender.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicSenderTarget {
    address public lastSender;

    function touch(uint256 x) external {
        if (x == 3) {
            lastSender = msg.sender;
        }
    }
}

contract SymbolicInvariantSender is Test {
    SymbolicSenderTarget target;
    address constant BOB = address(0xB0B);

    function setUp() public {
        target = new SymbolicSenderTarget();
        targetContract(address(target));
        targetSender(BOB);
    }

    function invariant_senderIsNotBob() public view {
        assert(target.lastSender() != BOB);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "invariant_senderIsNotBob"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("touch(uint256)"), "{stdout}");
    assert!(
        stdout.to_lowercase().contains("sender=0x0000000000000000000000000000000000000b0b"),
        "{stdout}"
    );
});

forgetest_init!(symbolic_vm_expect_revert_matches_external_reverts, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_revert_matches_external_reverts because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectRevert.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedReverter {
    error Custom(uint256 value);

    function failWithCustom(uint256 value) external pure {
        revert Custom(value);
    }

    function failPanic() external pure {
        assert(false);
    }
}

contract SymbolicExpectRevert is Test {
    SymbolicExpectedReverter helper;

    function setUp() public {
        helper = new SymbolicExpectedReverter();
    }

    function checkExpectRevert(uint256) public {
        vm.expectRevert(SymbolicExpectedReverter.Custom.selector);
        helper.failWithCustom(7);

        vm.expectRevert(abi.encodeWithSelector(SymbolicExpectedReverter.Custom.selector, uint256(9)));
        helper.failWithCustom(9);

        vm.expectRevert(bytes4(0x4e487b71));
        helper.failPanic();
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExpectRevert"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkExpectRevert(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_revert_missing_is_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_revert_missing_is_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectRevertMissing.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedNoop {
    function noFail() external pure {}
}

contract SymbolicExpectRevertMissing is Test {
    SymbolicExpectedNoop helper;

    function setUp() public {
        helper = new SymbolicExpectedNoop();
    }

    function checkMissingExpectedRevert(uint256) public {
        vm.expectRevert();
        helper.noFail();
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMissingExpectedRevert"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkMissingExpectedRevert(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_revert_mismatch_is_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_revert_mismatch_is_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectRevertMismatch.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedMismatchReverter {
    error Custom(uint256 value);

    function failWithCustom(uint256 value) external pure {
        revert Custom(value);
    }
}

contract SymbolicExpectRevertMismatch is Test {
    SymbolicExpectedMismatchReverter helper;

    function setUp() public {
        helper = new SymbolicExpectedMismatchReverter();
    }

    function checkMismatchedExpectedRevert(uint256) public {
        vm.expectRevert(abi.encodeWithSelector(SymbolicExpectedMismatchReverter.Custom.selector, uint256(1)));
        helper.failWithCustom(2);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMismatchedExpectedRevert"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkMismatchedExpectedRevert(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_revert_accepts_symbolic_data, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_revert_accepts_symbolic_data because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectRevertSymbolicData.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedSymbolicReverter {
    error Custom(uint256 value);

    function failWithCustom(uint256 value) external pure {
        revert Custom(value);
    }

    function failWithSelector(bytes4 selector) external pure {
        assembly {
            mstore(0, selector)
            revert(0, 4)
        }
    }
}

contract SymbolicExpectRevertSymbolicData is Test {
    SymbolicExpectedSymbolicReverter helper;

    function setUp() public {
        helper = new SymbolicExpectedSymbolicReverter();
    }

    function checkSymbolicExpectedRevertPayload(uint256 value) public {
        vm.expectRevert(abi.encodeWithSelector(SymbolicExpectedSymbolicReverter.Custom.selector, value));
        helper.failWithCustom(value);
    }

    function checkSymbolicExpectedRevertSelector(bytes4 selector) public {
        vm.expectRevert(selector);
        helper.failWithSelector(selector);
    }

    function checkSymbolicExpectedReverter(address reverter) public {
        vm.assume(reverter == address(helper));
        vm.expectRevert(reverter);
        helper.failWithCustom(9);
    }

    function checkSymbolicExpectedRevertMismatch(uint256 value) public {
        vm.expectRevert(abi.encodeWithSelector(SymbolicExpectedSymbolicReverter.Custom.selector, uint256(7)));
        helper.failWithCustom(value);
    }
}
"#,
    );

    let stdout = cmd
        .args([
            "test",
            "--symbolic",
            "--match-contract",
            "SymbolicExpectRevertSymbolicData",
            "--match-test",
            "checkSymbolicExpectedRevertPayload|checkSymbolicExpectedRevertSelector|checkSymbolicExpectedReverter",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSymbolicExpectedRevertPayload(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkSymbolicExpectedRevertSelector(bytes4)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkSymbolicExpectedReverter(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.expectRevert"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_revert_symbolic_data_mismatch_fails, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_revert_symbolic_data_mismatch_fails because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectRevertSymbolicMismatch.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedSymbolicMismatchReverter {
    error Custom(uint256 value);

    function failWithCustom(uint256 value) external pure {
        revert Custom(value);
    }
}

contract SymbolicExpectRevertSymbolicMismatch is Test {
    SymbolicExpectedSymbolicMismatchReverter helper;

    function setUp() public {
        helper = new SymbolicExpectedSymbolicMismatchReverter();
    }

    function checkSymbolicExpectedRevertMismatch(uint256 value) public {
        vm.expectRevert(abi.encodeWithSelector(SymbolicExpectedSymbolicMismatchReverter.Custom.selector, uint256(7)));
        helper.failWithCustom(value);
    }

    function checkSymbolicExpectedReverterMismatch(address reverter) public {
        vm.expectRevert(reverter);
        helper.failWithCustom(7);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicExpectedRevertMismatch"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkSymbolicExpectedRevertMismatch(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic expected revert data"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkSymbolicExpectedReverterMismatch"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkSymbolicExpectedReverterMismatch(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.expectRevert"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_emit_matches_external_logs, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_emit_matches_external_logs because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectEmit.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicEmitter {
    event Seen(address indexed who, uint256 indexed id, uint256 value);

    function fire(address who, uint256 id, uint256 value) external {
        emit Seen(who, id, value);
    }
}

contract SymbolicExpectEmit is Test {
    event Seen(address indexed who, uint256 indexed id, uint256 value);

    SymbolicEmitter emitter;

    function setUp() public {
        emitter = new SymbolicEmitter();
    }

    function checkExpectEmit(uint256) public {
        vm.expectEmit(true, true, false, true, address(emitter));
        emit Seen(address(0xB0B), 7, 9);
        emitter.fire(address(0xB0B), 7, 9);
    }

    function checkExpectEmitSymbolicEmitter(address expectedEmitter) public {
        vm.assume(expectedEmitter == address(emitter));
        vm.expectEmit(true, true, false, true, expectedEmitter);
        emit Seen(address(0xB0B), 7, 9);
        emitter.fire(address(0xB0B), 7, 9);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExpectEmit"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkExpectEmit(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkExpectEmitSymbolicEmitter(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.expectEmit"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_emit_mismatch_is_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_emit_mismatch_is_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectEmitMismatch.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicMismatchEmitter {
    event Seen(address indexed who, uint256 indexed id, uint256 value);

    function fire(address who, uint256 id, uint256 value) external {
        emit Seen(who, id, value);
    }
}

contract SymbolicExpectEmitMismatch is Test {
    event Seen(address indexed who, uint256 indexed id, uint256 value);

    SymbolicMismatchEmitter emitter;

    function setUp() public {
        emitter = new SymbolicMismatchEmitter();
    }

    function checkMismatchedExpectEmit(uint256) public {
        vm.expectEmit(true, true, false, true, address(emitter));
        emit Seen(address(0xB0B), 7, 9);
        emitter.fire(address(0xB0B), 8, 9);
    }

    function checkMismatchedExpectEmitSymbolicEmitter(address expectedEmitter) public {
        vm.expectEmit(true, true, false, true, expectedEmitter);
        emit Seen(address(0xB0B), 7, 9);
        emitter.fire(address(0xB0B), 7, 9);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMismatchedExpectEmit"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkMismatchedExpectEmit(uint256)"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMismatchedExpectEmitSymbolicEmitter"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkMismatchedExpectEmitSymbolicEmitter(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.expectEmit"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_call_matches_and_reports_missing, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_call_matches_and_reports_missing because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectCall.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedCallTarget {
    function ping(uint256 value) external pure returns (uint256) {
        return value + 1;
    }
}

contract SymbolicExpectCall is Test {
    SymbolicExpectedCallTarget target;

    function setUp() public {
        target = new SymbolicExpectedCallTarget();
    }

    function checkExpectCallMatches(uint256) public {
        vm.expectCall(
            address(target),
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(7))
        );
        assertEq(target.ping(7), 8);

        vm.expectCall(address(target), 0, abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(9)), 1);
        assertEq(target.ping(9), 10);

        vm.expectCall(
            address(target),
            0,
            uint64(50000),
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(13))
        );
        assertEq(target.ping{gas: 50000}(13), 14);

        vm.expectCallMinGas(
            address(target),
            0,
            uint64(25000),
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(14))
        );
        assertEq(target.ping{gas: 50000}(14), 15);
    }

    function checkExpectCallSymbolicCallee(address expectedCallee) public {
        vm.assume(expectedCallee == address(target));
        vm.expectCall(
            expectedCallee,
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(7))
        );
        assertEq(target.ping(7), 8);
    }

    function checkExpectCallMissing(uint256) public {
        vm.expectCall(
            address(target),
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(11))
        );
    }

    function checkSymbolicCalleeExpectedCallMismatch(address expectedCallee) public {
        vm.expectCall(
            expectedCallee,
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(7))
        );
        assertEq(target.ping(7), 8);
    }

    function checkExpectCallMinGasMissing(uint256) public {
        vm.expectCallMinGas(
            address(target),
            0,
            uint64(60000),
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(15))
        );
        assertEq(target.ping{gas: 50000}(15), 16);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExpectCallMatches"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkExpectCallMatches(uint256)"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkExpectCallSymbolicCallee"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkExpectCallSymbolicCallee(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.expectCall"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkExpectCallMissing"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkExpectCallMissing(uint256)"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCalleeExpectedCallMismatch"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkSymbolicCalleeExpectedCallMismatch(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.expectCall"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkExpectCallMinGasMissing"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkExpectCallMinGasMissing(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_vm_mock_call_returns_and_reverts, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_mock_call_returns_and_reverts because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMockCall.t.sol",
        r#"
import "forge-std/Test.sol";

interface IMockedTarget {
    function value(uint256 input) external returns (uint256);
}

contract SymbolicMockCall is Test {
    function checkMockCall(uint256) public {
        address target = address(0x1234);

        vm.mockCall(
            target,
            abi.encodeWithSelector(IMockedTarget.value.selector, uint256(1)),
            abi.encode(uint256(99))
        );
        assertEq(IMockedTarget(target).value(1), 99);

        vm.mockCallRevert(
            target,
            abi.encodeWithSelector(IMockedTarget.value.selector, uint256(2)),
            abi.encodeWithSignature("Error(string)", "mocked")
        );
        (bool ok, bytes memory data) =
            target.call(abi.encodeWithSelector(IMockedTarget.value.selector, uint256(2)));
        assertFalse(ok);
        assertGt(data.length, 0);
    }

    function checkMockCallSymbolicCallee(address mocked) public {
        address target = address(0x1234);
        vm.assume(mocked == target);

        vm.mockCall(
            mocked,
            abi.encodeWithSelector(IMockedTarget.value.selector, uint256(1)),
            abi.encode(uint256(99))
        );
        assertEq(IMockedTarget(target).value(1), 99);
    }

    function checkSymbolicCalleeMockMismatch(address mocked) public {
        address target = address(0x1234);

        vm.mockCall(
            mocked,
            abi.encodeWithSelector(IMockedTarget.value.selector, uint256(1)),
            abi.encode(uint256(99))
        );
        (bool ok, bytes memory data) =
            target.call(abi.encodeWithSelector(IMockedTarget.value.selector, uint256(1)));
        uint256 value = data.length == 32 ? abi.decode(data, (uint256)) : 0;
        assertTrue(ok);
        assertEq(value, 99);
    }

    function checkSelectorMockCallsAndClear(uint256 input) public {
        address target = address(0x4567);
        bytes[] memory returnValues = new bytes[](2);
        returnValues[0] = abi.encode(uint256(100));
        returnValues[1] = abi.encode(uint256(200));

        vm.mockCalls(target, abi.encodePacked(IMockedTarget.value.selector), returnValues);
        assertEq(IMockedTarget(target).value(input), 100);
        assertEq(IMockedTarget(target).value(1), 200);
        assertEq(IMockedTarget(target).value(2), 200);

        vm.clearMockedCalls();
        (bool ok, bytes memory data) =
            target.call(abi.encodeWithSelector(IMockedTarget.value.selector, input));
        assertTrue(ok);
        assertEq(data.length, 0);
    }

    function checkMockCallsAcceptsSymbolicData(address mocked, uint256 input) public {
        address target = address(0x4567);
        vm.assume(mocked == target);
        vm.assume(input < type(uint256).max - 2);

        bytes[] memory returnValues = new bytes[](2);
        returnValues[0] = abi.encode(input + 1);
        returnValues[1] = abi.encode(input + 2);

        vm.mockCalls(
            mocked,
            abi.encodeWithSelector(IMockedTarget.value.selector, input),
            returnValues
        );
        assertEq(IMockedTarget(target).value(input), input + 1);
        assertEq(IMockedTarget(target).value(input), input + 2);
        assertEq(IMockedTarget(target).value(input), input + 2);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMockCall"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkMockCall(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkMockCallSymbolicCallee(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.mockCall"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkSelectorMockCallsAndClear"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkSelectorMockCallsAndClear(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMockCallsAcceptsSymbolicData"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(
        stdout.contains("[PASS] checkMockCallsAcceptsSymbolicData(address,uint256)"),
        "{stdout}"
    );
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.mockCalls"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCalleeMockMismatch"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkSymbolicCalleeMockMismatch(address)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.mockCall"), "{stdout}");
});

forgetest_init!(symbolic_vm_call_expectations_allow_symbolic_value_when_unpinned, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_call_expectations_allow_symbolic_value_when_unpinned because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicUnpinnedCallValue.t.sol",
        r#"
import "forge-std/Test.sol";

interface IValueTarget {
    function ping(uint256 input) external payable returns (uint256);
}

contract ValueTarget {
    function ping(uint256 input) external payable returns (uint256) {
        return input + msg.value;
    }
}

contract SymbolicUnpinnedCallValue is Test {
    ValueTarget target;

    function setUp() public {
        target = new ValueTarget();
    }

    function checkExpectCallAllowsSymbolicValue(uint8 amount) public {
        vm.assume(amount <= 1);
        vm.deal(address(this), 1);

        vm.expectCall(
            address(target),
            abi.encodeWithSelector(ValueTarget.ping.selector, uint256(7))
        );

        assertEq(target.ping{value: amount}(7), 7 + amount);
    }

    function checkMockCallAllowsSymbolicValue(uint8 amount) public {
        vm.assume(amount <= 1);
        address mocked = address(0xBEEF);

        vm.mockCall(
            mocked,
            abi.encodeWithSelector(IValueTarget.ping.selector, uint256(3)),
            abi.encode(uint256(44))
        );

        assertEq(IValueTarget(mocked).ping{value: amount}(3), 44);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicUnpinnedCallValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkExpectCallAllowsSymbolicValue(uint8)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkMockCallAllowsSymbolicValue(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic expected call value"), "{stdout}");
    assert!(!stdout.contains("symbolic mocked call value"), "{stdout}");
});

forgetest_init!(symbolic_vm_call_expectations_branch_symbolic_pinned_value, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_call_expectations_branch_symbolic_pinned_value because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPinnedCallValue.t.sol",
        r#"
import "forge-std/Test.sol";

interface IPinnedValueTarget {
    function ping(uint256 input) external payable returns (uint256);
}

contract PinnedValueTarget {
    function ping(uint256 input) external payable returns (uint256) {
        return input + msg.value;
    }
}

contract SymbolicPinnedCallValue is Test {
    PinnedValueTarget target;

    function setUp() public {
        target = new PinnedValueTarget();
    }

    function checkExpectCallPinnedValueFindsMismatch(uint8 amount) public {
        vm.assume(amount <= 1);
        vm.deal(address(this), 1);

        vm.expectCall(
            address(target),
            uint256(1),
            abi.encodeWithSelector(PinnedValueTarget.ping.selector, uint256(7)),
            1
        );

        assertEq(target.ping{value: amount}(7), 7 + amount);
    }

    function checkMockCallPinnedValueMatches(uint8 amount) public {
        vm.assume(amount == 1);
        vm.deal(address(this), 1);
        address mocked = address(0xCAFE);

        vm.mockCall(
            mocked,
            uint256(1),
            abi.encodeWithSelector(IPinnedValueTarget.ping.selector, uint256(3)),
            abi.encode(uint256(44))
        );

        assertEq(IPinnedValueTarget(mocked).ping{value: amount}(3), 44);
    }

    function checkMockCallPinnedValueFindsMismatch(uint8 amount) public {
        vm.assume(amount <= 1);
        vm.deal(address(this), 1);
        address mocked = address(0xBEEF);

        vm.mockCall(
            mocked,
            uint256(1),
            abi.encodeWithSelector(IPinnedValueTarget.ping.selector, uint256(3)),
            abi.encode(uint256(44))
        );

        (bool ok, bytes memory data) = mocked.call{value: amount}(
            abi.encodeWithSelector(IPinnedValueTarget.ping.selector, uint256(3))
        );
        assertTrue(ok);
        assertEq(data.length, 32);
        assertEq(abi.decode(data, (uint256)), 44);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExpectCallPinnedValueFindsMismatch"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkExpectCallPinnedValueFindsMismatch(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic expected call value"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMockCallPinnedValueMatches"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkMockCallPinnedValueMatches(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic mocked call value"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMockCallPinnedValueFindsMismatch"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkMockCallPinnedValueFindsMismatch(uint8)"), "{stdout}");
    assert!(!stdout.contains("symbolic mocked call value"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_and_mock_call_accept_symbolic_data, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_and_mock_call_accept_symbolic_data because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCallDataCheatcodes.t.sol",
        r#"
import "forge-std/Test.sol";

interface ISymbolicDataTarget {
    function value(uint256 input) external returns (uint256);
}

contract SymbolicDataTarget {
    function value(uint256 input) external pure returns (uint256) {
        return input;
    }
}

contract SymbolicFunctionMockTarget {
    function value(uint256 input) external pure returns (uint256) {
        return input + 10;
    }
}

contract SymbolicCallDataCheatcodes is Test {
    SymbolicDataTarget target;
    SymbolicFunctionMockTarget functionTarget;

    function setUp() public {
        target = new SymbolicDataTarget();
        functionTarget = new SymbolicFunctionMockTarget();
    }

    function checkExpectCallAcceptsSymbolicData(uint256 input) public {
        vm.expectCall(
            address(target),
            abi.encodeWithSelector(SymbolicDataTarget.value.selector, input)
        );

        assertEq(target.value(input), input);
    }

    function checkMockCallAcceptsSymbolicDataAndReturn(uint256 input) public {
        address mocked = address(0xDADA);

        vm.mockCall(
            mocked,
            abi.encodeWithSelector(ISymbolicDataTarget.value.selector, input),
            abi.encode(input + 1)
        );

        assertEq(ISymbolicDataTarget(mocked).value(input), input + 1);
    }

    function checkMockCallAcceptsSymbolicBytes4Selector(bytes4 selector) public {
        vm.assume(selector == ISymbolicDataTarget.value.selector);
        address mocked = address(0xFACE);

        vm.mockCall(mocked, selector, abi.encode(uint256(99)));

        assertEq(ISymbolicDataTarget(mocked).value(1), 99);
    }

    function checkMockFunctionAcceptsSymbolicData(uint256 input) public {
        address mocked = address(0xF00D);

        vm.mockFunction(
            mocked,
            address(functionTarget),
            abi.encodeWithSelector(ISymbolicDataTarget.value.selector, input)
        );

        assertEq(ISymbolicDataTarget(mocked).value(input), input + 10);
    }

    function checkMockFunctionAcceptsSymbolicCallee(address mocked, uint256 input) public {
        address actual = address(0xF00D);
        vm.assume(mocked == actual);

        vm.mockFunction(
            mocked,
            address(functionTarget),
            abi.encodeWithSelector(ISymbolicDataTarget.value.selector, input)
        );

        assertEq(ISymbolicDataTarget(actual).value(input), input + 10);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicCallDataCheatcodes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkExpectCallAcceptsSymbolicData(uint256)"), "{stdout}");
    assert!(
        stdout.contains("[PASS] checkMockCallAcceptsSymbolicDataAndReturn(uint256)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("[PASS] checkMockCallAcceptsSymbolicBytes4Selector(bytes4)"),
        "{stdout}"
    );
    assert!(stdout.contains("[PASS] checkMockFunctionAcceptsSymbolicData(uint256)"), "{stdout}");
    assert!(
        stdout.contains("[PASS] checkMockFunctionAcceptsSymbolicCallee(address,uint256)"),
        "{stdout}"
    );
    assert!(!stdout.contains("symbolic vm.expectCall"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.mockCall"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.mockFunction"), "{stdout}");
});

forgetest_init!(symbolic_vm_call_data_match_branches_find_mismatch, |prj, _cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_call_data_match_branches_find_mismatch because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCallDataMismatch.t.sol",
        r#"
import "forge-std/Test.sol";

interface ISymbolicCallDataMismatchTarget {
    function value(uint256 input) external returns (uint256);
}

contract SymbolicCallDataMismatchTarget {
    function value(uint256 input) external pure returns (uint256) {
        return input;
    }
}

contract SymbolicFunctionMockMismatchTarget {
    function value(uint256 input) external pure returns (uint256) {
        return input + 10;
    }
}

contract SymbolicCallDataMismatch is Test {
    SymbolicCallDataMismatchTarget target;
    SymbolicFunctionMockMismatchTarget functionTarget;

    function setUp() public {
        target = new SymbolicCallDataMismatchTarget();
        functionTarget = new SymbolicFunctionMockMismatchTarget();
    }

    function checkExpectCallSymbolicDataFindsMismatch(uint256 expected, uint256 actual) public {
        vm.expectCall(
            address(target),
            abi.encodeWithSelector(SymbolicCallDataMismatchTarget.value.selector, expected)
        );

        target.value(actual);
    }

    function checkMockCallSymbolicDataFindsMismatch(uint256 expected, uint256 actual) public {
        address mocked = address(0xDADA);

        vm.mockCall(
            mocked,
            abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, expected),
            abi.encode(uint256(99))
        );

        (bool ok, bytes memory data) =
            mocked.call(abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, actual));
        assertTrue(ok);
        assertEq(data.length, 32);
        assertEq(abi.decode(data, (uint256)), 99);
    }

    function checkMockFunctionSymbolicDataFindsMismatch(uint256 expected, uint256 actual) public {
        address mocked = address(0xF00D);

        vm.mockFunction(
            mocked,
            address(functionTarget),
            abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, expected)
        );

        (bool ok, bytes memory data) =
            mocked.call(abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, actual));
        assertTrue(ok);
        assertEq(data.length, 32);
        assertEq(abi.decode(data, (uint256)), actual + 10);
    }

    function checkMockFunctionSymbolicCalleeFindsMismatch(address mocked) public {
        address actual = address(0xF00D);

        vm.mockFunction(
            mocked,
            address(functionTarget),
            abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, uint256(1))
        );

        (bool ok, bytes memory data) =
            actual.call(abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, uint256(1)));
        assertTrue(ok);
        assertEq(data.length, 32);
        assertEq(abi.decode(data, (uint256)), uint256(11));
    }
}
"#,
    );

    for test in [
        "checkExpectCallSymbolicDataFindsMismatch",
        "checkMockCallSymbolicDataFindsMismatch",
        "checkMockFunctionSymbolicDataFindsMismatch",
        "checkMockFunctionSymbolicCalleeFindsMismatch",
    ] {
        let stdout = prj
            .forge_command()
            .args(["test", "--symbolic", "--match-test", test])
            .assert_failure()
            .get_output()
            .stdout_lossy();

        assert!(stdout.contains("[FAIL:"), "{stdout}");
        assert!(stdout.contains(test), "{stdout}");
        assert!(!stdout.contains("symbolic vm.expectCall"), "{stdout}");
        assert!(!stdout.contains("symbolic vm.mockCall"), "{stdout}");
        assert!(!stdout.contains("symbolic vm.mockFunction"), "{stdout}");
    }
});

forgetest_init!(symbolic_vm_mock_function_routes_to_target, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_mock_function_routes_to_target because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMockFunction.t.sol",
        r#"
import "forge-std/Test.sol";

interface IFunctionMock {
    function value(uint256 input) external returns (uint256);
    function who(uint256 input) external returns (address);
}

contract FunctionCallee {
    function value(uint256 input) external pure returns (uint256) {
        return input + 1;
    }

    function who(uint256) external view returns (address) {
        return address(this);
    }
}

contract FunctionTarget {
    function value(uint256 input) external pure returns (uint256) {
        return input ^ 0x55;
    }

    function who(uint256) external view returns (address) {
        return address(this);
    }
}

contract SymbolicMockFunction is Test {
    FunctionCallee callee;
    FunctionTarget target;

    function setUp() public {
        callee = new FunctionCallee();
        target = new FunctionTarget();
    }

    function checkMockFunction(uint256 input) public {
        vm.mockFunction(
            address(callee),
            address(target),
            abi.encodePacked(IFunctionMock.value.selector)
        );
        assertEq(IFunctionMock(address(callee)).value(input), input ^ 0x55);

        vm.mockFunction(
            address(callee),
            address(target),
            abi.encodePacked(IFunctionMock.who.selector)
        );
        assertEq(IFunctionMock(address(callee)).who(input), address(callee));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMockFunction"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkMockFunction(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_record_accesses_tracks_symbolic_slots, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_record_accesses_tracks_symbolic_slots because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRecordAccesses.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicRecordAccesses is Test {
    function checkRecordAccesses(bytes32 slot, bytes32 stored) public {
        vm.record();

        bytes32 loadedSlot;
        assembly {
            sstore(slot, stored)
            loadedSlot := sload(slot)
        }

        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(address(this));
        assertEq(loadedSlot, stored);
        assertEq(reads.length, 1);
        assertEq(writes.length, 1);
        assertEq(reads[0], slot);
        assertEq(writes[0], slot);

        vm.stopRecord();
    }

    function checkRecordAccessesSymbolicTarget(address target, bytes32 slot, bytes32 stored) public {
        vm.assume(target == address(this));
        vm.record();

        bytes32 loadedSlot;
        assembly {
            sstore(slot, stored)
            loadedSlot := sload(slot)
        }

        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(target);
        assertEq(loadedSlot, stored);
        assertEq(reads.length, 1);
        assertEq(writes.length, 1);
        assertEq(reads[0], slot);
        assertEq(writes[0], slot);

        vm.stopRecord();
    }

    function checkRecordAccessesSymbolicTargetBranches(address target, bytes32 slot, bytes32 stored) public {
        address other = address(0xBEEF);
        vm.assume(target == address(this) || target == other);
        vm.record();

        bytes32 loadedSlot;
        assembly {
            sstore(slot, stored)
            loadedSlot := sload(slot)
        }

        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(target);
        assertEq(loadedSlot, stored);
        if (target == address(this)) {
            assertEq(reads.length, 1);
            assertEq(writes.length, 1);
            assertEq(reads[0], slot);
            assertEq(writes[0], slot);
        } else {
            assertEq(reads.length, 0);
            assertEq(writes.length, 0);
        }

        vm.stopRecord();
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkRecordAccesses"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkRecordAccesses(bytes32,bytes32)"), "{stdout}");
    assert!(
        stdout.contains("[PASS] checkRecordAccessesSymbolicTarget(address,bytes32,bytes32)"),
        "{stdout}"
    );
    assert!(
        stdout
            .contains("[PASS] checkRecordAccessesSymbolicTargetBranches(address,bytes32,bytes32)"),
        "{stdout}"
    );
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.accesses address"), "{stdout}");
});

forgetest_init!(symbolic_vm_bound_skip_and_gas_noops, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_bound_skip_and_gas_noops because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBoundSkip.t.sol",
        r#"
import "forge-std/Test.sol";

interface SymbolicVmCompat {
    enum CallerMode {
        None,
        Broadcast,
        RecurrentBroadcast,
        Prank,
        RecurrentPrank
    }

    enum ForgeContext {
        TestGroup,
        Test,
        Coverage,
        Snapshot,
        ScriptGroup,
        ScriptDryRun,
        ScriptBroadcast,
        ScriptResume,
        Unknown
    }

    function readCallers() external view returns (CallerMode callerMode, address msgSender, address txOrigin);
    function isContext(ForgeContext context) external view returns (bool result);
}

contract SymbolicBoundSkip is Test {
    function externalNoop() external {}

    function checkBoundSkipAndGasNoops(uint256 x, int256 y) public {
        vm.pauseGasMetering();
        vm.resumeGasMetering();
        vm.resetGasMetering();
        vm.expectSafeMemory(0, 0x80);
        vm.expectSafeMemoryCall(0, 0x80);
        vm.startSnapshotGas("scope");
        this.externalNoop();
        vm.snapshotGasLastCall("last");
        vm.snapshotGasLastCall("group", "last");
        vm.stopSnapshotGas();
        vm.stopSnapshotGas("scope");
        vm.stopSnapshotGas("group", "scope");
        Vm.Gas memory gas = vm.lastCallGas();
        gas.gasTotalUsed;

        uint256 bounded = vm.bound(x, 10, 12);
        assertGe(bounded, 10);
        assertLe(bounded, 12);

        int256 signedBounded = vm.bound(y, -3, 3);
        assertGe(signedBounded, -3);
        assertLe(signedBounded, 3);

        vm.skip(x == 42);
        assertTrue(x != 42);
    }

    function checkVmCompatibilityTail() public {
        SymbolicVmCompat compat = SymbolicVmCompat(address(vm));
        SymbolicVmCompat.CallerMode mode;
        address sender;
        (mode,,) = compat.readCallers();
        assertEq(uint256(mode), uint256(SymbolicVmCompat.CallerMode.None));

        vm.prank(address(0xB0B));
        (mode, sender,) = compat.readCallers();
        assertEq(uint256(mode), uint256(SymbolicVmCompat.CallerMode.Prank));
        assertEq(sender, address(0xB0B));
        vm.stopPrank();

        vm.startPrank(address(0xCAFE));
        (mode, sender,) = compat.readCallers();
        assertEq(uint256(mode), uint256(SymbolicVmCompat.CallerMode.RecurrentPrank));
        assertEq(sender, address(0xCAFE));
        vm.stopPrank();

        vm.allowCheatcodes(address(this));
        vm.makePersistent(address(this));
        assertTrue(vm.isPersistent(address(this)));
        address[] memory accounts = new address[](1);
        accounts[0] = address(0xBEEF);
        vm.makePersistent(accounts);
        assertTrue(vm.isPersistent(address(0xBEEF)));
        vm.revokePersistent(address(this));
        vm.revokePersistent(accounts);
        assertFalse(vm.isPersistent(address(this)));
        assertFalse(vm.isPersistent(address(0xBEEF)));

        vm.label(address(this), "self");
        assertEq(vm.getLabel(address(this)), "self");
        vm.snapshotValue("value", 1);
        vm.snapshotValue("group", "value", 1);
        vm.cool(address(this));
        vm.warmSlot(address(this), bytes32(uint256(1)));
        vm.coolSlot(address(this), bytes32(uint256(1)));
        vm.noAccessList();
        assertEq(vm.getChainId(), block.chainid);
        assertTrue(bytes(vm.projectRoot()).length != 0);
        assertTrue(vm.unixTime() != 0);
        assertTrue(compat.isContext(SymbolicVmCompat.ForgeContext.TestGroup));
        assertTrue(compat.isContext(SymbolicVmCompat.ForgeContext.Test));
        assertFalse(compat.isContext(SymbolicVmCompat.ForgeContext.ScriptGroup));
    }

    function checkRuntimeNoopsAndArrayAssertions() public {
        vm.breakpoint("symbolic");
        vm.breakpoint("symbolic", true);
        vm.stopExpectSafeMemory();
        vm.setEvmVersion("cancun");
        assertEq(vm.getEvmVersion(), "cancun");
        assertTrue(bytes(vm.getFoundryVersion()).length != 0);
        vm.sleep(0);
        Vm.AccessListItem[] memory access = new Vm.AccessListItem[](0);
        vm.accessList(access);

        uint256[] memory left = new uint256[](1);
        uint256[] memory right = new uint256[](1);
        left[0] = 1;
        right[0] = 1;
        assertEq(left, right);
        right[0] = 2;
        assertNotEq(left, right);

        string[] memory words = new string[](1);
        string[] memory sameWords = new string[](1);
        words[0] = "foundry";
        sameWords[0] = "foundry";
        assertEq(words, sameWords);

        assertEqDecimal(uint256(1e18), uint256(1e18), 18);
        assertEqDecimal(int256(-1e18), int256(-1e18), 18);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicBoundSkip"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkBoundSkipAndGasNoops(uint256,int256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkVmCompatibilityTail()"), "{stdout}");
    assert!(stdout.contains("[PASS] checkRuntimeNoopsAndArrayAssertions()"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_bound_invalid_range_fails_without_stuck, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_bound_invalid_range_fails_without_stuck because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBoundInvalid.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBoundInvalid is Test {
    function checkInvalidUnsignedBound(uint256 x) public {
        vm.bound(x, 12, 10);
    }

    function checkInvalidSignedBound(int256 x) public {
        vm.bound(x, 3, -3);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicBoundInvalid"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(stdout.contains("checkInvalidUnsignedBound(uint256)"), "{stdout}");
    assert!(stdout.contains("checkInvalidSignedBound(int256)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.bound range"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_assume_no_revert_prunes_reverting_call, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_assume_no_revert_prunes_reverting_call because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicAssumeNoRevert.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicAssumeNoRevertTarget {
    function maybeRevert(uint256 x) external pure {
        require(x != 7, "seven");
    }
}

contract SymbolicAssumeNoRevert is Test {
    SymbolicAssumeNoRevertTarget target;

    function setUp() public {
        target = new SymbolicAssumeNoRevertTarget();
    }

    function checkAssumeNoRevertPrunes(uint256 x) public {
        vm.assumeNoRevert();
        (bool ok,) = address(target).call(abi.encodeWithSelector(target.maybeRevert.selector, x));
        assertTrue(ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkAssumeNoRevertPrunes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkAssumeNoRevertPrunes(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.assumeNoRevert"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_assume_no_revert_filters_revert_matches, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_assume_no_revert_filters_revert_matches because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicAssumeNoRevertFilters.t.sol",
        r#"
import "forge-std/Test.sol";

interface SymbolicVm {
    struct PotentialRevert {
        address reverter;
        bool partialMatch;
        bytes revertData;
    }

    function assume(bool condition) external pure;
    function assumeNoRevert(PotentialRevert calldata potentialRevert) external pure;
    function assumeNoRevert(PotentialRevert[] calldata potentialReverts) external pure;
}

error Expected(uint256 value);
error Other(uint256 value);

contract SymbolicAssumeNoRevertFilterTarget {
    function onlyExpected(uint256 x) external pure {
        if (x == 7) revert Expected(7);
    }

    function twoReverts(uint256 x) external pure {
        if (x == 7) revert Expected(x);
        if (x == 9) revert Other(x);
    }
}

contract SymbolicAssumeNoRevertOtherTarget {
    function onlyExpected(uint256 x) external pure {
        if (x == 7) revert Expected(7);
    }
}

contract SymbolicAssumeNoRevertFilters is Test {
    SymbolicAssumeNoRevertFilterTarget target;
    SymbolicAssumeNoRevertOtherTarget other;
    SymbolicVm symbolicVm = SymbolicVm(VM_ADDRESS);

    function setUp() public {
        target = new SymbolicAssumeNoRevertFilterTarget();
        other = new SymbolicAssumeNoRevertOtherTarget();
    }

    function checkAssumeNoRevertExactFilterPrunes(uint256 x) public {
        symbolicVm.assumeNoRevert(SymbolicVm.PotentialRevert({
            reverter: address(target),
            partialMatch: false,
            revertData: abi.encodeWithSelector(Expected.selector, uint256(7))
        }));

        (bool ok,) = address(target).call(abi.encodeWithSelector(target.onlyExpected.selector, x));
        assertTrue(ok);
    }

    function checkAssumeNoRevertArrayFilterPrunes(uint256 x) public {
        SymbolicVm.PotentialRevert[] memory filters = new SymbolicVm.PotentialRevert[](2);
        filters[0] = SymbolicVm.PotentialRevert({
            reverter: address(target),
            partialMatch: true,
            revertData: abi.encodeWithSelector(Expected.selector)
        });
        filters[1] = SymbolicVm.PotentialRevert({
            reverter: address(target),
            partialMatch: false,
            revertData: abi.encodeWithSelector(Other.selector, uint256(9))
        });
        symbolicVm.assumeNoRevert(filters);

        (bool ok,) = address(target).call(abi.encodeWithSelector(target.twoReverts.selector, x));
        assertTrue(ok);
    }

    function checkAssumeNoRevertWrongDataFails(uint256 x) public {
        symbolicVm.assume(x == 9);
        symbolicVm.assumeNoRevert(SymbolicVm.PotentialRevert({
            reverter: address(target),
            partialMatch: false,
            revertData: abi.encodeWithSelector(Other.selector, uint256(8))
        }));

        (bool ok,) = address(target).call(abi.encodeWithSelector(target.twoReverts.selector, x));
        assertTrue(ok);
    }

    function checkAssumeNoRevertWrongReverterFails(uint256 x) public {
        symbolicVm.assume(x == 7);
        symbolicVm.assumeNoRevert(SymbolicVm.PotentialRevert({
            reverter: address(other),
            partialMatch: true,
            revertData: abi.encodeWithSelector(Expected.selector)
        }));

        (bool ok,) = address(target).call(abi.encodeWithSelector(target.onlyExpected.selector, x));
        assertTrue(ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkAssumeNoRevert.*Prunes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] checkAssumeNoRevertExactFilterPrunes(uint256)"), "{stdout}");
    assert!(stdout.contains("[PASS] checkAssumeNoRevertArrayFilterPrunes(uint256)"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.assumeNoRevert"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");

    for test in ["checkAssumeNoRevertWrongDataFails", "checkAssumeNoRevertWrongReverterFails"] {
        let stdout = prj
            .forge_command()
            .args(["test", "--symbolic", "--match-test", test])
            .assert_failure()
            .get_output()
            .stdout_lossy();

        assert!(stdout.contains("[FAIL:"), "{stdout}");
        assert!(stdout.contains(test), "{stdout}");
        assert!(!stdout.contains("symbolic vm.assumeNoRevert"), "{stdout}");
        assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
    }
});
