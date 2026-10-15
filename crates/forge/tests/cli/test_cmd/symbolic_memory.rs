use super::symbolic_helpers::{assert_symbolic, assert_symbolic_witness, z3_available};
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, snapbox::IntoData, str, util::OutputExt};

use crate::skip_unless_z3;

// MLOAD, MSTORE and MSIZE with symbolic offsets are modeled instead of reported as Stuck.
#[forgetest_init]
fn symbolic_memory_word_ops_accept_symbolic_offsets(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_memory_word_ops_accept_symbolic_offsets");

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

    // Dynamic-offset memory read must respect write-epoch ordering: if a later
    // concrete MSTORE has written to an offset, a subsequent symbolic-offset MLOAD
    // that aliases that offset must see the later value, not the stale earlier
    // symbolic write. If epoch ordering regressed, Z3 could pick `symKey == 0x80`
    // and the symbolic MLOAD would surface `0xdeadbeef` instead of `0x1234`,
    // flipping the assertion below into a counterexample.
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

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkSymbolicMload|checkConstrainedMstore|checkSymbolicMsize|checkLaterConcreteWriteWins)\\(",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicMsizeAfterWrite.t.sol:SymbolicMsizeAfterWrite
[PASS] checkSymbolicMsize(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicMemoryEpochOrdering.t.sol:SymbolicMemoryEpochOrdering
[PASS] checkLaterConcreteWriteWins(uint256,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicMstoreConstrained.t.sol:SymbolicMstoreConstrained
[PASS] checkConstrainedMstore(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicMload.t.sol:SymbolicMload
[PASS] checkSymbolicMload(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]].unordered())
    .get_output()
    .stdout_lossy();
    for reason in [
        "symbolic MLOAD offset",
        "symbolic MSIZE after symbolic memory write",
        "symbolic MSTORE offset",
    ] {
        assert!(!stdout.contains(reason), "{stdout}");
    }
}

#[forgetest_init]
fn symbolic_fixed_memory_access_rejects_oversized_offset(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_fixed_memory_access_rejects_oversized_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicOversizedMemoryOffset.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicOversizedMemoryOffset is Test {
    function load(uint256 offset) external pure {
        assembly {
            pop(mload(offset))
        }
    }

    function store(uint256 offset) external pure {
        assembly {
            mstore(offset, 1)
        }
    }

    function store8(uint256 offset) external pure {
        assembly {
            mstore8(offset, 1)
        }
    }

    function checkOversizedFixedMemoryAccesses() public {
        uint256 offset = type(uint256).max;
        (bool loadOk,) = address(this).call(abi.encodeCall(this.load, (offset)));
        (bool storeOk,) = address(this).call(abi.encodeCall(this.store, (offset)));
        (bool store8Ok,) = address(this).call(abi.encodeCall(this.store8, (offset)));
        assertFalse(loadOk);
        assertFalse(storeOk);
        assertFalse(store8Ok);
    }

    function checkConstrainedOversizedMemoryAccess(uint256 offset) public {
        vm.assume(offset == type(uint256).max);
        (bool ok,) = address(this).call(abi.encodeCall(this.store, (offset)));
        assertFalse(ok);
    }

    function checkMixedMemoryOffsetExploresValidSibling(uint256 offset) public {
        bool endpoint;
        assembly {
            endpoint := or(iszero(offset), eq(offset, not(0)))
        }
        vm.assume(endpoint);
        (bool ok,) = address(this).call(abi.encodeCall(this.store, (offset)));
        assertFalse(ok);
    }

    function createWithOversizedSize() external {
        assembly {
            pop(create(0, 0, not(0)))
        }
    }

    function create2WithOversizedSize() external {
        assembly {
            pop(create2(0, 0, not(0), 0))
        }
    }

    function checkOversizedCreateRanges() public {
        (bool createOk,) = address(this).call(abi.encodeCall(this.createWithOversizedSize, ()));
        (bool create2Ok,) = address(this).call(abi.encodeCall(this.create2WithOversizedSize, ()));
        assertFalse(createOk);
        assertFalse(create2Ok);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "check.*Oversized"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 3 tests for test/SymbolicOversizedMemoryOffset.t.sol:SymbolicOversizedMemoryOffset
[PASS] checkConstrainedOversizedMemoryAccess(uint256) ([METRICS])
[PASS] checkOversizedCreateRanges() ([METRICS])
[PASS] checkOversizedFixedMemoryAccesses() ([METRICS])
Suite result: ok. 3 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    cmd.forge_fuse();
    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkMixedMemoryOffsetExploresValidSibling",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicOversizedMemoryOffset.t.sol:SymbolicOversizedMemoryOffset
[FAIL: assertion failed; counterexample: 		[SENDER] [SENDER] calldata=0x8ba9bb5b0000000000000000000000000000000000000000000000000000000000000000 args=[0]] checkMixedMemoryOffsetExploresValidSibling(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("counterexample did not replay"), "{stdout}");
}

#[forgetest_init]
fn symbolic_variable_memory_access_rejects_oversized_ranges(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_variable_memory_access_rejects_oversized_ranges because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicOversizedMemoryRange.t.sol",
        r#"
contract SymbolicOversizedMemoryRange {
    function calldataCopy() external pure {
        assembly {
            calldatacopy(not(0), 0, 1)
        }
    }

    function codeCopy() external pure {
        assembly {
            codecopy(not(0), 0, 1)
        }
    }

    function extcodeCopy() external view {
        assembly {
            extcodecopy(address(), not(0), 0, 1)
        }
    }

    function returndataCopy() external view {
        assembly {
            pop(staticcall(gas(), 4, 0, 1, 0, 1))
            returndatacopy(not(0), 0, 1)
        }
    }

    function memoryCopyDest() external pure {
        assembly {
            mcopy(not(0), 0, 1)
        }
    }

    function memoryCopySource() external pure {
        assembly {
            mcopy(0, not(0), 1)
        }
    }

    function hash() external pure {
        assembly {
            pop(keccak256(not(0), 1))
        }
    }

    function log() external {
        assembly {
            log0(not(0), 1)
        }
    }

    function ret() external pure {
        assembly {
            return(not(0), 1)
        }
    }

    function rev() external pure {
        assembly {
            revert(not(0), 1)
        }
    }

    function testOversizedVariableMemoryRanges() public {
        verifyOversizedVariableMemoryRanges(true);
    }

    function checkOversizedVariableMemoryRanges() public {
        verifyOversizedVariableMemoryRanges(false);
    }

    function verifyOversizedVariableMemoryRanges(bool capGas) internal {
        assertFails(this.calldataCopy.selector, capGas);
        assertFails(this.codeCopy.selector, capGas);
        assertFails(this.extcodeCopy.selector, capGas);
        assertFails(this.returndataCopy.selector, capGas);
        assertFails(this.memoryCopyDest.selector, capGas);
        assertFails(this.memoryCopySource.selector, capGas);
        assertFails(this.hash.selector, capGas);
        assertFails(this.log.selector, capGas);
        assertFails(this.ret.selector, capGas);
        assertFails(this.rev.selector, capGas);
    }

    function assertFails(bytes4 selector, bool capGas) internal {
        bytes memory input = abi.encodeWithSelector(selector);
        (bool ok, bytes memory data) = capGas
            ? address(this).call{gas: 100_000}(input)
            : address(this).call(input);
        assert(!ok);
        assert(data.length == 0);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--match-test",
        "testOversizedVariableMemoryRanges",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicOversizedMemoryRange.t.sol:SymbolicOversizedMemoryRange
[PASS] testOversizedVariableMemoryRanges() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    cmd.forge_fuse();
    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkOversizedVariableMemoryRanges",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicOversizedMemoryRange.t.sol:SymbolicOversizedMemoryRange
[PASS] checkOversizedVariableMemoryRanges() ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_fixed_memory_access_respects_memory_limit(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_fixed_memory_access_respects_memory_limit because z3 is not available"
        );
        return;
    }

    prj.wipe_contracts();
    prj.update_config(|config| config.memory_limit = 4096);
    prj.add_test(
        "SymbolicMemoryLimit.t.sol",
        r#"
contract SymbolicMemoryLimit {
    fallback() external payable {
        assembly {
            switch callvalue()
            case 0 { mstore(4065, 1) }
            case 1 { mstore8(4096, 1) }
            case 2 { mstore(2048, 1) }
            default { mstore8(0, 1) }
        }
    }

    function checkMemoryLimitExactBoundaries() public pure {
        assembly {
            mstore(4064, 1)
            mstore8(4095, 1)
        }
    }

    function checkMemoryLimitFirstInvalidBoundaries() public {
        bool wordOk;
        bool byteOk;
        assembly {
            wordOk := call(gas(), address(), 0, 0, 0, 0, 0)
            byteOk := call(gas(), address(), 1, 0, 0, 0, 0)
        }
        assert(!wordOk && !byteOk);
    }

    function checkMemoryLimitNestedCall() public {
        bool ok;
        assembly {
            mstore(2048, 1)
            ok := call(gas(), address(), 2, 0, 0, 0, 0)
        }
        assert(ok);
    }

    function checkMemoryLimitCallInputExpansion() public {
        bool ok;
        assembly {
            ok := call(gas(), address(), 3, 2048, 2048, 0, 0)
        }
        assert(ok);
    }

    function checkMemoryLimitSymbolicCallSize(bool expand) public {
        bool ok;
        assembly {
            let size := mul(expand, 2048)
            ok := call(gas(), address(), 3, 2048, size, 0, 0)
        }
        assert(ok);
    }

    function wrappingCallRange() external {
        assembly {
            pop(call(gas(), address(), 0, not(0), 1, 0, 0))
        }
    }

    function nestedWrappingCallRange() external {
        this.wrappingCallRange();
    }

    function checkMemoryLimitRejectsWrappingCallRanges() public {
        (bool directOk,) = address(this).call(abi.encodeCall(this.wrappingCallRange, ()));
        (bool nestedOk,) = address(this).call(abi.encodeCall(this.nestedWrappingCallRange, ()));
        assert(!directOk && !nestedOk);
    }
}
"#,
    );

    cmd.args(["test", "--match-test", "checkMemoryLimitNestedCall"]).assert_success();

    cmd.forge_fuse();
    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkMemoryLimit"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 6 tests for test/SymbolicMemoryLimit.t.sol:SymbolicMemoryLimit
[PASS] checkMemoryLimitCallInputExpansion() ([METRICS])
[PASS] checkMemoryLimitExactBoundaries() ([METRICS])
[PASS] checkMemoryLimitFirstInvalidBoundaries() ([METRICS])
[PASS] checkMemoryLimitNestedCall() ([METRICS])
[PASS] checkMemoryLimitRejectsWrappingCallRanges() ([METRICS])
[PASS] checkMemoryLimitSymbolicCallSize(bool) ([METRICS])
Suite result: ok. 6 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_mstore_accepts_unconstrained_symbolic_offset(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkSymbolicMstore",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicMstoreUnconstrained.t.sol:SymbolicMstoreUnconstrained
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkSymbolicMstore(uint16,uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic MSTORE offset"), "{stdout}");
}

#[forgetest_init]
fn symbolic_mstore8_accepts_unconstrained_symbolic_offset(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkSymbolicMstore8",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicMstore8Unconstrained.t.sol:SymbolicMstore8Unconstrained
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkSymbolicMstore8(uint16,uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic MSTORE8 offset"), "{stdout}");
}

#[forgetest_init]
fn symbolic_msize_tracks_read_only_memory_expansion(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_msize_tracks_read_only_memory_expansion");

    prj.add_test(
        "SymbolicMsizeAfterRead.t.sol",
        r#"
contract SymbolicMsizeAfterRead {
    function checkReadOnlyExpansion() public {
        uint256 afterHash;
        uint256 afterLog;
        uint256 afterCopy;
        assembly {
            pop(keccak256(0x200, 1))
            afterHash := msize()
            log0(0x400, 1)
            afterLog := msize()
            mcopy(0, 0x600, 1)
            afterCopy := msize()
        }

        assert(afterHash == 0x220);
        assert(afterLog == 0x420);
        assert(afterCopy == 0x620);
    }

    function checkSymbolicReadOnlyExpansion(uint16 offset) public pure {
        uint256 afterHash;
        assembly {
            pop(keccak256(offset, 1))
            afterHash := msize()
        }

        assert(afterHash > offset);
    }
}
"#,
    );

    cmd.args(["test", "--match-test", "check.*ReadOnlyExpansion"]).assert_success();

    cmd.forge_fuse();
    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "check.*ReadOnlyExpansion",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 2 tests for test/SymbolicMsizeAfterRead.t.sol:SymbolicMsizeAfterRead
[PASS] checkReadOnlyExpansion() ([METRICS])
[PASS] checkSymbolicReadOnlyExpansion(uint16) ([METRICS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_msize_respects_zero_symbolic_copy_size(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_msize_respects_zero_symbolic_copy_size");

    prj.add_test(
        "SymbolicMsizeAfterCopy.t.sol",
        r#"
contract SymbolicMsizeAfterCopy {
    function checkZeroLengthCopy(uint8 n) public pure {
        uint256 size = uint256(n & 3);
        uint256 beforeSize;
        uint256 afterSize;
        assembly {
            beforeSize := msize()
            calldatacopy(0x100, 0, size)
            afterSize := msize()
        }

        assert(size != 0 || afterSize != beforeSize);
    }
}
"#,
    );

    let stdout =
        assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkZeroLengthCopy"]))
            .failure()
            .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicMsizeAfterCopy.t.sol:SymbolicMsizeAfterCopy
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] calldata=0xac831bba0000000000000000000000000000000000000000000000000000000000000000 args=[0]] checkZeroLengthCopy(uint8) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
            .get_output()
            .stdout_lossy();

    assert!(!stdout.contains("symbolic counterexample did not replay"), "{stdout}");
}

// SHA3 over a symbolic offset or a constrained or bounded symbolic size is modeled.
#[forgetest_init]
fn symbolic_sha3_accepts_symbolic_operands(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_sha3_accepts_symbolic_operands");

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

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkSymbolicSha3|checkConstrainedSha3Size|checkBoundedSha3Size)\\(",
    ]))
    .success()
    .stdout_eq(
        str![[r#"
...
Ran 1 test for test/SymbolicSha3BoundedSize.t.sol:SymbolicSha3BoundedSize
[PASS] checkBoundedSha3Size(uint8,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicSha3ConstrainedSize.t.sol:SymbolicSha3ConstrainedSize
[PASS] checkConstrainedSha3Size(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicSha3.t.sol:SymbolicSha3
[PASS] checkSymbolicSha3(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]
        .unordered(),
    )
    .get_output()
    .stdout_lossy();
    for reason in ["symbolic SHA3 offset", "symbolic SHA3 size"] {
        assert!(!stdout.contains(reason), "{stdout}");
    }
}

// LOG with a symbolic offset or a bounded symbolic size is modeled.
#[forgetest_init]
fn symbolic_log_accepts_symbolic_operands(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_log_accepts_symbolic_operands");

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

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkSymbolicLogOffset|checkSymbolicLogSize)\\(",
    ]))
    .success()
    .stdout_eq(
        str![[r#"
...
Ran 1 test for test/SymbolicLogSize.t.sol:SymbolicLogSize
[PASS] checkSymbolicLogSize(uint8) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicLogOffset.t.sol:SymbolicLogOffset
[PASS] checkSymbolicLogOffset(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]
        .unordered(),
    )
    .get_output()
    .stdout_lossy();
    for reason in ["symbolic LOG offset", "symbolic LOG size"] {
        assert!(!stdout.contains(reason), "{stdout}");
    }
}

// RETURNDATACOPY with symbolic offsets, destination or bounded size is modeled.
#[forgetest_init]
fn symbolic_returndatacopy_accepts_symbolic_operands(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_returndatacopy_accepts_symbolic_operands");

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

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkConstrainedReturndataCopy|checkSymbolicReturndataCopyOffset|checkSymbolicReturndataCopyDest|checkSymbolicReturndataCopySize)\\(",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicReturndataCopySize.t.sol:SymbolicReturndataCopySize
[PASS] checkSymbolicReturndataCopySize(uint8,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicReturndataCopyOffset.t.sol:SymbolicReturndataCopyOffset
[PASS] checkSymbolicReturndataCopyOffset(uint8,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicReturndataCopyDest.t.sol:SymbolicReturndataCopyDest
[PASS] checkSymbolicReturndataCopyDest(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicReturndataCopyConstrained.t.sol:SymbolicReturndataCopyConstrained
[PASS] checkConstrainedReturndataCopy(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]].unordered())
    .get_output()
    .stdout_lossy();
    for reason in [
        "symbolic RETURNDATACOPY dest",
        "symbolic RETURNDATACOPY offset",
        "symbolic RETURNDATACOPY size",
    ] {
        assert!(!stdout.contains(reason), "{stdout}");
    }
}

#[forgetest_init]
fn symbolic_returndatacopy_reverts_on_out_of_bounds_offset_with_symbolic_size(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_returndatacopy_reverts_on_out_of_bounds_offset_with_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicReturndataCopyOobOffset.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicReturndataCopyOobOffsetHelper {
    function pair(uint256 marker) external pure returns (uint256, uint256) {
        return (11, marker);
    }
}

contract SymbolicReturndataCopyOobOffsetTrigger {
    SymbolicReturndataCopyOobOffsetHelper public helper;

    constructor(SymbolicReturndataCopyOobOffsetHelper _helper) {
        helper = _helper;
    }

    function copy(uint256 offset, uint256 size) external {
        bytes4 selector = SymbolicReturndataCopyOobOffsetHelper.pair.selector;
        address target = address(helper);
        assembly {
            mstore(0x80, selector)
            mstore(0x84, 0)
            pop(staticcall(gas(), target, 0x80, 36, 0, 0))
            returndatacopy(0, offset, size)
        }
    }
}

contract SymbolicReturndataCopyOobOffset is Test {
    SymbolicReturndataCopyOobOffsetHelper helper;
    SymbolicReturndataCopyOobOffsetTrigger trigger;

    function setUp() public {
        helper = new SymbolicReturndataCopyOobOffsetHelper();
        trigger = new SymbolicReturndataCopyOobOffsetTrigger(helper);
    }

    function checkOutOfBoundsOffsetForcedZeroSizeReverts(uint256 size) public {
        vm.assume(size <= 0);
        vm.expectRevert();
        trigger.copy(65, size);
    }

    function checkOutOfBoundsClearsReturnData(uint256 size) public {
        vm.assume(size <= 0);
        (bool ok, bytes memory data) = address(trigger).call(
            abi.encodeCall(SymbolicReturndataCopyOobOffsetTrigger.copy, (65, size))
        );
        assertFalse(ok);
        assertEq(data.length, 0);
    }

}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkOutOfBoundsOffsetForcedZeroSizeReverts|checkOutOfBoundsClearsReturnData",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 2 tests for test/SymbolicReturndataCopyOobOffset.t.sol:SymbolicReturndataCopyOobOffset
[PASS] checkOutOfBoundsClearsReturnData(uint256) ([METRICS])
[PASS] checkOutOfBoundsOffsetForcedZeroSizeReverts(uint256) ([METRICS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_return_revert_accept_symbolic_offset(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-contract",
        "SymbolicReturnRevertOffset",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 2 tests for test/SymbolicReturnRevertOffset.t.sol:SymbolicReturnRevertOffset
[PASS] checkSymbolicReturnOffset(uint16,uint256) ([METRICS])
[PASS] checkSymbolicRevertOffset(uint16,uint256) ([METRICS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic RETURN offset"), "{stdout}");
    assert!(!stdout.contains("symbolic REVERT offset"), "{stdout}");
}

// MCOPY with a symbolic source and RETURN / REVERT with a bounded symbolic size are modeled.
#[forgetest_init]
fn symbolic_mcopy_return_revert_accept_symbolic_operands(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_mcopy_return_revert_accept_symbolic_operands");

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

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkSymbolicMcopy|checkSymbolicReturnSize|checkSymbolicRevertSize)\\(",
    ]))
    .success()
    .stdout_eq(
        str![[r#"
...
Ran 1 test for test/SymbolicRevertSize.t.sol:SymbolicRevertSize
[PASS] checkSymbolicRevertSize(uint8,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicReturnSize.t.sol:SymbolicReturnSize
[PASS] checkSymbolicReturnSize(uint8,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicMcopy.t.sol:SymbolicMcopy
[PASS] checkSymbolicMcopy(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]
        .unordered(),
    )
    .get_output()
    .stdout_lossy();
    for reason in ["symbolic MCOPY src", "symbolic RETURN size", "symbolic REVERT size"] {
        assert!(!stdout.contains(reason), "{stdout}");
    }
}

// A chain of hashes over the previous hash shares every earlier preimage. Naming and searching such
// expressions must stay linear in the chain length; before the fix, six rounds ran out of memory.
#[forgetest_init]
fn symbolic_chained_keccak_stays_linear(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_chained_keccak_stays_linear");

    prj.add_test(
        "SymbolicKeccakChain.t.sol",
        r#"
contract SymbolicKeccakChain {
    uint256 public sink;

    function checkKeccakChain(uint256 seed) external {
        uint256 s = seed;
        for (uint256 i; i < 16; ++i) {
            s = uint256(keccak256(abi.encode(s))) + 1;
        }
        sink = s;
    }
}
"#,
    );

    cmd.args(["test", "--symbolic", "--match-test", "checkKeccakChain"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] checkKeccakChain(uint256) [..]
...
"#]]);
}
