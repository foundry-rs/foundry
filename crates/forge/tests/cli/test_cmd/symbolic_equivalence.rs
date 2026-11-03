//! `forge equivalence` checks of two runtime bytecodes.

use crate::skip_unless_z3;
use foundry_test_utils::{forgetest, util::OutputExt};

// Returns `calldataload(4) + 1` with the addition's operands in either order.
const ADD_ONE: &str = "0x60043560010160005260206000f3";
const ADD_ONE_COMMUTED: &str = "0x60016004350160005260206000f3";
// Returns `calldataload(4) | 1`.
const OR_ONE: &str = "0x60043560011760005260206000f3";
// `sstore(0, calldataload(4))`, and the same followed by `sstore(1, 0)`.
const STORE: &str = "0x60043560005500";
const STORE_AND_CLEAR: &str = "0x600435600055600060015500";

#[forgetest]
fn equivalence_proves_commuted_addition(cmd: _) {
    skip_unless_z3!("equivalence_proves_commuted_addition");
    let out = cmd
        .args(["equivalence", "--code-a", ADD_ONE, "--code-b", ADD_ONE_COMMUTED])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(out.contains("Equivalent: no explored input distinguishes"), "{out}");
}

#[forgetest]
fn equivalence_reports_replayed_counterexample(cmd: _) {
    skip_unless_z3!("equivalence_reports_replayed_counterexample");
    let out = cmd
        .args(["equivalence", "--code-a", ADD_ONE, "--code-b", OR_ONE])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(out.contains("Not equivalent: the contracts differ on calldata 0x"), "{out}");
    assert!(out.contains("code-a: returned 0x"), "{out}");
    assert!(out.contains("code-b: returned 0x"), "{out}");
}

#[forgetest]
fn equivalence_limits_calldata_to_signature(cmd: _) {
    skip_unless_z3!("equivalence_limits_calldata_to_signature");
    let out = cmd
        .args([
            "equivalence",
            "--code-a",
            ADD_ONE,
            "--code-b",
            OR_ONE,
            "--sig",
            "f(uint256)",
            "--json",
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    let json: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(json["status"], "not_equivalent");
    assert!(json["calldata"].as_str().unwrap().starts_with("0xb3de648b"), "{json}");
    assert_eq!(json["args"].as_array().unwrap().len(), 1);
}

#[forgetest]
fn equivalence_compares_storage_from_empty_or_symbolic_state(cmd: _) {
    skip_unless_z3!("equivalence_compares_storage_from_empty_or_symbolic_state");
    // Clearing an empty slot changes nothing when storage starts empty.
    cmd.args(["equivalence", "--code-a", STORE, "--code-b", STORE_AND_CLEAR]).assert_success();

    // From arbitrary storage, slot 1 may hold a value that only the second code clears.
    let out = cmd
        .forge_fuse()
        .args(["equivalence", "--code-a", STORE, "--code-b", STORE_AND_CLEAR, "--symbolic-storage"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(out.contains("storage slot 0x1: code-a "), "{out}");
    assert!(out.contains(", code-b 0x0"), "{out}");
}

#[forgetest]
fn equivalence_finds_checked_overflow_in_solidity(prj: _, cmd: _) {
    skip_unless_z3!("equivalence_finds_checked_overflow_in_solidity");
    prj.add_source(
        "Counters.sol",
        r#"
contract Checked {
    uint256 public x;
    function set(uint256 v) external { x = v; }
    function inc() external { x += 1; }
}

contract Unchecked {
    uint256 public x;
    function set(uint256 v) external { assembly { sstore(0, v) } }
    function inc() external { unchecked { x = x + 1; } }
}
"#,
    );
    let checked = cmd
        .args(["inspect", "Checked", "deployedBytecode"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let unchecked = cmd
        .forge_fuse()
        .args(["inspect", "Unchecked", "deployedBytecode"])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let (checked, unchecked) = (checked.trim().to_string(), unchecked.trim().to_string());

    // From empty storage one call cannot overflow the counter.
    cmd.forge_fuse()
        .args(["equivalence", "--code-a", &checked, "--code-b", &unchecked])
        .assert_success();

    // From arbitrary storage the checked increment reverts with Panic(0x11) where the other wraps.
    let out = cmd
        .forge_fuse()
        .args(["equivalence", "--code-a", &checked, "--code-b", &unchecked, "--symbolic-storage"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(
        out.contains(
            "code-a: reverted with \
             0x4e487b710000000000000000000000000000000000000000000000000000000000000011"
        ),
        "{out}"
    );
    assert!(out.contains("code-b: returned no data"), "{out}");
}
