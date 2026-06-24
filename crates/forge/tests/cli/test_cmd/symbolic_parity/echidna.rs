use super::{assert_relevant_lines, assert_symbolic_witness, json_test_result, read_artifact_ref};
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str, util::OutputExt};

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
    bool public flag0 = true;
    bool public flag1 = true;

    function set0(int256 val) public {
        if (val % 100 == 0) flag0 = false;
    }

    function set1(int256 val) public {
        if (val % 10 == 0 && !flag0) flag1 = false;
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

    let output = cmd
        .args([
            "test",
            "--symbolic",
            "--json",
            "--match-test",
            "invariant_flag1_holds",
            "--fuzz-seed",
            "1",
        ])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "invariant_flag1_holds()");
    let failures = result["invariant_failures"].as_array().expect("invariant failures");
    let failure = failures.first().expect("invariant failure");
    let minimization = &failure["minimization"];
    assert_eq!(failure["artifact"], minimization["minimized"]);
    assert_eq!(minimization["minimized_sequence_len"], 2);
    assert!(
        minimization["original_sequence_len"].as_u64().unwrap()
            > minimization["minimized_sequence_len"].as_u64().unwrap()
    );
    assert!(minimization["accepted"].as_u64().unwrap() > 0);

    let original = read_artifact_ref(&minimization["original"]);
    let minimized = read_artifact_ref(&minimization["minimized"]);
    assert_eq!(original["replay"]["status"], "confirmed");
    assert_eq!(minimized["replay"]["status"], "confirmed");
    assert_eq!(minimized["calls"].as_array().unwrap().len(), 2);

    let calls = minimized["calls"].as_array().unwrap();
    assert_eq!(calls[0]["function_name"], "set0");
    assert_eq!(calls[0]["args"], "0");
    assert_eq!(calls[1]["function_name"], "set1");
    assert_eq!(calls[1]["args"], "0");

    let artifact_path = minimization["minimized"]["path"].as_str().unwrap();
    let replay_stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", artifact_path])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &replay_stdout,
        str![[r#"
[FAIL:
"#]],
    );
});

