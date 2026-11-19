use foundry_common::sh_eprintln;
use foundry_compilers::artifacts::EvmVersion;
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
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkNativeGenericStorage(address) ([METRICS])
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

// Each solver model takes the `assert(false)` branch with a nonzero unwritten balance, but the
// concrete replay reads zero and runs out of gas in a loop, at the SSTORE sentry, or while
// expanding memory. The engine does not model gas, so those halts do not confirm the model and
// the run must report incomplete. A replay that failed a soft assertion before running out of
// gas still confirms it, and so does a modeled memory-limit halt that replays as out of gas at the
// same instruction, but not one whose replay runs out of gas elsewhere or within the limit.
#[forgetest_init]
fn symbolic_replay_out_of_gas_is_incomplete(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_replay_out_of_gas_is_incomplete");

    prj.add_test(
        "SymbolicReplayOutOfGas.t.sol",
        r#"
contract SymbolicReplayOutOfGas {
    mapping(address => uint256) balanceOf;

    function checkReplayOutOfGas(address owner) public view {
        if (balanceOf[owner] == 0) {
            uint256 acc;
            for (uint256 i; i < 5000; i++) {
                acc += i;
            }
            require(acc != 1);
            return;
        }
        assert(false);
    }

    function checkReplayOutOfGasBeforeMemoryHalt(address owner) public view {
        if (balanceOf[owner] == 0) {
            uint256 acc;
            for (uint256 i; i < 5000; i++) {
                acc += i;
            }
            require(acc != 1);
            return;
        }
        assembly {
            mstore(0x10000000000, 1)
        }
    }

    function checkReplayOutOfGasAtMemoryHalt(address owner) public view {
        uint256 offset = balanceOf[owner] == 0 ? 0x100000 : 0x10000000000;
        assembly {
            mstore(offset, 1)
        }
    }
}
"#,
    );

    prj.add_test(
        "SymbolicReplayGasHalts.t.sol",
        r#"
interface Vm {
    function assertTrue(bool condition) external pure;
}

contract SymbolicReplayGasHalts {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    mapping(address => uint256) balanceOf;
    uint256 counter;

    function checkReplaySstoreOutOfGas(address owner) public {
        if (balanceOf[owner] == 0) {
            for (uint256 i; i < 5000; i++) {
                counter = i + 1;
            }
            return;
        }
        assert(false);
    }

    function checkReplayMemoryOutOfGas(address owner) public view {
        if (balanceOf[owner] == 0) {
            assembly {
                mstore(0x100000, 1)
            }
            return;
        }
        assert(false);
    }

    /// forge-config: default.assertions_revert = false
    function checkSoftAssertThenOutOfGas(uint256 x) public pure {
        vm.assertTrue(x != 7);
        assembly {
            mstore(0x100000, 1)
        }
    }
}
"#,
    );

    assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-contract",
        "SymbolicReplayOutOfGas",
        "--gas-limit",
        "300000",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 3 tests for test/SymbolicReplayOutOfGas.t.sol:SymbolicReplayOutOfGas
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay: ran out of gas] checkReplayOutOfGas(address) ([METRICS])
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay: ran out of gas] checkReplayOutOfGasAtMemoryHalt(address) ([METRICS])
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay: ran out of gas] checkReplayOutOfGasBeforeMemoryHalt(address) ([METRICS])
Suite result: FAILED. 0 passed; 3 failed; 0 skipped; [ELAPSED]
...
"#]]);

    assert_symbolic(cmd.forge_fuse().args([
        "test",
        "--symbolic",
        "--match-contract",
        "SymbolicReplayGasHalts",
        "--gas-limit",
        "300000",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 3 tests for test/SymbolicReplayGasHalts.t.sol:SymbolicReplayGasHalts
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay: ran out of gas] checkReplayMemoryOutOfGas(address) ([METRICS])
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay: ran out of gas] checkReplaySstoreOutOfGas(address) ([METRICS])
[FAIL: EvmError: MemoryOOG; counterexample: 		[SENDER] [SENDER] calldata=[..] args=[7]] checkSoftAssertThenOutOfGas(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 3 failed; 0 skipped; [ELAPSED]
...
"#]]);

    // Concrete execution charges copy gas before it checks `memory_limit`, so the modeled
    // memory-limit halt replays as out of gas and still confirms and minimizes the model.
    prj.add_test(
        "SymbolicReplayMemoryLimit.t.sol",
        r#"
contract SymbolicReplayMemoryLimit {
    function checkNewBytes(uint256 n) public pure {
        bytes memory b = new bytes(n);
        assert(b.length == n);
    }
}
"#,
    );

    assert_symbolic(cmd.forge_fuse().args([
        "test",
        "--symbolic",
        "--match-contract",
        "SymbolicReplayMemoryLimit",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicReplayMemoryLimit.t.sol:SymbolicReplayMemoryLimit
[FAIL: EvmError: OutOfGas; counterexample: 		[SENDER] [SENDER] calldata=[..] args=[11453013985 [1.145e10]]] checkNewBytes(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

// A constant slot whose top 128 bits are all zeros or all ones, read in assembly, is a fixed slot,
// not a folded nested mapping entry, so it is not searched against the layout's nested mappings.
// The 256 branches add enough candidate keys that the search would exhaust its budget. The default
// profile is unoptimized, so solc pushes the owner slot whole instead of as `not(0x8b78c6d8)`.
#[forgetest_init]
fn symbolic_non_hash_constant_slot_skips_folded_mapping_search(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_non_hash_constant_slot_skips_folded_mapping_search");

    let branches: String =
        (0..256).map(|i| format!("        if (x == {}) return {i};\n", 1000 + i)).collect();
    prj.add_test(
        "SymbolicNonHashConstantSlot.t.sol",
        &format!(
            r#"
contract ConstantSlotCounter {{
    struct Buckets {{
        mapping(uint256 => uint256) a;
        mapping(uint256 => uint256) b;
        mapping(uint256 => uint256) c;
        mapping(uint256 => uint256) d;
    }}

    mapping(uint256 => Buckets) buckets;

    function add(uint256 amount) external {{
        assembly {{
            sstore(0x05345cdf77eb68f44c, add(sload(0x05345cdf77eb68f44c), amount))
        }}
    }}

    function total() external view returns (uint256 value) {{
        assembly {{
            value := sload(0x05345cdf77eb68f44c)
        }}
    }}

    function setOwner(address owner) external {{
        assembly {{
            sstore(0xffffffffffffffffffffffffffffffffffffffffffffffffffffffff74873927, owner)
        }}
    }}

    function getOwner() external view returns (address owner) {{
        assembly {{
            owner := sload(0xffffffffffffffffffffffffffffffffffffffffffffffffffffffff74873927)
        }}
    }}

    function index(uint256 x) external pure returns (uint256) {{
{branches}        return type(uint256).max;
    }}
}}

contract SymbolicNonHashConstantSlot {{
    ConstantSlotCounter counter = new ConstantSlotCounter();

    function checkAddSetsTotal(uint256 amount) public {{
        counter.add(amount);
        assert(counter.total() == amount);
    }}

    function checkAddChangesTotal(uint256 amount) public {{
        counter.add(amount);
        assert(counter.total() != amount);
    }}

    function checkSetOwnerSetsOwner(address owner) public {{
        counter.setOwner(owner);
        assert(counter.getOwner() == owner);
    }}
}}
"#
        ),
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-contract",
        "SymbolicNonHashConstantSlot",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 3 tests for test/SymbolicNonHashConstantSlot.t.sol:SymbolicNonHashConstantSlot
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkAddChangesTotal(uint256) ([METRICS])
[PASS] checkAddSetsTotal(uint256) ([METRICS])
[PASS] checkSetOwnerSetsOwner(address) ([METRICS])
Suite result: FAILED. 2 passed; 1 failed; 0 skipped; [ELAPSED]
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
// the cache keeps pointing at deleted artifacts of other profiles and every run recompiles. The
// kept artifact of the other profile has no layout, so it must take the default profile's layout.
#[forgetest_init]
fn symbolic_keeps_cached_artifacts_of_other_profiles(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_keeps_cached_artifacts_of_other_profiles");

    prj.update_config(|config| {
        config.optimizer = Some(true);
        config.ast = true;
        config.additional_compiler_profiles = vec![SettingsOverrides {
            name: "cancun".to_string(),
            via_ir: None,
            evm_version: Some(EvmVersion::Cancun),
            optimizer: None,
            optimizer_runs: None,
            bytecode_hash: None,
        }];
        config.compilation_restrictions = vec![CompilationRestrictions {
            paths: "test/Cancun.t.sol".parse().unwrap(),
            version: None,
            via_ir: None,
            bytecode_hash: None,
            min_optimizer_runs: None,
            optimizer_runs: None,
            max_optimizer_runs: None,
            min_evm_version: None,
            evm_version: None,
            max_evm_version: Some(EvmVersion::Cancun),
        }];
    });
    let source = r#"
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
        "Default.t.sol",
        r#"
import {FoldedStore} from "../src/FoldedStore.sol";

interface Vm {
    function deployCode(string calldata artifactPath) external returns (address);
}

contract SymbolicDefaultProfile {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    FoldedStore store = FoldedStore(vm.deployCode("FoldedStore.sol:FoldedStore:cancun"));

    function checkFoldedSlotAliases(uint256 key) public {
        store.set(key, 1);
        assert(store.readFive() == 0);
    }
}
"#,
    );
    prj.add_test(
        "Cancun.t.sol",
        r#"
import {FoldedStore} from "../src/FoldedStore.sol";

contract CancunProfile {
    FoldedStore store = new FoldedStore();
}
"#,
    );
    let failure = str![[r#"
...
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkFoldedSlotAliases(uint256) ([METRICS])
...
"#]];
    assert_symbolic_witness(cmd.args(["test", "--symbolic"])).failure().stdout_eq(failure.clone());

    prj.add_source("FoldedStore.sol", &format!("{source}// Edited.\n"));
    cmd.forge_fuse().arg("build").assert_success();

    let args = ["test", "--symbolic", "--match-path", "test/Default.t.sol"];
    assert_symbolic_witness(cmd.forge_fuse().args(args)).failure().stdout_eq(failure).stderr_eq(
        str![[r#"
Recompiling cached artifacts without storage layouts...
...
"#]],
    );
    assert_symbolic_witness(cmd.forge_fuse().args(args)).failure().stdout_eq(str![[r#"
No files changed, compilation skipped
...
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkFoldedSlotAliases(uint256) ([METRICS])
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

// A counterexample that depends on arbitrary storage, including a mapping slot keyed by calldata,
// replays only when replay applies the solver model's storage instead of fresh arbitrary values.
// Replay draws the modeled values only where arbitrary storage would draw a fresh one, so slots
// that setUp fixed, nonzero slots read with `vm.load`, and storage that the test itself makes
// arbitrary keep their values, and the first read of a copy still writes back to its source.
#[forgetest_init]
fn symbolic_single_call_replays_arbitrary_storage(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_single_call_replays_arbitrary_storage");

    prj.add_test(
        "SymbolicStorageReplay.t.sol",
        r#"
interface Vm {
    function assertGe(uint256 left, uint256 right) external pure;
    function assume(bool condition) external pure;
    function setArbitraryStorage(address target, bool overwrite) external;
    function store(address target, bytes32 slot, bytes32 value) external;
    function load(address target, bytes32 slot) external view returns (bytes32);
    function copyStorage(address from, address to) external;
}

interface Svm {
    function enableSymbolicStorage(address target) external;
}

contract Token {
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;

    function mint(address to, uint256 amount) external {
        totalSupply += amount;
        unchecked {
            balanceOf[to] += amount;
        }
    }
}

contract Box {
    uint256 public val;
    mapping(uint256 => uint256) public values;

    function set(uint256 value) external {
        val = value;
    }
}

contract SymbolicStorageReplay {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    Svm constant svm = Svm(0xF3993A62377BCd56AE39D773740A5390411E8BC9);
    Token token;
    Token concreteToken;
    Token pinnedToken;
    Box readBox;
    uint256 seen;
    Box loadBox;
    Box copySource;
    Box copyTarget;

    function setUp() public {
        token = new Token();
        concreteToken = new Token();
        pinnedToken = new Token();
        vm.setArbitraryStorage(address(token), true);
        vm.setArbitraryStorage(address(pinnedToken), true);
        vm.store(address(pinnedToken), bytes32(0), bytes32(uint256(5)));

        readBox = new Box();
        vm.setArbitraryStorage(address(readBox), true);
        seen = readBox.val();

        loadBox = new Box();
        loadBox.set(5);
        vm.setArbitraryStorage(address(loadBox), true);

        copySource = new Box();
        copyTarget = new Box();
        vm.setArbitraryStorage(address(copySource), true);
        vm.copyStorage(address(copySource), address(copyTarget));
    }

    function checkMint(uint256 amount, address account) public {
        uint256 balanceBefore = token.balanceOf(account);
        token.mint(account, amount);
        vm.assertGe(token.balanceOf(account), balanceBefore);
    }

    function checkMintNonZero(uint256 amount, address account) public {
        vm.assume(amount != 0);
        uint256 balanceBefore = token.balanceOf(account);
        token.mint(account, amount);
        vm.assertGe(token.balanceOf(account), balanceBefore);
    }

    // Arbitrary storage keeps a value that setUp already read, so the assertion cannot fail.
    function checkSetUpRead() public view {
        assert(readBox.val() == seen);
    }

    // `vm.load` keeps a nonzero slot of arbitrary storage, so the assertion cannot fail.
    function checkLoad() public view {
        assert(uint256(vm.load(address(loadBox), bytes32(0))) == 5);
    }

    // Both reads use one slot, so they cannot take the values the assertion needs.
    function checkAliased(uint256 a, uint256 b) public view {
        uint256 x = readBox.values(a);
        uint256 y = readBox.values(b);
        vm.assume(a == b && a < 10);
        if (x == 42 && y == 99) assert(false);
        if (x == 42 || y == 99) revert("ordinary revert");
    }

    // The first read of the copy writes its value back to the source.
    function checkCopyWriteBack() public {
        copySource.set(5);
        uint256 value = copyTarget.val();
        if (value != 5) assert(copySource.val() != value);
    }

    // Arbitrary storage keeps a slot written with `vm.store`, so the assertion cannot fail.
    function checkPinnedSlot() public view {
        assert(pinnedToken.totalSupply() == 5);
    }

    // The storage was concrete when the call started, so the first assertion cannot fail.
    function checkStorageMadeArbitraryByTest() public {
        assert(concreteToken.totalSupply() == 0);
        svm.enableSymbolicStorage(address(concreteToken));
        if (concreteToken.totalSupply() == 7) assert(false);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic"])).failure().stdout_eq(str![[r#"
...
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay] checkAliased(uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkCopyWriteBack() ([METRICS])
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay] checkLoad() ([METRICS])
[FAIL: assertion failed: [..]; counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkMint(uint256,address) ([METRICS])
[FAIL: assertion failed: [..]; counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkMintNonZero(uint256,address) ([METRICS])
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay] checkPinnedSlot() ([METRICS])
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay] checkSetUpRead() ([METRICS])
[FAIL: incomplete symbolic execution (Error): symbolic counterexample did not replay] checkStorageMadeArbitraryByTest() ([METRICS])
...
"#]]);

    let artifact_dir =
        prj.root().join("cache/symbolic/test_SymbolicStorageReplay_t_sol_SymbolicStorageReplay");
    let mut artifacts = std::fs::read_dir(artifact_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    artifacts.sort();
    assert_eq!(artifacts.len(), 3, "{artifacts:?}");
    for artifact in artifacts {
        assert_symbolic_witness(cmd.forge_fuse().args([
            "test",
            "--replay-symbolic-artifact",
            artifact.to_str().unwrap(),
        ]))
        .failure()
        .stdout_eq(str![[r#"
...
[FAIL: [..]assertion failed[..]; counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] check[..] ([GAS])
...
"#]]);
    }
}

// Two members of a struct in a mapping, `keccak(key, slot)` and `keccak(key', slot) + 1`, do not
// alias when `key'` is the same value as `key` in another expression, here a masked copy.
#[forgetest_init]
fn symbolic_struct_member_mapping_roots_do_not_alias(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_struct_member_mapping_roots_do_not_alias");

    prj.add_test(
        "SymbolicStructMemberRoots.t.sol",
        r#"
struct Pair {
    mapping(uint256 => uint256) first;
    mapping(uint256 => uint256) second;
}

contract Store {
    mapping(uint256 => Pair) pairs;

    function setSecond(uint256 key, uint256 id, uint256 amount) external {
        pairs[key].second[id] = amount;
    }

    function getSecond(uint256 key, uint256 id) external view returns (uint256) {
        return pairs[key].second[id];
    }

    function setMaskedFirst(uint256 key, uint256 id, uint256 amount) external {
        pairs[uint160(key)].first[id] = amount;
    }

    function setMaskedSecond(uint256 key, uint256 id, uint256 amount) external {
        pairs[uint160(key)].second[id] = amount;
    }
}

contract SymbolicStructMemberRoots {
    Store store = new Store();

    function checkOtherMemberKeepsValue(uint256 key, uint256 id, uint256 a, uint256 b) public {
        require(key <= type(uint160).max);
        store.setSecond(key, id, a);
        store.setMaskedFirst(key, id, b);
        assert(store.getSecond(key, id) == a);
    }

    function checkSameMemberOverwritesValue(uint256 key, uint256 id, uint256 a, uint256 b) public {
        require(key <= type(uint160).max);
        store.setSecond(key, id, a);
        store.setMaskedSecond(key, id, b);
        assert(store.getSecond(key, id) == a);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-contract", "SymbolicStructMemberRoots"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 2 tests for test/SymbolicStructMemberRoots.t.sol:SymbolicStructMemberRoots
[PASS] checkOtherMemberKeepsValue(uint256,uint256,uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [..]] checkSameMemberOverwritesValue(uint256,uint256,uint256,uint256) ([METRICS])
Suite result: FAILED. 1 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

// A nested entry `keccak(id, keccak(key, root) + 1)` does not alias a top-level entry
// `keccak(other, root)` when `id == other`, for a root at slot 0 and at a fixed namespaced slot.
#[forgetest_init]
fn symbolic_nested_mapping_entry_does_not_alias_fixed_root_entry(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_nested_mapping_entry_does_not_alias_fixed_root_entry");

    prj.add_test(
        "SymbolicNestedFixedRoot.t.sol",
        r#"
struct Node {
    uint32 tag;
    mapping(uint256 => uint256) inner;
}

contract SlotZeroStore {
    mapping(uint256 => Node) outer;

    function setTag(uint256 key, uint32 tag) external {
        outer[key].tag = tag;
    }

    function setInner(uint256 key, uint256 id, uint256 value) external {
        outer[key].inner[id] = value;
    }

    function getInner(uint256 key, uint256 id) external view returns (uint256) {
        return outer[key].inner[id];
    }
}

contract NamespacedStore {
    struct Layout {
        mapping(uint256 => Node) outer;
    }

    function layout() internal pure returns (Layout storage l) {
        assembly {
            l.slot := 0x183a6125c38840424c4a85fa12bab2ab606c4b6d0e7cc73c0c06ba5300eab514
        }
    }

    function setTag(uint256 key, uint32 tag) external {
        layout().outer[key].tag = tag;
    }

    function setInner(uint256 key, uint256 id, uint256 value) external {
        layout().outer[key].inner[id] = value;
    }

    function getInner(uint256 key, uint256 id) external view returns (uint256) {
        return layout().outer[key].inner[id];
    }
}

contract SymbolicNestedFixedRoot {
    SlotZeroStore slotZero = new SlotZeroStore();
    NamespacedStore namespaced = new NamespacedStore();

    function checkSlotZeroTagKeepsInner(uint256 key, uint256 id, uint256 other, uint256 value, uint32 tag) public {
        slotZero.setInner(key, id, value);
        slotZero.setTag(other, tag);
        assert(slotZero.getInner(key, id) == value);
    }

    function checkNamespacedTagKeepsInner(uint256 key, uint256 id, uint256 other, uint256 value, uint32 tag) public {
        namespaced.setInner(key, id, value);
        namespaced.setTag(other, tag);
        assert(namespaced.getInner(key, id) == value);
    }

    function checkNamespacedInnerOverwritesInner(uint256 key, uint256 id, uint256 value, uint256 next) public {
        namespaced.setInner(key, id, value);
        namespaced.setInner(key, id, next);
        assert(namespaced.getInner(key, id) == value);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-contract", "SymbolicNestedFixedRoot"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 3 tests for test/SymbolicNestedFixedRoot.t.sol:SymbolicNestedFixedRoot
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [..]] checkNamespacedInnerOverwritesInner(uint256,uint256,uint256,uint256) ([METRICS])
[PASS] checkNamespacedTagKeepsInner(uint256,uint256,uint256,uint256,uint32) ([METRICS])
[PASS] checkSlotZeroTagKeepsInner(uint256,uint256,uint256,uint256,uint32) ([METRICS])
Suite result: FAILED. 2 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

// The optimizer folds the inner mapping roots of `cells[7][9]`, `values[7][9]`,
// `deepCells[7][8][9]`, `mids[7][8].cells[9]` and `root.children[7].children[8]`, and the entry of
// `deepValues[7][8][9]`, into constants. Each is found among the values of the symbolic entry it
// is compared with, so an entry written through symbolic keys still aliases them. The entry of
// `wides[7].last` sits at a struct offset above `2^64` and is not searched. The folded root of an
// etched store with no known layout is never found and stays a word equality.
#[forgetest_init]
fn symbolic_folded_nested_mapping_root_aliases_symbolic_entry(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_folded_nested_mapping_root_aliases_symbolic_entry");

    prj.update_config(|config| config.optimizer = Some(true));
    prj.add_test(
        "SymbolicFoldedNestedRoot.t.sol",
        r#"
struct Cell {
    uint32 tag;
    mapping(uint256 => uint256) inner;
}

contract Store {
    mapping(uint256 => mapping(uint256 => Cell)) cells;
    mapping(uint256 => mapping(uint256 => mapping(uint256 => uint256))) values;

    function setConstCell(uint256 id, uint256 value) external {
        cells[7][9].inner[id] = value;
    }

    function getConstCell(uint256 id) external view returns (uint256) {
        return cells[7][9].inner[id];
    }

    function setCell(uint256 k1, uint256 k2, uint256 id, uint256 value) external {
        cells[k1][k2].inner[id] = value;
    }

    function setConstValue(uint256 id, uint256 value) external {
        values[7][9][id] = value;
    }

    function getConstValue(uint256 id) external view returns (uint256) {
        return values[7][9][id];
    }

    function setValue(uint256 k1, uint256 k2, uint256 id, uint256 value) external {
        values[k1][k2][id] = value;
    }
}

contract DeepCellStore {
    mapping(uint256 => mapping(uint256 => mapping(uint256 => Cell))) deepCells;

    function setConstDeepCell(uint256 id, uint256 value) external {
        deepCells[7][8][9].inner[id] = value;
    }

    function getConstDeepCell(uint256 id) external view returns (uint256) {
        return deepCells[7][8][9].inner[id];
    }

    function setDeepCell(uint256 k1, uint256 k2, uint256 k3, uint256 id, uint256 value) external {
        deepCells[k1][k2][k3].inner[id] = value;
    }
}

contract DeepValueStore {
    mapping(uint256 => mapping(uint256 => mapping(uint256 => uint256))) deepValues;

    function setConstDeepValue(uint256 value) external {
        deepValues[7][8][9] = value;
    }

    function getConstDeepValue() external view returns (uint256) {
        return deepValues[7][8][9];
    }

    function setDeepValue(uint256 k1, uint256 k2, uint256 k3, uint256 value) external {
        deepValues[k1][k2][k3] = value;
    }
}

struct Node {
    uint256 value;
    mapping(uint256 => Node) children;
}

struct Mid {
    uint32 tag;
    mapping(uint256 => Cell) cells;
}

contract MidStore {
    mapping(uint256 => mapping(uint256 => Mid)) mids;

    function setConst(uint256 id, uint256 value) external {
        mids[7][8].cells[9].inner[id] = value;
    }

    function getConst(uint256 id) external view returns (uint256) {
        return mids[7][8].cells[9].inner[id];
    }

    function set(uint256 k1, uint256 k2, uint256 k3, uint256 id, uint256 value) external {
        mids[k1][k2].cells[k3].inner[id] = value;
    }
}

contract TreeStore {
    Node root;

    function setConst(uint256 value) external {
        root.children[7].children[8].children[9].value = value;
    }

    function getConst() external view returns (uint256) {
        return root.children[7].children[8].children[9].value;
    }

    function set(uint256 k1, uint256 k2, uint256 k3, uint256 value) external {
        root.children[k1].children[k2].children[k3].value = value;
    }
}

struct Wide {
    uint256[2 ** 64] items;
    uint256 last;
}

contract WideStore {
    mapping(uint256 => Wide) wides;

    function setConst(uint256 value) external {
        wides[7].last = value;
    }

    function getConst() external view returns (uint256) {
        return wides[7].last;
    }

    function set(uint256 key, uint256 value) external {
        wides[key].last = value;
    }
}

contract EtchedCellStore {
    mapping(uint256 => mapping(uint256 => Cell)) cells;

    function setConstCell(uint256 id, uint256 value) external {
        cells[3][5].inner[id] = value;
    }

    function getConstCell(uint256 id) external view returns (uint256) {
        return cells[3][5].inner[id];
    }

    function setCell(uint256 k1, uint256 k2, uint256 id, uint256 value) external {
        cells[k1][k2].inner[id] = value;
    }
}

interface Vm {
    function etch(address target, bytes calldata code) external;
}

contract SymbolicFoldedNestedRoot {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    Store store = new Store();
    DeepCellStore deepCellStore = new DeepCellStore();
    DeepValueStore deepValueStore = new DeepValueStore();
    MidStore midStore = new MidStore();
    TreeStore treeStore = new TreeStore();
    WideStore wideStore = new WideStore();
    EtchedCellStore etchedCellStore = EtchedCellStore(address(0xCE11));

    constructor() {
        vm.etch(address(etchedCellStore), abi.encodePacked(type(EtchedCellStore).runtimeCode, hex"00"));
    }

    function checkStructMemberEntryAliases(uint256 k1, uint256 k2, uint256 id, uint256 a, uint256 b) public {
        store.setConstCell(id, a);
        store.setCell(k1, k2, id, b);
        assert(store.getConstCell(id) == a);
    }

    function checkNestedEntryAliases(uint256 k1, uint256 k2, uint256 id, uint256 a, uint256 b) public {
        store.setConstValue(id, a);
        store.setValue(k1, k2, id, b);
        assert(store.getConstValue(id) == a);
    }

    function checkDeepStructMemberEntryAliases(uint256 k1, uint256 k2, uint256 k3, uint256 id, uint256 a, uint256 b)
        public
    {
        deepCellStore.setConstDeepCell(id, a);
        deepCellStore.setDeepCell(k1, k2, k3, id, b);
        assert(deepCellStore.getConstDeepCell(id) == a);
    }

    function checkDeepEntryAliases(uint256 k1, uint256 k2, uint256 k3, uint256 a, uint256 b) public {
        deepValueStore.setConstDeepValue(a);
        deepValueStore.setDeepValue(k1, k2, k3, b);
        assert(deepValueStore.getConstDeepValue() == a);
    }

    function checkMidStructEntryAliases(uint256 k1, uint256 k2, uint256 k3, uint256 id, uint256 a, uint256 b)
        public
    {
        midStore.setConst(id, a);
        midStore.set(k1, k2, k3, id, b);
        assert(midStore.getConst(id) == a);
    }

    function checkRecursiveEntryAliases(uint256 k1, uint256 k2, uint256 k3, uint256 a, uint256 b) public {
        treeStore.setConst(a);
        treeStore.set(k1, k2, k3, b);
        assert(treeStore.getConst() == a);
    }

    function checkWideOffsetEntryAliases(uint256 key, uint256 a, uint256 b) public {
        wideStore.setConst(a);
        wideStore.set(key, b);
        assert(wideStore.getConst() == a);
    }

    function checkUnknownNestedCellAliases(uint256 k1, uint256 k2, uint256 id, uint256 a, uint256 b) public {
        etchedCellStore.setConstCell(id, a);
        etchedCellStore.setCell(k1, k2, id, b);
        assert(etchedCellStore.getConstCell(id) == a);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-contract", "SymbolicFoldedNestedRoot"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 8 tests for test/SymbolicFoldedNestedRoot.t.sol:SymbolicFoldedNestedRoot
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [..]] checkDeepEntryAliases(uint256,uint256,uint256,uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [..]] checkDeepStructMemberEntryAliases(uint256,uint256,uint256,uint256,uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [..]] checkMidStructEntryAliases(uint256,uint256,uint256,uint256,uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [..]] checkNestedEntryAliases(uint256,uint256,uint256,uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [..]] checkRecursiveEntryAliases(uint256,uint256,uint256,uint256,uint256) ([METRICS])
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [..]] checkStructMemberEntryAliases(uint256,uint256,uint256,uint256,uint256) ([METRICS])
[FAIL: incomplete symbolic execution (Error): solver error: solver model does not satisfy path constraints involving symbolic Keccak heuristic] checkUnknownNestedCellAliases(uint256,uint256,uint256,uint256,uint256) ([METRICS])
[FAIL: incomplete symbolic execution (Error): solver error: solver model does not satisfy path constraints involving symbolic Keccak heuristic] checkWideOffsetEntryAliases(uint256,uint256,uint256) ([METRICS])
Suite result: FAILED. 0 passed; 8 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

// With enough source literals the search for the folded entry `values[7][9][id]` among the values
// of `values[k1][k2][id]` runs out of hashes, and the two slots stay a word equality.
#[forgetest_init]
fn symbolic_folded_mapping_entry_search_runs_out_of_hashes(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_folded_mapping_entry_search_runs_out_of_hashes");

    prj.update_config(|config| config.optimizer = Some(true));
    let branches: String = (1..=300)
        .map(|i| format!("        if (i == {}) return {};\n", i * 7919, i * 104729))
        .collect();
    prj.add_source(
        "Literals.sol",
        &format!(
            "contract Literals {{\n    function pick(uint256 i) external pure returns (uint256) {{\n{branches}        return 0;\n    }}\n}}\n"
        ),
    );
    prj.add_test(
        "SymbolicFoldedSearchBudget.t.sol",
        r#"
contract ValueStore {
    mapping(uint256 => mapping(uint256 => mapping(uint256 => uint256))) values;

    function setConstValue(uint256 id, uint256 value) external {
        values[7][9][id] = value;
    }

    function getConstValue(uint256 id) external view returns (uint256) {
        return values[7][9][id];
    }

    function setValue(uint256 k1, uint256 k2, uint256 id, uint256 value) external {
        values[k1][k2][id] = value;
    }
}

contract SymbolicFoldedSearchBudget {
    ValueStore store = new ValueStore();

    function checkNestedEntryAliases(uint256 k1, uint256 k2, uint256 id, uint256 a, uint256 b) public {
        store.setConstValue(id, a);
        store.setValue(k1, k2, id, b);
        assert(store.getConstValue(id) == a);
    }
}
"#,
    );

    assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-contract",
        "SymbolicFoldedSearchBudget",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicFoldedSearchBudget.t.sol:SymbolicFoldedSearchBudget
[FAIL: incomplete symbolic execution (Error): solver error: solver model does not satisfy path constraints involving symbolic Keccak heuristic] checkNestedEntryAliases(uint256,uint256,uint256,uint256,uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}
