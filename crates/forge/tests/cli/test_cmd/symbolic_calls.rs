use super::symbolic_helpers::assert_relevant_lines;
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, util::OutputExt};

use super::symbolic_helpers::z3_available;

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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicCalldataLoad(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicCalldataCopy(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicCalldataCopyDest(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicCalldataCopySize(uint8)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCalldataCopyDestAndSize(uint8,uint8,bytes32)
"#]],
    );
    assert!(!stdout.contains("symbolic CALLDATACOPY dest"), "{stdout}");
    assert!(!stdout.contains("symbolic CALLDATACOPY size"), "{stdout}");
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicCallInputOffset(uint16,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicCallOutputSize(uint8)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicCallInputSize(uint8,uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic CALL input size"), "{stdout}");
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkExternal(uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkLowLevel(uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkNoBackdoor(bytes4,uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
args=
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkNoBadTarget(address,uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
args=
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkTarget(address,uint256)
"#]],
    );
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

        assert_relevant_lines(
            &stdout,
            foundry_test_utils::str![[r#"
symbolic CALL target
"#]],
        );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkUnbounded(address)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkDelegateTarget(address,uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkDelegateTarget(address,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkUnknownSelector(bytes4)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSvmBytes4Selector()
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSvmCreateCalldata()
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkRequireCall(uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkStatic(uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkDelegate(uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkValueTransfer()
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicValueTransfer(uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicInsufficientValue(uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicCallcodeValue(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic CALLCODE value"), "{stdout}");
});
