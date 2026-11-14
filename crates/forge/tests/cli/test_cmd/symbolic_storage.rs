use foundry_common::sh_eprintln;
use foundry_compilers::artifacts::BytecodeHash;
use foundry_config::{CompilationRestrictions, SettingsOverrides};
use foundry_test_utils::{
    forgetest_init, str,
    util::{OTHER_SOLC_VERSION, OutputExt},
};

use super::symbolic_helpers::{assert_symbolic, assert_symbolic_witness, z3_available};
use crate::skip_unless_z3;

#[forgetest_init]
fn symbolic_mapping_storage_finds_counterexample(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkMapping"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicMappingStorage.t.sol:SymbolicMappingStorage
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkMapping(address,uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
        .get_output()
        .stdout_lossy();

    assert!(!stdout.contains("symbolic SHA3"), "{stdout}");
    assert!(!stdout.contains("symbolic SSTORE key"), "{stdout}");
    assert!(!stdout.contains("symbolic SLOAD key"), "{stdout}");
}

#[forgetest_init]
fn symbolic_nested_mapping_storage_round_trips(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkNestedMapping",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicNestedMappingStorage.t.sol:SymbolicNestedMappingStorage
[PASS] checkNestedMapping(address,address,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic SHA3"), "{stdout}");
}

#[forgetest_init]
fn symbolic_vm_store_load_accepts_symbolic_slot(prj: _, cmd: _) {
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

    let stdout =
        assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkStoreLoad"]))
            .success()
            .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicVmStoreLoadSlot.t.sol:SymbolicVmStoreLoadSlot
[PASS] checkStoreLoad(bytes32,bytes32) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
            .get_output()
            .stdout_lossy();

    assert!(!stdout.contains("symbolic vm.store slot"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.load slot"), "{stdout}");
}

#[forgetest_init]
fn symbolic_mapping_dynamic_array_storage_round_trips(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkMappingArray",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicMappingDynamicArrayStorage.t.sol:SymbolicMappingDynamicArrayStorage
[PASS] checkMappingArray(address,uint256,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic SHA3"), "{stdout}");
    assert!(!stdout.contains("symbolic SSTORE key"), "{stdout}");
    assert!(!stdout.contains("symbolic SLOAD key"), "{stdout}");
}

#[forgetest_init]
fn symbolic_packed_storage_round_trips(prj: _, cmd: _) {
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

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkPacked"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicPackedStorage.t.sol:SymbolicPackedStorage
[PASS] checkPacked(uint128,uint128,bool,address) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_erc20_storage_paths_round_trip(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkErc20Storage",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicErc20Storage.t.sol:SymbolicErc20Storage
[PASS] checkErc20Storage(address,address,uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic SHA3"), "{stdout}");
    assert!(!stdout.contains("symbolic SSTORE key"), "{stdout}");
    assert!(!stdout.contains("symbolic SLOAD key"), "{stdout}");
}

#[forgetest_init]
fn symbolic_erc20_transfer_from_storage_paths_do_not_alias(prj: _, cmd: _) {
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
}

#[forgetest_init]
fn symbolic_svm_storage_helpers_are_supported(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkSvmStorageHelpers",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicSvmStorageHelpers.t.sol:SymbolicSvmStorageHelpers
[PASS] checkSvmStorageHelpers(bytes32,bytes32) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic Halmos compatibility cheatcode"), "{stdout}");
}

#[forgetest_init]
fn symbolic_generic_storage_exposes_arbitrary_uninitialized_reads(prj: _, cmd: _) {
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

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-contract",
        "SymbolicGenericStorage",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 2 tests for test/SymbolicGenericStorage.t.sol:SymbolicGenericStorage
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay] checkNativeGenericStorage(address) ([METRICS])
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay] checkSvmArbitraryStorage(address) ([METRICS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(!stdout.contains("symbolic SLOAD key"), "{stdout}");
    assert!(!stdout.contains("symbolic Halmos compatibility cheatcode"), "{stdout}");
}

// Reading an unwritten mapping at a symbolic key must yield a fresh symbolic
// value, not a concrete zero. The assertion below claims that no caller is an
// admin; because nothing has ever written to `isAdmin`, the solver can satisfy
// `isAdmin[user] == true`. That candidate does not replay concretely from the
// default concrete storage value, so Forge must report Incomplete instead of a
// user-facing counterexample. The same holds when the candidate's concrete
// replay stops at the `vm.assume` that the symbolic path satisfied. An entry that
// `setUp` wrote is known state, so a symbolic key that hits it reads the written
// value: a bounded burn proves safe and a real burn replays.
#[forgetest_init]
fn symbolic_sload_unwritten_mapping_default_layout(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_sload_unwritten_mapping_default_layout");

    prj.add_test(
        "SymbolicSLoadUnwrittenMapping.t.sol",
        r#"
interface Vm {
    function assume(bool condition) external pure;
}

contract Token {
    mapping(address => uint256) public balanceOf;

    constructor(address holder, uint256 amount) {
        balanceOf[holder] = amount;
    }

    function burn(address from, uint256 amount) external {
        require(balanceOf[from] >= amount);
        balanceOf[from] -= amount;
    }
}

contract SymbolicSLoadUnwrittenMapping {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    mapping(address => bool) isAdmin;
    Token token;

    function setUp() public {
        token = new Token(address(this), 100);
    }

    function checkNoUserIsAdmin(address user) public view {
        assert(!isAdmin[user]);
    }

    function checkAssumedAdminIsUnreachable(address user) public view {
        vm.assume(isAdmin[user]);
        assert(false);
    }

    // A read at a symbolic key that hits the entry `setUp` wrote returns the written balance.
    function checkBurnFromSetUpHolderIsBounded(address from, uint256 amount) public {
        (bool ok,) = address(token).call(abi.encodeCall(Token.burn, (from, amount)));
        if (ok && from == address(this)) assert(amount <= 100);
    }

    function checkSetUpHolderKeepsBalance(address from, uint256 amount) public {
        (bool ok,) = address(token).call(abi.encodeCall(Token.burn, (from, amount)));
        ok;
        assert(token.balanceOf(address(this)) == 100);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-contract",
        "SymbolicSLoadUnwrittenMapping",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 3 failing tests in test/SymbolicSLoadUnwrittenMapping.t.sol:SymbolicSLoadUnwrittenMapping
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay: rejected by `vm.assume`] checkAssumedAdminIsUnreachable(address) ([METRICS])
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay] checkNoUserIsAdmin(address) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkSetUpHolderKeepsBalance(address,uint256) ([METRICS])

Encountered a total of 3 failing tests, 1 tests succeeded

Tip: Run `forge test --rerun` to retry only the 3 failed tests

"#]]);
}

// An access at `keccak(key, slot) + index` with an unbounded `index` can reach any slot, here the
// owner at slot 0 and another mapping's slot, and a bounded `index` reaches the folded constant
// slots `keccak(7, 1) + 1` and `keccak(keccak(7, 1)) + 1` for both a concrete and a symbolic `key`.
// These must not be proven safe.
#[forgetest_init]
fn symbolic_storage_layout_offset_aliases_fixed_slot(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_storage_layout_offset_aliases_fixed_slot");

    prj.add_test(
        "SymbolicStorageLayoutAlias.t.sol",
        r#"
contract Registry {
    address public owner = address(0xBEEF);
    mapping(uint256 => uint256[]) internal lists;
    mapping(uint256 => uint256) public balances;

    function setAt(uint256 key, uint256 index, uint256 value) external {
        assembly {
            mstore(0, key)
            mstore(32, 1)
            sstore(add(keccak256(0, 64), index), value)
        }
    }

    function getAt(uint256 key, uint256 index) external view returns (uint256 value) {
        assembly {
            mstore(0, key)
            mstore(32, 1)
            value := sload(add(keccak256(0, 64), index))
        }
    }

    function setListAt(uint256 key, uint256 index, uint256 value) external {
        assembly {
            mstore(0, key)
            mstore(32, 1)
            mstore(0, keccak256(0, 64))
            sstore(add(keccak256(0, 32), index), value)
        }
    }

    function getListAt(uint256 key, uint256 index) external view returns (uint256 value) {
        assembly {
            mstore(0, key)
            mstore(32, 1)
            mstore(0, keccak256(0, 64))
            value := sload(add(keccak256(0, 32), index))
        }
    }
}

contract SymbolicStorageLayoutAlias {
    Registry registry;

    function setUp() public {
        registry = new Registry();
    }

    function checkConstantKeyIndexCannotReachOwner(uint256 index, uint256 value) public {
        registry.setAt(7, index, value);
        assert(registry.owner() == address(0xBEEF));
    }

    function checkSymbolicKeyIndexCannotReachOwner(uint256 key, uint256 index, uint256 value)
        public
    {
        registry.setAt(key, index, value);
        assert(registry.owner() == address(0xBEEF));
    }

    function checkIndexCannotReachOtherMapping(uint256 index, uint256 value) public {
        registry.setAt(7, index, value);
        assert(registry.balances(7) == 0);
    }

    function checkSymbolicKeyBoundedIndexCannotReachOwner(
        uint256 key,
        uint256 index,
        uint256 value
    ) public {
        require(index < 3);
        registry.setAt(key, index, value);
        assert(registry.owner() == address(0xBEEF));
    }

    function checkBoundedIndexCannotReachNextSlot(uint256 index, uint256 value) public {
        require(index < 3);
        registry.setAt(7, index, value);
        assert(registry.getAt(7, 1) == 0);
    }

    function checkSymbolicKeyBoundedIndexCannotReachNextSlot(
        uint256 key,
        uint256 index,
        uint256 value
    ) public {
        require(index < 3);
        registry.setAt(key, index, value);
        assert(registry.getAt(7, 1) == 0);
    }

    function checkSymbolicKeyBoundedIndexCannotReachListElement(
        uint256 key,
        uint256 index,
        uint256 value
    ) public {
        require(index < 3);
        registry.setListAt(key, index, value);
        assert(registry.getListAt(7, 1) == 0);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-contract",
        "SymbolicStorageLayoutAlias",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 7 tests for test/SymbolicStorageLayoutAlias.t.sol:SymbolicStorageLayoutAlias
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkBoundedIndexCannotReachNextSlot(uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkConstantKeyIndexCannotReachOwner(uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkIndexCannotReachOtherMapping(uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkSymbolicKeyBoundedIndexCannotReachListElement(uint256,uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkSymbolicKeyBoundedIndexCannotReachNextSlot(uint256,uint256,uint256) ([METRICS])
[PASS] checkSymbolicKeyBoundedIndexCannotReachOwner(uint256,uint256,uint256) ([METRICS])
[FAIL: incomplete symbolic execution (Error): solver error: solver model does not satisfy path constraints involving symbolic Keccak heuristic] checkSymbolicKeyIndexCannotReachOwner(uint256,uint256,uint256) ([METRICS])
Suite result: FAILED. 1 passed; 6 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

// A symbolic Keccak compared with a hash computed during the run, or with a symbolic Keccak over a
// different input length, is decided by its input instead of a free alias. The optimizer folds the
// slots of `balances[KEY]` and `nested[KEY][1]` into constants. The storage layout and source
// literals give them their preimages, so a write through a symbolic key still reaches them, also
// when a proxy delegates to the contract whose code holds the folded slot.
#[forgetest_init]
fn symbolic_keccak_known_preimage_bases_do_not_alias(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_keccak_known_preimage_bases_do_not_alias");

    prj.update_config(|config| config.optimizer = Some(true));
    prj.add_test(
        "SymbolicKeccakPreimage.t.sol",
        r#"
contract SymbolicKeccakPreimage {
    address constant KEY = address(0xD00D);

    mapping(address => uint256) balances;
    address[] holders;
    mapping(address => uint256[]) deposits;
    mapping(string => uint256) names;
    FoldedNested proxy;

    constructor() {
        holders.push(address(1));
        holders.push(address(2));
        proxy = FoldedNested(address(new DelegatingProxy(address(new FoldedNested()))));
    }

    function checkArrayElementSurvivesMappingWrite(address who, uint256 amount, uint256 i) public {
        require(i < holders.length);
        address before = holders[i];
        balances[who] = amount;
        assert(holders[i] == before);
    }

    function checkNestedArraySurvivesMappingWrite(
        address user,
        address who,
        uint256 value,
        uint256 amount
    ) public {
        deposits[user].push(value);
        uint256 before = deposits[user][0];
        balances[who] = amount;
        assert(deposits[user][0] == before);
    }

    function checkKnownKeccakPreimage(uint256 x) public pure {
        assert(keccak256(abi.encode(x)) != keccak256(abi.encode(uint256(42))));
    }

    // A common offset on two hashes, as in `keccak(key, slot) + 1`, keeps them distinct.
    function checkKeccakPlusOffsetDiffers(uint256 a, uint256 b) public pure {
        require(a != b);
        assert(uint256(keccak256(abi.encode(a))) + 1 != uint256(keccak256(abi.encode(b))) + 1);
    }

    function checkKeccakPlusOffsetCanCollide(uint256 a, uint256 b) public pure {
        assert(uint256(keccak256(abi.encode(a))) + 1 != uint256(keccak256(abi.encode(b))) + 1);
    }

    // The pinned key folds to a constant slot that must still alias the symbolic-key write.
    function checkPinnedKeyAliasesSymbolicWrite(address who) public {
        require(who == address(4));
        balances[who] = 39;
        assert(balances[who] != 39);
    }

    // The literal key hashes a 34-byte preimage that must still alias the symbolic-key write.
    /// forge-config: default.symbolic.default_bytes_lengths = [2]
    function checkStringKeyAliasesLiteralRead(string memory k) public {
        names[k] = 5;
        assert(names["ab"] != 5);
    }

    function checkFoldedConstantKeySlotAliases(address who) public {
        balances[who] = 5;
        assert(balances[KEY] == 0);
    }

    function checkFoldedNestedConstantKeySlotAliases(address who, uint256 id) public {
        proxy.write(who, id);
        assert(proxy.readFolded() == 0);
    }
}

contract FoldedNested {
    address constant KEY = address(0xD00D);

    mapping(address => mapping(uint256 => uint256)) nested;

    function write(address who, uint256 id) external {
        nested[who][id] = 3;
    }

    function readFolded() external view returns (uint256) {
        return nested[KEY][1];
    }
}

contract DelegatingProxy {
    address immutable implementation;

    constructor(address implementation_) {
        implementation = implementation_;
    }

    fallback() external {
        address target = implementation;
        assembly {
            calldatacopy(0, 0, calldatasize())
            let ok := delegatecall(gas(), target, 0, calldatasize(), 0, 0)
            returndatacopy(0, 0, returndatasize())
            if iszero(ok) { revert(0, returndatasize()) }
            return(0, returndatasize())
        }
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-contract", "SymbolicKeccakPreimage"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 9 tests for test/SymbolicKeccakPreimage.t.sol:SymbolicKeccakPreimage
[PASS] checkArrayElementSurvivesMappingWrite(address,uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] calldata=0x622f5688000000000000000000000000000000000000000000000000000000000000d00d args=[0x000000000000000000000000000000000000D00d]] checkFoldedConstantKeySlotAliases(address) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] calldata=[..] args=[0x000000000000000000000000000000000000D00d, 1]] checkFoldedNestedConstantKeySlotAliases(address,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [..]] checkKeccakPlusOffsetCanCollide(uint256,uint256) ([METRICS])
[PASS] checkKeccakPlusOffsetDiffers(uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] calldata=0x4df8b26f000000000000000000000000000000000000000000000000000000000000002a args=[42]] checkKnownKeccakPreimage(uint256) ([METRICS])
[PASS] checkNestedArraySurvivesMappingWrite(address,address,uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] calldata=[..] args=[0x0000000000000000000000000000000000000004]] checkPinnedKeyAliasesSymbolicWrite(address) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] calldata=[..] args=["ab"]] checkStringKeyAliasesLiteralRead(string) ([METRICS])
Suite result: FAILED. 3 passed; 6 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

// A build without storage layouts after an edit keeps the cached symbolic profile, so the next
// symbolic run must recompile the artifacts it wrote without layouts.
#[forgetest_init]
fn symbolic_recompiles_cached_artifacts_without_storage_layouts(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_recompiles_cached_artifacts_without_storage_layouts");

    prj.update_config(|config| {
        config.optimizer = Some(true);
        config.ast = true;
    });
    // A source without contracts has an artifact without a layout, which must stay cached.
    prj.add_source("Helpers.sol", "function five() pure returns (uint256) { return 5; }");
    let source = r#"
import {five} from "../src/Helpers.sol";

contract FoldedStore {
    mapping(uint256 => uint256) public values;

    function set(uint256 key, uint256 value) external {
        values[key] = value;
    }

    function readFive() external view returns (uint256) {
        return values[five()];
    }
}

contract SymbolicStaleLayout {
    FoldedStore store = new FoldedStore();

    function checkFoldedSlotAliases(uint256 key) public {
        store.set(key, 1);
        assert(store.readFive() == 0);
    }
}
"#;
    prj.add_test("SymbolicStaleLayout.t.sol", source);
    let args = ["test", "--symbolic", "--match-contract", "SymbolicStaleLayout"];
    assert_symbolic_witness(cmd.args(args)).failure().stdout_eq(str![[r#"
...
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkFoldedSlotAliases(uint256) ([METRICS])
...
"#]]);

    prj.add_test("SymbolicStaleLayout.t.sol", &format!("{source}// Edited.\n"));
    cmd.forge_fuse().arg("build").assert_success();

    assert_symbolic_witness(cmd.forge_fuse().args(args)).failure().stdout_eq(str![[r#"
...
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkFoldedSlotAliases(uint256) ([METRICS])
...
"#]]);

    // The recompiled artifacts have layouts, so the next run reuses them.
    assert_symbolic_witness(cmd.forge_fuse().args(args)).failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/SymbolicStaleLayout.t.sol:SymbolicStaleLayout
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkFoldedSlotAliases(uint256) ([METRICS])
...
"#]]);
}

// A filtered symbolic run must only recompile the artifacts of the compiler profiles it uses, or
// the cache keeps pointing at deleted artifacts of other profiles and every run recompiles.
#[forgetest_init]
fn symbolic_keeps_cached_artifacts_of_other_profiles(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_keeps_cached_artifacts_of_other_profiles");

    prj.update_config(|config| {
        config.ast = true;
        config.additional_compiler_profiles = vec![SettingsOverrides {
            name: "no-metadata".to_string(),
            via_ir: None,
            evm_version: None,
            optimizer: None,
            optimizer_runs: None,
            bytecode_hash: Some(BytecodeHash::None),
        }];
        config.compilation_restrictions = vec![CompilationRestrictions {
            paths: "test/NoMetadata.t.sol".parse().unwrap(),
            version: None,
            via_ir: None,
            bytecode_hash: Some(BytecodeHash::None),
            min_optimizer_runs: None,
            optimizer_runs: None,
            max_optimizer_runs: None,
            min_evm_version: None,
            evm_version: None,
            max_evm_version: None,
        }];
    });
    let source = "contract Shared { uint256 public value; }\n";
    prj.add_source("Shared.sol", source);
    prj.add_test(
        "Default.t.sol",
        r#"
import {Shared} from "../src/Shared.sol";

contract SymbolicDefaultProfile {
    function checkShared() public {
        assert(new Shared().value() == 0);
    }
}
"#,
    );
    prj.add_test(
        "NoMetadata.t.sol",
        r#"
import {Shared} from "../src/Shared.sol";

contract NoMetadataProfile {
    Shared shared = new Shared();
}
"#,
    );
    cmd.args(["test", "--symbolic"]).assert_success();

    prj.add_source("Shared.sol", &format!("{source}// Edited.\n"));
    cmd.forge_fuse().arg("build").assert_success();

    let args = ["test", "--symbolic", "--match-path", "test/Default.t.sol"];
    cmd.forge_fuse().args(args).assert_success().stderr_eq(str![[r#"
Recompiling cached artifacts without storage layouts...

"#]]);
    cmd.forge_fuse().args(args).assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped
...
"#]]);
}

// When filtered roots hit solc error 2449, the retry compiles every root and can select another
// solc version, so the stale artifacts of that compilation must be recompiled.
#[forgetest_init]
fn symbolic_recompiles_cached_artifacts_after_source_order_fallback(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_recompiles_cached_artifacts_after_source_order_fallback");

    prj.update_config(|config| {
        config.solc = None;
        config.optimizer = Some(true);
        config.ast = true;
    });
    let source = r#"
pragma solidity ^0.8.24;

function five() pure returns (uint256) {
    return 5;
}

contract FoldedStore {
    mapping(uint256 => uint256) public values;

    function set(uint256 key, uint256 value) external {
        values[key] = value;
    }

    function readFive() external view returns (uint256) {
        return values[five()];
    }
}
"#;
    prj.add_source("FoldedStore.sol", source);
    prj.add_test(
        "SymbolicFallback.t.sol",
        r#"
pragma solidity ^0.8.24;
import {FoldedStore} from "../src/FoldedStore.sol";

contract SymbolicFallback {
    FoldedStore store = new FoldedStore();

    function checkFoldedSlotAliases(uint256 key) public {
        store.set(key, 1);
        assert(store.readFive() == 0);
    }
}
"#,
    );
    prj.add_test(
        "helpers/Base.sol",
        r#"
pragma solidity ^0.8.24;
import "./Derived.sol";
contract Base {}
"#,
    );
    prj.add_test(
        "helpers/Derived.sol",
        r#"
pragma solidity ^0.8.24;
import "./Base.sol";
contract Derived is Base {}
"#,
    );
    // Without this root, solc visits the import cycle in the order that produces error 2449. It
    // also pins the shared sources to another solc version in the retry.
    prj.add_test(
        "Anchor.t.sol",
        &format!(
            r#"
pragma solidity ={OTHER_SOLC_VERSION};
import "./helpers/Derived.sol";
import {{SymbolicFallback}} from "./SymbolicFallback.t.sol";
contract Anchor {{}}
"#
        ),
    );
    let expected = str![[r#"
...
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkFoldedSlotAliases(uint256) ([METRICS])
...
"#]];
    assert_symbolic_witness(cmd.args(["test", "--symbolic"])).failure().stdout_eq(expected.clone());

    prj.add_source("FoldedStore.sol", &format!("{source}// Edited.\n"));
    cmd.forge_fuse().arg("build").assert_success();

    let args = ["test", "--symbolic", "--match-path", "test/SymbolicFallback.t.sol"];
    assert_symbolic_witness(cmd.forge_fuse().args(args)).failure().stdout_eq(expected);
}
