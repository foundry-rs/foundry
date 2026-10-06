use super::symbolic_helpers::{assert_symbolic, assert_symbolic_witness, z3_available};
use crate::skip_unless_z3;
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, snapbox::IntoData, str, util::OutputExt};

#[forgetest_init]
fn symbolic_call_contains_invalid_child_halt(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_call_contains_invalid_child_halt because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInvalidChildCall.t.sol",
        r#"
contract InvalidChild {
    fallback() external {
        assembly ("memory-safe") {
            invalid()
        }
    }
}

contract SymbolicInvalidChildCall {
    uint256 marker;

    function checkInvalidChildCall() public {
        InvalidChild child = new InvalidChild();
        marker = 17;
        (bool success, bytes memory output) = address(child).call("");
        assert(!success);
        assert(output.length == 0);
        assert(marker == 17);
    }
}
"#,
    );

    cmd.args(["test", "--symbolic", "--match-test", "checkInvalidChildCall"]).assert_success();
}

#[forgetest_init]
fn symbolic_assume_no_revert_does_not_prune_invalid_child_halt(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_assume_no_revert_does_not_prune_invalid_child_halt because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicAssumeNoRevertInvalidChild.t.sol",
        r#"
import "forge-std/Test.sol";

contract InvalidAssumeNoRevertChild {
    fallback() external {
        assembly ("memory-safe") {
            invalid()
        }
    }
}

contract SymbolicAssumeNoRevertInvalidChild is Test {
    function checkAssumeNoRevertInvalidChild() public {
        InvalidAssumeNoRevertChild child = new InvalidAssumeNoRevertChild();
        vm.assumeNoRevert();
        (bool success,) = address(child).call("");
        assertTrue(success);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkAssumeNoRevertInvalidChild",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicAssumeNoRevertInvalidChild.t.sol:SymbolicAssumeNoRevertInvalidChild
[FAIL: assertion failed; counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkAssumeNoRevertInvalidChild() ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

// CALLDATALOAD and CALLDATACOPY with symbolic offsets, destination and size are modeled.
#[forgetest_init]
fn symbolic_calldata_ops_accept_symbolic_operands(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_calldata_ops_accept_symbolic_operands");

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

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkSymbolicCalldataLoad|checkSymbolicCalldataCopy|checkSymbolicCalldataCopyDest|checkSymbolicCalldataCopySize|checkCalldataCopyDestAndSize)\\(",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicCalldataCopySize.t.sol:SymbolicCalldataCopySize
[PASS] checkSymbolicCalldataCopySize(uint8) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCalldataLoad.t.sol:SymbolicCalldataLoad
[PASS] checkSymbolicCalldataLoad(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCalldataCopyDest.t.sol:SymbolicCalldataCopyDest
[PASS] checkSymbolicCalldataCopyDest(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCalldataCopy.t.sol:SymbolicCalldataCopy
[PASS] checkSymbolicCalldataCopy(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCalldataCopyDestAndSize.t.sol:SymbolicCalldataCopyDestAndSize
[PASS] checkCalldataCopyDestAndSize(uint8,uint8,bytes32) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]].unordered())
    .get_output()
    .stdout_lossy();
    for reason in [
        "symbolic CALLDATACOPY dest",
        "symbolic CALLDATACOPY offset",
        "symbolic CALLDATACOPY size",
        "symbolic CALLDATALOAD offset",
    ] {
        assert!(!stdout.contains(reason), "{stdout}");
    }
}

// CALL with a symbolic input offset and bounded symbolic input and output sizes is modeled.
#[forgetest_init]
fn symbolic_call_accepts_symbolic_memory_operands(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_call_accepts_symbolic_memory_operands");

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

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkSymbolicCallInputOffset|checkSymbolicCallOutputSize|checkSymbolicCallInputSize)\\(",
    ]))
    .success()
    .stdout_eq(
        str![[r#"
...
Ran 1 test for test/SymbolicCallInputSize.t.sol:SymbolicCallInputSize
[PASS] checkSymbolicCallInputSize(uint8,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCallOutputSize.t.sol:SymbolicCallOutputSize
[PASS] checkSymbolicCallOutputSize(uint8) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCallInputOffset.t.sol:SymbolicCallInputOffset
[PASS] checkSymbolicCallInputOffset(uint16,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]
        .unordered(),
    )
    .get_output()
    .stdout_lossy();
    for reason in
        ["symbolic CALL input offset", "symbolic CALL input size", "symbolic CALL output size"]
    {
        assert!(!stdout.contains(reason), "{stdout}");
    }
}

#[forgetest_init]
fn symbolic_executes_typed_external_call(prj: _, cmd: _) {
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

    let stdout =
        assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkExternal"]))
            .failure()
            .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicExternalCall.t.sol:SymbolicExternalCall
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkExternal(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
            .get_output()
            .stdout_lossy();

    assert!(!stdout.contains("unsupported symbolic execution feature: external CALL"), "{stdout}");
}

#[forgetest_init]
fn symbolic_executes_low_level_external_call(prj: _, cmd: _) {
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

    let stdout =
        assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkLowLevel"]))
            .failure()
            .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicLowLevelCall.t.sol:SymbolicLowLevelCall
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkLowLevel(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
            .get_output()
            .stdout_lossy();

    assert!(!stdout.contains("unsupported symbolic execution feature: external CALL"), "{stdout}");
}

#[forgetest_init]
fn symbolic_external_call_with_symbolic_selector_finds_backdoor(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkNoBackdoor",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicSelectorBackdoor.t.sol:SymbolicSelectorBackdoor
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkNoBackdoor(bytes4,uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic external CALL selector"), "{stdout}");
}

#[forgetest_init]
fn symbolic_external_call_with_symbolic_target_finds_backdoor(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkNoBadTarget",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicTargetBackdoor.t.sol:SymbolicTargetBackdoor
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkNoBadTarget(address,uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic CALL target outside known contracts"), "{stdout}");
}

#[forgetest_init]
fn symbolic_call_target_explores_mock_mismatch(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_call_target_explores_mock_mismatch because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicTargetMockMismatch.t.sol",
        r#"
import "forge-std/Test.sol";

contract RealToken {
    function balanceOf(address) external pure returns (uint256) {
        return 7;
    }
}

contract FiveToken {
    function balanceOf(address) external pure returns (uint256) {
        return 5;
    }
}

contract SymbolicTargetMockMismatch is Test {
    RealToken real;
    FiveToken five;

    function setUp() public {
        real = new RealToken();
        five = new FiveToken();
    }

    // The mock only covers `balanceOf(user)`; for `user != this` the real code answers 7.
    function checkMockedSymbolicTargetMayMiss(address callee, address user) public {
        vm.assume(callee == address(real) || callee == address(five));
        vm.mockCall(
            address(real),
            abi.encodeWithSelector(RealToken.balanceOf.selector, user),
            abi.encode(uint256(5))
        );
        assert(RealToken(callee).balanceOf(address(this)) == 5);
    }

    function checkMockedSymbolicTargetAlwaysHits(address callee) public {
        vm.assume(callee == address(real) || callee == address(five));
        vm.mockCall(
            address(real),
            abi.encodeWithSelector(RealToken.balanceOf.selector, address(this)),
            abi.encode(uint256(5))
        );
        assert(RealToken(callee).balanceOf(address(this)) == 5);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-contract",
        "SymbolicTargetMockMismatch",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 2 tests for test/SymbolicTargetMockMismatch.t.sol:SymbolicTargetMockMismatch
[PASS] checkMockedSymbolicTargetAlwaysHits(address) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkMockedSymbolicTargetMayMiss(address,address) ([METRICS])
Suite result: FAILED. 1 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

// Call targets: known and symbolic targets, unknown selectors and failing callees.
#[forgetest_init]
fn symbolic_call_targets_and_dispatch(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_call_targets_and_dispatch");

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

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkTarget|checkDelegateTarget|checkUnknownSelector|checkRequireCall)\\(",
    ]))
    .success()
    .stdout_eq(
        str![[r#"
...
Ran 1 test for test/SymbolicTargetDefaultAuto.t.sol:SymbolicTargetDefaultAuto
[PASS] checkTarget(address,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicExternalRequire.t.sol:SymbolicExternalRequire
[PASS] checkRequireCall(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicDelegateTarget.t.sol:SymbolicDelegateTarget
[PASS] checkDelegateTarget(address,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicUnknownSelector.t.sol:SymbolicUnknownSelector
[PASS] checkUnknownSelector(bytes4) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]
        .unordered(),
    )
    .get_output()
    .stdout_lossy();
    for reason in [
        "symbolic CALL target",
        "symbolic external CALL selector",
        "unsupported symbolic execution feature: external CALL",
    ] {
        assert!(!stdout.contains(reason), "{stdout}");
    }
}

#[forgetest_init]
fn symbolic_external_call_with_unbounded_symbolic_target_requires_config(prj: _, cmd: _) {
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

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkTarget"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicTargetDefaultOff.t.sol:SymbolicTargetDefaultOff
[FAIL: incomplete symbolic execution (Stuck): unsupported symbolic execution feature: symbolic CALL target] checkTarget(address) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_external_call_with_empty_unknown_target_is_modeled(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_external_call_with_empty_unknown_target_is_modeled because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicUnboundedTarget.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicUnboundedTarget is Test {
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

    /// forge-config: default.symbolic.symbolic_call_targets = true
    function checkUnboundedInsufficientValue(address target) public {
        vm.assume(uint160(target) > 9 && target != address(this));
        vm.deal(address(this), 0);
        bool ok;
        assembly {
            ok := call(gas(), target, 1, 0, 0, 0, 0)
        }
        assert(!ok);
    }
}
"#,
    );

    let stdout =
        assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkUnbounded"]))
            .success()
            .stdout_eq(str![[r#"
...
Ran 2 tests for test/SymbolicUnboundedTarget.t.sol:SymbolicUnboundedTarget
[PASS] checkUnbounded(address) ([METRICS])
[PASS] checkUnboundedInsufficientValue(address) ([METRICS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
            .get_output()
            .stdout_lossy();

    assert!(!stdout.contains("symbolic CALL target outside known contracts"), "{stdout}");
}

#[forgetest_init]
fn symbolic_svm_create_bytes4_can_drive_selector_dispatch(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkSvmBytes4Selector",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicSvmBytes4Selector.t.sol:SymbolicSvmBytes4Selector
[PASS] checkSvmBytes4Selector() ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic external CALL selector"), "{stdout}");
}

#[forgetest_init]
fn symbolic_svm_create_calldata_generates_bounded_dispatch(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkSvmCreateCalldata",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicSvmCreateCalldata.t.sol:SymbolicSvmCreateCalldata
[PASS] checkSvmCreateCalldata() ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic external CALL selector"), "{stdout}");
}

// STATICCALL and DELEGATECALL context rules.
#[forgetest_init]
fn symbolic_call_context_semantics(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_call_context_semantics");

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

    prj.add_test(
        "SymbolicStaticCallValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract StaticValueSink {
    receive() external payable {}
}

contract StaticValueHelper {
    function callWithValue(address target, uint256 amount) external returns (bool ok) {
        assembly {
            ok := call(gas(), target, amount, 0, 0, 0, 0)
        }
    }
}

contract SymbolicStaticCallValue is Test {
    StaticValueSink sink;
    StaticValueHelper helper;

    function setUp() public {
        sink = new StaticValueSink();
        helper = new StaticValueHelper();
    }

    function checkStaticCallValue(uint256 amount) public {
        vm.assume(amount <= 1);
        (bool ok,) = address(helper).staticcall(
            abi.encodeCall(StaticValueHelper.callWithValue, (address(sink), amount))
        );
        assertEq(ok, amount == 0);
    }
}
"#,
    );

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

    assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkStatic|checkStaticCallValue|checkDelegate)\\(",
    ]))
    .success()
    .stdout_eq(
        str![[r#"
...
Ran 1 test for test/SymbolicDelegateCall.t.sol:SymbolicDelegateCall
[PASS] checkDelegate(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicStaticCall.t.sol:SymbolicStaticCall
[PASS] checkStatic(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicStaticCallValue.t.sol:SymbolicStaticCallValue
[PASS] checkStaticCallValue(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]
        .unordered(),
    );
}

// CALL with concrete and insufficient symbolic value.
#[forgetest_init]
fn symbolic_call_value_transfers(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_call_value_transfers");

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

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkValueTransfer|checkSymbolicInsufficientValue)\\(",
    ]))
    .success()
    .stdout_eq(
        str![[r#"
...
Ran 1 test for test/SymbolicValueCall.t.sol:SymbolicValueCall
[PASS] checkValueTransfer() ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicInsufficientValueCall.t.sol:SymbolicInsufficientValueCall
[PASS] checkSymbolicInsufficientValue(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]
        .unordered(),
    )
    .get_output()
    .stdout_lossy();
    assert!(!stdout.contains("symbolic external CALL value"), "{stdout}");
}

#[forgetest_init]
fn symbolic_call_accepts_symbolic_value(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkSymbolicValueTransfer",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicValueCall.t.sol:SymbolicValueCall
[PASS] checkSymbolicValueTransfer(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic external CALL value"), "{stdout}");
}

#[forgetest_init]
fn symbolic_callcode_accepts_symbolic_value(prj: _, cmd: _) {
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
        vm.assume(amount <= 8);
        vm.deal(address(this), 7);

        bytes memory input = abi.encodeWithSelector(SymbolicCallcodeValueTarget.echoValue.selector);
        uint256 echoed;
        bool ok;
        address callTarget = address(target);
        assembly {
            ok := callcode(gas(), callTarget, amount, add(input, 0x20), mload(input), 0x80, 0x20)
            echoed := mload(0x80)
        }

        assertEq(ok, amount <= 7);
        if (ok) {
            assertEq(echoed, amount);
        }
    }

    function checkPrankedCallcodeValue(uint256 amount) public {
        vm.assume(amount <= 1);
        address caller = address(0xBEEF);
        vm.deal(address(this), 0);
        vm.deal(caller, 1);
        vm.prank(caller);

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
        assertEq(caller.balance, 1 - amount);
        assertEq(address(this).balance, amount);
    }

    function checkPrankedCallcodeOverflow() public {
        address caller = address(0xBEEF);
        vm.deal(address(this), type(uint256).max);
        vm.deal(caller, 1);
        vm.prank(caller);

        bool ok;
        address callTarget = address(target);
        assembly {
            ok := callcode(gas(), callTarget, 1, 0, 0, 0, 0)
        }

        assert(!ok);
        assertEq(caller.balance, 1);
        assertEq(address(this).balance, type(uint256).max);
    }

    function checkMockedPrankedCallcodeValue() public {
        address caller = address(0xBEEF);
        bytes memory input = abi.encodeWithSelector(SymbolicCallcodeValueTarget.echoValue.selector);
        vm.mockCall(address(target), 1, input, abi.encode(uint256(99)));
        vm.deal(address(this), 0);
        vm.deal(caller, 1);
        vm.prank(caller);

        uint256 echoed;
        bool ok;
        address callTarget = address(target);
        assembly {
            ok := callcode(gas(), callTarget, 1, add(input, 0x20), mload(input), 0x80, 0x20)
            echoed := mload(0x80)
        }

        assert(ok);
        assertEq(echoed, 99);
        assertEq(caller.balance, 0);
        assertEq(address(this).balance, 1);

        vm.deal(caller, 0);
        vm.prank(caller);
        assembly {
            ok := callcode(gas(), callTarget, 1, add(input, 0x20), mload(input), 0, 0)
        }
        assert(!ok);
        assertEq(caller.balance, 0);
        assertEq(address(this).balance, 1);
    }
}
"#,
    );

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "check.*Callcode",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 4 tests for test/SymbolicCallcodeValue.t.sol:SymbolicCallcodeValue
[PASS] checkMockedPrankedCallcodeValue() ([METRICS])
[PASS] checkPrankedCallcodeOverflow() ([METRICS])
[PASS] checkPrankedCallcodeValue(uint256) ([METRICS])
[PASS] checkSymbolicCallcodeValue(uint256) ([METRICS])
Suite result: ok. 4 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic CALLCODE value"), "{stdout}");
}
