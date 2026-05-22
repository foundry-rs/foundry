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