// ---------------------------------------------------------------------------
// Echidna basic/revert.sol — stateful revert counterexample shrinking.
// ---------------------------------------------------------------------------
// Source: https://github.com/crytic/echidna/blob/master/tests/solidity/basic/revert.sol
// Echidna's suite asserts this shrinks to one `f(int,address,address)` call.
forgetest_init!(echidna_revert_magic_args_parity, |prj, cmd| {
    skip_unless_z3!("echidna_revert_magic_args_parity");

    prj.add_test(
        "EchidnaRevertParity.t.sol",
        r#"
import "forge-std/Test.sol";

contract EchidnaRevertTarget {
    int256 private state;

    function f(int256 x, address, address z) public {
        require(z != address(0));
        state = x;
    }

    function echidna_fails_on_revert() public view returns (bool) {
        if (state < 0) revert();
        return true;
    }
}

contract EchidnaRevertParity is Test {
    EchidnaRevertTarget target;

    function setUp() public {
        target = new EchidnaRevertTarget();
        targetContract(address(target));
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 20
    /// forge-config: default.invariant.shrink_run_limit = 10000
    function invariant_no_revert() public view {
        assertTrue(target.echidna_fails_on_revert());
    }
}
"#,
    );

    let output = cmd
        .args([
            "test",
            "--symbolic",
            "--json",
            "--match-test",
            "invariant_no_revert",
            "--fuzz-seed",
            "1",
        ])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "invariant_no_revert()");
    let failure = result["invariant_failures"].as_array().unwrap().first().unwrap();
    let minimization = &failure["minimization"];
    assert_eq!(failure["artifact"], minimization["minimized"]);
    assert_eq!(minimization["minimized_sequence_len"], 1);
    assert!(
        minimization["original_sequence_len"].as_u64().unwrap()
            > minimization["minimized_sequence_len"].as_u64().unwrap()
    );
    assert!(minimization["accepted"].as_u64().unwrap() > 0);

    let minimized = read_artifact_ref(&minimization["minimized"]);
    assert_eq!(minimized["replay"]["status"], "confirmed");
    let call = &minimized["calls"][0];
    assert_eq!(call["function_name"], "f");
    let args = call["args"].as_str().unwrap();
    let args: Vec<_> = args.split(',').map(str::trim).collect();
    assert_eq!(args.len(), 3);
    assert_eq!(args[0], "-1");
    assert_eq!(args[1], "0x0000000000000000000000000000000000000000");
    assert_ne!(args[2], "0x0000000000000000000000000000000000000000");

    let replay_stdout = cmd
        .forge_fuse()
        .args([
            "test",
            "--replay-symbolic-artifact",
            minimization["minimized"]["path"].as_str().unwrap(),
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &replay_stdout,
        str![[r#"
invariant_no_revert()
"#]],
    );
});

// ---------------------------------------------------------------------------
// Echidna values/darray.sol — dynamic address[] calldata shrinking.
// ---------------------------------------------------------------------------
// Source: https://github.com/crytic/echidna/blob/master/tests/solidity/values/darray.sol
// Echidna shrinks to one address element: `[0x123456]`.
forgetest_init!(echidna_dynamic_address_array_parity, |prj, cmd| {
    skip_unless_z3!("echidna_dynamic_address_array_parity");

    prj.add_test(
        "EchidnaDarrayParity.t.sol",
        r#"
contract EchidnaDarrayParity {
    address constant TARGET = address(0x123456);

    /// forge-config: default.symbolic.array_lengths = [4]
    function checkDarray(address[] memory xs) public pure {
        for (uint256 i; i < xs.length; i++) {
            assert(xs[i] != TARGET);
        }
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--json", "--match-test", "checkDarray"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "checkDarray(address[])");
    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["status"], "fail_counterexample");
    assert_eq!(symbolic["minimization"]["minimized"], symbolic["artifact"]);
    assert!(symbolic["minimization"]["accepted"].as_u64().unwrap() > 0);

    let minimized = read_artifact_ref(&symbolic["artifact"]);
    assert_eq!(minimized["replay"]["status"], "confirmed");
    assert_eq!(minimized["calls"][0]["args"], "[0x0000000000000000000000000000000000123456]");

    let replay_stdout = cmd
        .forge_fuse()
        .args([
            "test",
            "--replay-symbolic-artifact",
            symbolic["artifact"]["path"].as_str().unwrap(),
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &replay_stdout,
        str![[r#"
args=[[0x0000000000000000000000000000000000123456]]
"#]],
    );
});

// ---------------------------------------------------------------------------
// Echidna basic/darray-mutation.sol — dynamic bytes calldata shrinking.
// ---------------------------------------------------------------------------
// Source: https://github.com/crytic/echidna/blob/master/tests/solidity/basic/darray-mutation.sol
// Echidna shrinks to the required `abc` prefix with a length still greater than
// 16; Foundry also zeroes irrelevant suffix bytes.
forgetest_init!(echidna_bytes_mutation_parity, |prj, cmd| {
    skip_unless_z3!("echidna_bytes_mutation_parity");

    prj.add_test(
        "EchidnaBytesMutationParity.t.sol",
        r#"
contract EchidnaBytesMutationParity {
    /// forge-config: default.symbolic.array_lengths = [18]
    function checkMutated(bytes memory bs) public pure {
        assert(bs.length <= 16 || bs[0] != 0x61 || bs[1] != 0x62 || bs[2] != 0x63);
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--json", "--match-test", "checkMutated"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "checkMutated(bytes)");
    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["status"], "fail_counterexample");
    assert_eq!(symbolic["minimization"]["minimized"], symbolic["artifact"]);
    assert!(symbolic["minimization"]["accepted"].as_u64().unwrap() > 0);

    let minimized = read_artifact_ref(&symbolic["artifact"]);
    assert_eq!(minimized["replay"]["status"], "confirmed");
    assert_eq!(minimized["calls"][0]["args"], "0x6162630000000000000000000000000000");

    let replay_stdout = cmd
        .forge_fuse()
        .args([
            "test",
            "--replay-symbolic-artifact",
            symbolic["artifact"]["path"].as_str().unwrap(),
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &replay_stdout,
        str![[r#"
args=[0x6162630000000000000000000000000000]
"#]],
    );
});

// ---------------------------------------------------------------------------
// Echidna values/payable.sol — transaction value minimization.
// ---------------------------------------------------------------------------
// Source: https://github.com/crytic/echidna/blob/master/tests/solidity/values/payable.sol
// Echidna shrinks the value-bearing call to value 129.
forgetest_init!(echidna_payable_value_parity, |prj, cmd| {
    skip_unless_z3!("echidna_payable_value_parity");

    prj.add_test(
        "EchidnaPayableParity.t.sol",
        r#"
import "forge-std/Test.sol";

contract EchidnaPayableTarget {
    bool public touched;

    function payable_function() public payable {
        if (msg.value >= 129) touched = true;
    }

    function another_function(uint256) public {}
}

contract EchidnaPayableParity is Test {
    EchidnaPayableTarget target;
    address sender = address(0xBEEF);

    function setUp() public {
        target = new EchidnaPayableTarget();
        vm.deal(sender, 1000 ether);
        targetSender(sender);
        targetContract(address(target));
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 20
    /// forge-config: default.invariant.shrink_run_limit = 10000
    function invariant_payable_zero() public view {
        assertFalse(target.touched());
    }
}
"#,
    );

    let output = cmd
        .args([
            "test",
            "--symbolic",
            "--json",
            "--match-test",
            "invariant_payable_zero",
            "--fuzz-seed",
            "1",
        ])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "invariant_payable_zero()");
    let failure = result["invariant_failures"].as_array().unwrap().first().unwrap();
    let minimization = &failure["minimization"];
    assert_eq!(failure["artifact"], minimization["minimized"]);
    assert_eq!(minimization["minimized_sequence_len"], 1);
    assert!(
        minimization["original_sequence_len"].as_u64().unwrap()
            > minimization["minimized_sequence_len"].as_u64().unwrap()
    );
    assert!(minimization["accepted"].as_u64().unwrap() > 0);

    let minimized = read_artifact_ref(&minimization["minimized"]);
    assert_eq!(minimized["replay"]["status"], "confirmed");
    let call = &minimized["calls"][0];
    assert_eq!(call["function_name"], "payable_function");
    assert_eq!(call["value"].as_str().unwrap_or_default(), "0x81");

    let replay_stdout = cmd
        .forge_fuse()
        .args([
            "test",
            "--replay-symbolic-artifact",
            minimization["minimized"]["path"].as_str().unwrap(),
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &replay_stdout,
        str![[r#"
invariant_payable_zero()
"#]],
    );
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
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkNoOverflow(uint256,uint256) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
});
