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

    // Even a call that sends nothing lets the unknown callee send ether back, as in hevm.
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
    // A call with different calldata may answer differently; the model is not replayable.
    let out = cmd
        .forge_fuse()
        .args(["equivalence", "--code-a", &call, "--code-b", &static_call_then_revert("6001")])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(out.contains("answers 2 calls into unknown code"), "{out}");
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
fn equivalence_relates_calldata_reads_at_symbolic_offsets(cmd: _) {
    skip_unless_z3!("equivalence_relates_calldata_reads_at_symbolic_offsets");
    // For x = calldataload(0) & 0xffff, calldataload(x + 1) is
    // (calldataload(x) << 8) | byte(0, calldataload(x + 32)) at every offset.
    let shifted = "0x600161ffff5f351601355f5500";
    let assembled = "0x61ffff5f351660208101355f1a903560081b175f5500";
    cmd.args(["equivalence", "--code-a", shifted, "--code-b", assembled]).assert_success();
    // calldataload(x + 2) differs.
    let out = cmd
        .forge_fuse()
        .args(["equivalence", "--code-a", shifted, "--code-b", "0x600261ffff5f351601355f5500"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(out.contains("storage slot 0x0: code-a "), "{out}");
}

#[forgetest]
fn equivalence_checks_whole_contracts_by_selector(prj: _, cmd: _) {
    skip_unless_z3!("equivalence_checks_whole_contracts_by_selector");
    prj.add_source(
        "Library.sol",
        r#"
contract Checked {
    function average(uint256 a, uint256 b) external pure returns (uint256) {
        return a / 2 + b / 2 + (a % 2 + b % 2) / 2;
    }
    function max3(uint256 a, uint256 b, uint256 c) external pure returns (uint256) {
        uint256 m = a > b ? a : b;
        return m > c ? m : c;
    }
    function isZero(uint256 a) external pure returns (bool) { return a == 0; }
}

contract Assembly {
    function average(uint256 a, uint256 b) external pure returns (uint256 r) {
        assembly { r := add(and(a, b), shr(1, xor(a, b))) }
    }
    function max3(uint256 a, uint256 b, uint256 c) external pure returns (uint256 r) {
        assembly {
            r := a
            if gt(b, r) { r := b }
            if gt(c, r) { r := c }
        }
    }
    function isZero(uint256 a) external pure returns (bool r) { assembly { r := iszero(a) } }
}

contract Wrong {
    function average(uint256 a, uint256 b) external pure returns (uint256 r) {
        assembly { r := add(and(a, b), shr(1, xor(a, b))) }
    }
    function max3(uint256 a, uint256 b, uint256 c) external pure returns (uint256 r) {
        assembly {
            r := a
            if gt(b, r) { r := b }
            if gt(r, c) { r := c }
        }
    }
    function isZero(uint256 a) external pure returns (bool r) { assembly { r := iszero(a) } }
}
"#,
    );
    let checked = deployed_bytecode(&mut cmd, "Checked");
    let assembly = deployed_bytecode(&mut cmd, "Assembly");
    let wrong = deployed_bytecode(&mut cmd, "Wrong");
    cmd.forge_fuse()
        .args(["equivalence", "--code-a", &checked, "--code-b", &assembly, "--empty-storage"])
        .assert_success();
    let out = cmd
        .forge_fuse()
        .args([
            "equivalence",
            "--code-a",
            &checked,
            "--code-b",
            &wrong,
            "--empty-storage",
            "--json",
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    let json: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(json["status"], "not_equivalent");
    assert_eq!(json["replayed"], true);
    // The counterexample calls max3.
    assert!(json["calldata"].as_str().unwrap().starts_with("0x"), "{json}");
}

#[forgetest]
fn equivalence_merges_forward_jumps_on_request(cmd: _) {
    skip_unless_z3!("equivalence_merges_forward_jumps_on_request");
    // sstore(0, iszero(iszero(calldataload(0)))) vs the same through a forward jump:
    // x := 0; if calldataload(0) { x := 1 }; sstore(0, x).
    let direct = "0x5f35151560005500";
    let branchy = "0x5f5f3515600a575060015b5f5500";
    // The same, with x := 2 in the branch.
    let wrong = "0x5f5f3515600a575060025b5f5500";
    for flags in [&[][..], &["--merge-states"][..]] {
        cmd.forge_fuse()
            .args(["equivalence", "--code-a", direct, "--code-b", branchy])
            .args(flags)
            .assert_success();
        cmd.forge_fuse()
            .args(["equivalence", "--code-a", direct, "--code-b", wrong])
            .args(flags)
            .assert_failure();
    }
}

#[forgetest]
fn equivalence_refines_keccak_models(prj: _, cmd: _) {
    skip_unless_z3!("equivalence_refines_keccak_models");
    prj.add_source(
        "Maps.sol",
        r#"
contract Plain {
    mapping(uint256 => uint256) internal values;
    function set(uint256 key, uint256 value) external { values[key] = value; }
    function get(uint256 key) external view returns (uint256) { return values[key]; }
}

contract Shifted {
    mapping(uint256 => uint256) internal values;
    function set(uint256 key, uint256 value) external { values[key + 1] = value; }
    function get(uint256 key) external view returns (uint256) { return values[key]; }
}
"#,
    );
    let plain = deployed_bytecode(&mut cmd, "Plain");
    let shifted = deployed_bytecode(&mut cmd, "Shifted");
    let out = cmd
        .forge_fuse()
        .args([
            "equivalence",
            "--code-a",
            &plain,
            "--code-b",
            &shifted,
            "--empty-storage",
            "--json",
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    let json: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(json["status"], "not_equivalent");
    assert_eq!(json["replayed"], true);
}

#[forgetest]
fn equivalence_forks_on_narrow_memory_offsets(cmd: _) {
    skip_unless_z3!("equivalence_forks_on_narrow_memory_offsets");
    // mstore8(0x80 + (calldataload(0) & 7), 1) and the same byte placed by a shifted mstore; the
    // offset takes eight values, so the check forks on each instead of reasoning symbolically.
    let byte_store = "0x600160075f35166080015360086080f3";
    let shifted_store = "0x6001600860075f35160260f8031b60805260086080f3";
    cmd.args(["equivalence", "--code-a", byte_store, "--code-b", shifted_store]).assert_success();
    // Shifting by one byte too few places it one position later.
    let out = cmd
        .forge_fuse()
        .args([
            "equivalence",
            "--code-a",
            byte_store,
            "--code-b",
            "0x6001600860075f35160260f0031b60805260086080f3",
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(out.contains("code-a: returned 0x0100000000000000"), "{out}");
}
