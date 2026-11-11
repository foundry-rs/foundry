//! `forge equivalence` checks of two runtime bytecodes.

use crate::skip_unless_z3;
use foundry_test_utils::{forgetest, str, util::OutputExt};

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
    cmd.args(["equivalence", "--code-a", STORE, "--code-b", STORE_AND_CLEAR, "--empty-storage"])
        .assert_success();

    // From arbitrary storage, the default, slot 1 may hold a value that only the second code
    // clears.
    let out = cmd
        .forge_fuse()
        .args(["equivalence", "--code-a", STORE, "--code-b", STORE_AND_CLEAR])
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
        .args(["equivalence", "--code-a", &checked, "--code-b", &unchecked, "--empty-storage"])
        .assert_success();

    // From arbitrary storage the checked increment reverts with Panic(0x11) where the other wraps.
    let out = cmd
        .forge_fuse()
        .args(["equivalence", "--code-a", &checked, "--code-b", &unchecked])
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

fn deployed_bytecode(cmd: &mut foundry_test_utils::TestCommand, contract: &str) -> String {
    cmd.forge_fuse()
        .args(["inspect", contract, "deployedBytecode"])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string()
}

#[forgetest]
fn equivalence_compares_logs(prj: _, cmd: _) {
    skip_unless_z3!("equivalence_compares_logs");
    prj.add_source(
        "Logs.sol",
        r#"
contract Emit {
    event Set(uint256 v);
    function set(uint256 v) external { emit Set(v); }
}

contract EmitOffByOne {
    event Set(uint256 v);
    function set(uint256 v) external { emit Set(v == 7 ? 8 : v); }
}

contract EmitAfterLoop {
    event Set(uint256 v);
    function set(uint256 v) external {
        uint256 w = v;
        for (uint256 i; i < 3; i++) {
            w = w ^ i ^ i;
        }
        emit Set(w);
    }
}
"#,
    );
    let emit = deployed_bytecode(&mut cmd, "Emit");
    let off_by_one = deployed_bytecode(&mut cmd, "EmitOffByOne");
    let after_loop = deployed_bytecode(&mut cmd, "EmitAfterLoop");

    let out = cmd
        .forge_fuse()
        .args(["equivalence", "--code-a", &emit, "--code-b", &off_by_one, "--sig", "set(uint256)"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(out.contains("args:   (7)"), "{out}");
    assert!(out.contains("code-a logs:"), "{out}");
    assert!(
        out.contains("data 0x0000000000000000000000000000000000000000000000000000000000000008"),
        "{out}"
    );

    // Spending more gas on the same events is not a difference.
    cmd.forge_fuse()
        .args(["equivalence", "--code-a", &emit, "--code-b", &after_loop, "--sig", "set(uint256)"])
        .assert_success();
}

#[forgetest]
fn equivalence_compares_balances_without_gas(prj: _, cmd: _) {
    skip_unless_z3!("equivalence_compares_balances_without_gas");
    prj.add_source(
        "Refunds.sol",
        r#"
contract Refund {
    function pay(uint256 amt) external {
        require(amt <= 100);
        (bool ok,) = msg.sender.call{value: amt}("");
        require(ok);
    }
}

contract RefundShort {
    function pay(uint256 amt) external {
        require(amt <= 100);
        (bool ok,) = msg.sender.call{value: amt == 42 ? 41 : amt}("");
        require(ok);
    }
}

contract RefundAfterLoop {
    function pay(uint256 amt) external {
        require(amt <= 100);
        uint256 x;
        for (uint256 i; i < 5; i++) {
            x += i;
        }
        (bool ok,) = msg.sender.call{value: amt + x - 10}("");
        require(ok);
    }
}

contract KeepChange {
    function buy() external payable {
        require(msg.value >= 10);
        (bool ok,) = msg.sender.call{value: msg.value - 10}("");
        require(ok);
    }
}

contract KeepAll {
    function buy() external payable {
        require(msg.value >= 10);
    }
}
"#,
    );
    let refund = deployed_bytecode(&mut cmd, "Refund");
    let short = deployed_bytecode(&mut cmd, "RefundShort");
    let after_loop = deployed_bytecode(&mut cmd, "RefundAfterLoop");
    let keep_change = deployed_bytecode(&mut cmd, "KeepChange");
    let keep_all = deployed_bytecode(&mut cmd, "KeepAll");

    let out = cmd
        .forge_fuse()
        .args([
            "equivalence",
            "--code-a",
            &refund,
            "--code-b",
            &short,
            "--sig",
            "pay(uint256)",
            "--promise-no-reent",
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    // The contract's balance is symbolic too, so the solver may also pick one that covers only
    // the shorter refund.
    assert!(out.contains("args:   (42)"), "{out}");

    // Gas is not priced, so a costlier refund of the same amount is equivalent.
    cmd.forge_fuse()
        .args([
            "equivalence",
            "--code-a",
            &refund,
            "--code-b",
            &after_loop,
            "--sig",
            "pay(uint256)",
            "--promise-no-reent",
        ])
        .assert_success();

    // Even a call that sends nothing lets the unknown callee send ether back, as in hevm, but
    // plain execution can't replay that, so the check is incomplete.
    cmd.forge_fuse()
        .args([
            "equivalence",
            "--code-a",
            &keep_change,
            "--code-b",
            &keep_all,
            "--sig",
            "buy()",
            "--promise-no-reent",
            "--value",
            "10",
        ])
        .assert_failure();
    let out = cmd
        .forge_fuse()
        .args([
            "equivalence",
            "--code-a",
            &keep_change,
            "--code-b",
            &keep_all,
            "--sig",
            "buy()",
            "--promise-no-reent",
            "--value",
            "25",
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(out.contains("Not equivalent"), "{out}");
}

// hevm's equivalence cases: STATICCALL(gas, 0x1234, 0, size, 0, 0) with the size pushed by `size`,
// then revert with the call's return data.
fn static_call_then_revert(size: &str) -> String {
    format!("0x5f5f{size}5f6112345afa503d5f5f3e3d5ffd")
}

#[forgetest]
fn equivalence_treats_block_values_as_symbolic(cmd: _) {
    skip_unless_z3!("equivalence_treats_block_values_as_symbolic");
    // sstore(1, 0) vs sstore(1, number()), and the same with timestamp().
    for code_b in ["0x43600155", "0x42600155"] {
        cmd.forge_fuse()
            .args(["equivalence", "--code-a", "0x5f600155", "--code-b", code_b])
            .assert_failure();
    }
}

#[forgetest]
fn equivalence_abstracts_calls_into_unknown_code(cmd: _) {
    skip_unless_z3!("equivalence_abstracts_calls_into_unknown_code");
    let call = static_call_then_revert("5f");
    // The same call answers the same in both programs.
    cmd.args(["equivalence", "--code-a", &call, "--code-b", &format!("0x5b{}", &call[2..])])
        .assert_success();
    // A call with different calldata may answer differently, but plain execution can't replay
    // that answer, so the check is incomplete.
    cmd.forge_fuse()
        .args(["equivalence", "--code-a", &call, "--code-b", &static_call_then_revert("6001")])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: equivalence check incomplete (Error): the solver's counterexample [..] did not replay concretely; it answers 2 calls into unknown code, depends on GAS[..]

"#]]);
}

#[forgetest]
fn equivalence_reads_calldata_past_its_end_as_zero(cmd: _) {
    skip_unless_z3!("equivalence_reads_calldata_past_its_end_as_zero");
    // sstore(0, calldataload(2^256 - 1)) vs sstore(0, 0).
    let load = "0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff355f55";
    cmd.args(["equivalence", "--code-a", load, "--code-b", "0x5f5f55"]).assert_success();
}

#[forgetest]
fn equivalence_compares_modulo_gas_on_request(cmd: _) {
    skip_unless_z3!("equivalence_compares_modulo_gas_on_request");
    // mstore(x, mload(x)) for x = calldataload(0) runs out of gas for a large x, where stop
    // doesn't.
    let touch_memory = "0x5f3580519052600000";
    let out = cmd
        .args(["equivalence", "--code-a", touch_memory, "--code-b", "0x00"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(out.contains("code-a: ran out of gas"), "{out}");
    cmd.forge_fuse()
        .args(["equivalence", "--code-a", touch_memory, "--code-b", "0x00", "--ignore-out-of-gas"])
        .assert_success();
}

#[forgetest]
fn equivalence_treats_unknown_code_as_symbolic(cmd: _) {
    skip_unless_z3!("equivalence_treats_unknown_code_as_symbolic");
    // Returns `extcodesize(calldataload(0)) == 5`.
    let size_is_five = "0x5f353b6005145f5260205ff3";
    // Both codes read the same unknown account's size.
    cmd.args(["equivalence", "--code-a", size_is_five, "--code-b", "0x5b5f353b6005145f5260205ff3"])
        .assert_success();
    // Against returning zero: the size check above, the same with `extcodehash`, and the size
    // check of an address that equals one read before, 0xdead.
    for (code_a, code_b) in [
        (size_is_five, "0x60205ff3"),
        ("0x5f353f6005145f5260205ff3", "0x60205ff3"),
        (
            "0x61dead3b5061dead5f35146011575f5ffd5b5f353b6005145f5260205ff3",
            "0x61dead3b5061dead5f35146011575f5ffd5b60205ff3",
        ),
    ] {
        cmd.forge_fuse()
            .args(["equivalence", "--code-a", code_a, "--code-b", code_b])
            .assert_failure();
    }
}
