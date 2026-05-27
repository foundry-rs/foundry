use super::symbolic_helpers::assert_relevant_lines;
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, str, util::OutputExt};

use super::symbolic_helpers::{assert_symbolic, z3_available};
use crate::skip_unless_z3;

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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicMload(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkConstrainedMstore(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL: panic: assertion failed
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL: panic: assertion failed
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicMsize(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicSha3(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkConstrainedSha3Size(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkBoundedSha3Size(uint8,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicLogOffset(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicLogSize(uint8)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkConstrainedReturndataCopy(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicReturndataCopyOffset(uint8,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicReturndataCopyDest(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicReturndataCopySize(uint8,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicReturnOffset(uint16,uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicRevertOffset(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicMcopy(uint16,uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic MCOPY src"), "{stdout}");
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicReturnSize(uint8,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicRevertSize(uint8,uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic REVERT size"), "{stdout}");
});

// Dynamic-offset memory read must respect write-epoch ordering: if a later
// concrete MSTORE has written to an offset, a subsequent symbolic-offset MLOAD
// that aliases that offset must see the later value, not the stale earlier
// symbolic write. If epoch ordering regressed, Z3 could pick `symKey == 0x80`
// and the symbolic MLOAD would surface `0xdeadbeef` instead of `0x1234`,
// flipping the assertion below into a counterexample.
forgetest_init!(symbolic_dynamic_mload_respects_later_concrete_overwrite, |prj, cmd| {
    skip_unless_z3!("symbolic_dynamic_mload_respects_later_concrete_overwrite");

    prj.add_test(
        "SymbolicMemoryEpochOrdering.t.sol",
        r#"
contract SymbolicMemoryEpochOrdering {
    function checkLaterConcreteWriteWins(uint256 symKey, uint256 readKey) public pure {
        uint256 v;
        assembly {
            // Earlier symbolic-offset write.
            mstore(symKey, 0xdeadbeef)
            // Later concrete-offset write — must be visible at slot 0x80
            // regardless of what `symKey` was.
            mstore(0x80, 0x1234)
            // Dynamic-offset read.
            v := mload(readKey)
        }
        if (readKey == 0x80) {
            assert(v == 0x1234);
        }
    }
}
"#,
    );

    assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkLaterConcreteWriteWins",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicMemoryEpochOrdering.t.sol:SymbolicMemoryEpochOrdering
[PASS] checkLaterConcreteWriteWins(uint256,uint256) ([METRICS])
...
"#]]);
});
