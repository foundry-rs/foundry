use super::symbolic_helpers::assert_relevant_lines;
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, str, util::OutputExt};

use super::symbolic_helpers::{assert_symbolic, z3_available};
use crate::skip_unless_z3;

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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkMapping(address,uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
args=[
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkNestedMapping(address,address,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkStoreLoad(bytes32,bytes32)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkMappingArray(address,uint256,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkPacked(uint128,uint128,bool,address)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkErc20Storage(address,address,uint256)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSvmStorageHelpers(bytes32,bytes32)
"#]],
    );
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

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkNativeGenericStorage(address)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkSvmArbitraryStorage(address)
"#]],
    );
    assert!(!stdout.contains("symbolic SLOAD key"), "{stdout}");
    assert!(!stdout.contains("symbolic Halmos compatibility cheatcode"), "{stdout}");
});

// Reading an unwritten mapping at a symbolic key must yield a fresh symbolic
// value, not a concrete zero. The assertion below claims that no caller is an
// admin; because nothing has ever written to `isAdmin`, the solver can satisfy
// `isAdmin[user] == true`. That candidate does not replay concretely from the
// default concrete storage value, so Forge must report Incomplete instead of a
// user-facing counterexample.
forgetest_init!(symbolic_sload_unwritten_mapping_default_layout, |prj, cmd| {
    skip_unless_z3!("symbolic_sload_unwritten_mapping_default_layout");

    prj.add_test(
        "SymbolicSLoadUnwrittenMapping.t.sol",
        r#"
contract SymbolicSLoadUnwrittenMapping {
    mapping(address => bool) isAdmin;

    function checkNoUserIsAdmin(address user) public view {
        assert(!isAdmin[user]);
    }
}
"#,
    );

    assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkNoUserIsAdmin",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/SymbolicSLoadUnwrittenMapping.t.sol:SymbolicSLoadUnwrittenMapping
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay] checkNoUserIsAdmin(address) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
