//! Native Ethereum execution through the Forge CLI.

use alloy_consensus::{TxLegacy, transaction::Recovered};
use alloy_network::Ethereum;
use alloy_primitives::{Address, Bytes, TxKind, U256, bytes};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::{TransactionInput, TransactionRequest, state::AccountOverride};
use alloy_sol_types::SolCall;
use anvil::{EthereumHardfork, NodeConfig};
use evm2::{EvmFeatures, ExecutionConfig, SpecId, Version, env::BlockEnvExt, evm::EmptyDB};
use forge_script_sequence::ScriptSequence;
use foundry_cheatcodes::{CheatsConfig, Vm, ethereum::Cheatcodes};
use foundry_common::{FoundryTransactionBuilder, TransactionMaybeSigned};
use foundry_compilers::artifacts::EvmVersion;
use foundry_evm::{constants::CHEATCODE_ADDRESS, ethereum::Executor};
use foundry_test_utils::str;
use std::sync::Arc;

#[derive(serde::Deserialize)]
struct NativeInvariantFailureRecord {
    call_sequence: Vec<foundry_evm::fuzz::BaseCounterExample>,
    assertion_failure: bool,
}

#[forgetest_init]
fn ethereum_native_setup_and_test_state(prj: _, cmd: _) {
    prj.add_test(
        "NativeEthereum.t.sol",
        r#"
interface Vm {
    function deal(address account, uint256 balance) external;
    function warp(uint256 timestamp) external;
    function store(address target, bytes32 slot, bytes32 value) external;
}

contract NativeEthereumTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 public value;

    function setUp() public {
        value = 7;
        vm.warp(123);
    }

    function testMutatesItsOwnState() public {
        require(value == 7, "setup state");
        require(block.timestamp == 123, "setup environment");
        value = 9;
        vm.deal(address(42), 100);
        require(address(42).balance == 100, "deal");
        vm.store(address(this), bytes32(0), bytes32(uint256(11)));
        require(value == 11, "store");
    }

    function testRetainsSetupState() public view {
        require(value == 7, "test state leaked");
        require(address(42).balance == 0, "test balance leaked");
    }
}
"#,
    );

    cmd.args(["test", "--match-contract", "NativeEthereumTest"]).assert_success().stdout_eq(str![
        [r#"
...
Ran 2 tests for test/NativeEthereum.t.sol:NativeEthereumTest
[PASS] testMutatesItsOwnState() ([GAS])
[PASS] testRetainsSetupState() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)
...
"#]
    ]);
}

#[forgetest_init]
fn ethereum_native_revert_is_a_test_failure(prj: _, cmd: _) {
    prj.add_test(
        "NativeFailure.t.sol",
        r#"
contract NativeFailureTest {
    function testFailure() public pure {
        require(false, "native revert");
    }
}


"#,
    );

    cmd.args(["test", "--match-contract", "NativeFailureTest"]).assert_failure().stdout_eq(str![[
        r#"
...
Ran 1 test for test/NativeFailure.t.sol:NativeFailureTest
[FAIL: native revert] testFailure() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
Encountered a total of 1 failing tests, 0 tests succeeded
...
"#
    ]]);
}

#[forgetest_init]
fn ethereum_native_legacy_failure_flag_requires_opt_in(prj: _, cmd: _) {
    prj.add_test(
        "NativeLegacy.t.sol",
        r#"
contract NativeOrdinaryTest {
    bool public failed;

    function testFlag() public {
        failed = true;
    }
}

/// forge-config: default.legacy_assertions = true
contract NativeLegacyTest {
    bool public failed;

    function testFlag() public {
        failed = true;
    }
}
"#,
    );

    cmd.args(["test", "--match-contract", "NativeOrdinaryTest"]).assert_success();
    cmd.forge_fuse()
        .args(["test", "--match-contract", "NativeLegacyTest"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/NativeLegacy.t.sol:NativeLegacyTest
[FAIL] testFlag() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
Encountered a total of 1 failing tests, 0 tests succeeded
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_assertions_and_console(prj: _, cmd: _) {
    prj.add_test(
        "NativeAssertions.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {console} from "forge-std/console.sol";

contract NativeAssertionsTest is Test {
    function testAssertions() public {
        assertTrue(true);
        assertFalse(false);
        assertEq(uint256(42), uint256(42));
        assertEq(int256(-42), int256(-42));
        string memory text = "native";
        assertEq(text, text);
        assertEq(bytes("native"), bytes("native"));
        assertEqDecimal(uint256(100), uint256(100), 2);
        assertApproxEqAbs(uint256(100), uint256(101), 1);
        assertApproxEqRel(uint256(100), uint256(101), 0.01e18);
        uint256[] memory values = new uint256[](2);
        values[0] = 7;
        values[1] = 9;
        assertEq(values, values);
        console.log("native assertions passed");
    }
}

contract NativeAssertionFailureTest is Test {
    function testFailure() public {
        assertEq(uint256(1), uint256(2), "custom mismatch");
    }
}

/// forge-config: default.assertions_revert = false
contract NativeNonRevertingAssertionTest is Test {
    function testFailure() public {
        assertEq(uint256(1), uint256(2));
        console.log("continued after assertion");
    }

    function revertedAssertion() public {
        assertEq(uint256(1), uint256(2));
        console.log("inside reverted child");
        revert("after assertion");
    }

    function testCaughtRevert() public {
        (bool success,) = address(this).call(abi.encodeWithSignature("revertedAssertion()"));
        require(!success, "child should revert");
        console.log("after reverted child");
    }
}

/// forge-config: default.assertions_revert = false
contract NativeSetupAssertionTest is Test {
    function setUp() public { assertTrue(false); }
    function testShouldNotRun() public pure { revert("test ran after failed setup"); }
}
"#,
    );
    cmd.args(["test", "--match-contract", "^NativeAssertionsTest$", "-vv"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/NativeAssertions.t.sol:NativeAssertionsTest
[PASS] testAssertions() ([GAS])
Logs:
  native assertions passed

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
    cmd.forge_fuse()
        .args(["test", "--match-contract", "^NativeAssertionFailureTest$"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/NativeAssertions.t.sol:NativeAssertionFailureTest
[FAIL: custom mismatch: 1 != 2] testFailure() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
    cmd.forge_fuse()
        .args([
            "test",
            "--match-contract",
            "^NativeNonRevertingAssertionTest$",
            "--match-test",
            "testFailure",
            "-vv",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/NativeAssertions.t.sol:NativeNonRevertingAssertionTest
[FAIL: assertion failed] testFailure() ([GAS])
Logs:
  assertion failed: 1 != 2
  continued after assertion

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
    cmd.forge_fuse()
        .args([
            "test",
            "--match-contract",
            "^NativeNonRevertingAssertionTest$",
            "--match-test",
            "testCaughtRevert",
            "-vv",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/NativeAssertions.t.sol:NativeNonRevertingAssertionTest
[PASS] testCaughtRevert() ([GAS])
Logs:
  assertion failed: 1 != 2
  inside reverted child
  after reverted child

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
    cmd.forge_fuse()
        .args(["test", "--match-contract", "^NativeSetupAssertionTest$"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/NativeAssertions.t.sol:NativeSetupAssertionTest
[FAIL: assertion failed] setUp() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_expected_reverts(prj: _, cmd: _) {
    prj.add_test(
        "NativeReverts.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

interface NativeCheatVm {
    function _expectCheatcodeRevert(bytes calldata reason) external;
}

contract RevertTarget {
    error Custom(uint256 value);
    uint256 public value;
    function fail() external payable returns (uint256) { value = 9; revert("expected"); }
    function custom() external pure { revert Custom(7); }
    function write() external { value = 11; }
    function nested(RevertTarget inner) external { inner.fail(); }
}

contract RevertingConstructor {
    constructor(RevertTarget target) { target.write(); revert("constructor"); }
}

contract NativeExpectedRevertsTest is Test {
    RevertTarget target;
    function setUp() public { target = new RevertTarget(); }

    function testAnyRollback() public {
        vm.expectRevert();
        uint256 output = target.fail{value: 1}();
        require(output == 0, "dummy return");
        require(target.value() == 0, "reverted storage leaked");
        require(address(target).balance == 0, "reverted value leaked");
    }
    function testReason() public {
        vm.expectRevert(bytes("expected"));
        target.fail();
        vm.expectRevert(abi.encodeWithSignature("Error(string)", "expected"));
        target.fail();
    }
    function testCustomExact() public {
        vm.expectRevert(abi.encodeWithSelector(RevertTarget.Custom.selector, 7));
        target.custom();
    }
    function testCustomPartial() public {
        vm.expectPartialRevert(RevertTarget.Custom.selector);
        target.custom();
    }
    function testReverter() public {
        vm.expectRevert(address(target));
        target.fail();
    }
    function testNestedReverter() public {
        RevertTarget outer = new RevertTarget();
        vm.expectRevert(bytes("expected"), address(target));
        outer.nested(target);
    }
    function testCounted() public {
        vm.expectRevert(bytes("expected"), uint64(2));
        target.fail();
        target.fail();
    }
    function testCreateRollback() public {
        vm.expectRevert(bytes("constructor"));
        RevertingConstructor created = new RevertingConstructor(target);
        require(address(created) == address(1), "dummy create address");
        require(target.value() == 0, "constructor state leaked");
    }
    function testCountedCreate() public {
        vm.expectRevert(bytes("constructor"), uint64(2));
        new RevertingConstructor(target);
        new RevertingConstructor(target);
        require(target.value() == 0, "constructor state leaked");
    }
    function testCountedCreateReverter() public {
        bytes32 salt = bytes32(uint256(42));
        bytes32 codeHash = keccak256(abi.encodePacked(type(RevertingConstructor).creationCode, abi.encode(target)));
        address predicted = address(uint160(uint256(keccak256(abi.encodePacked(bytes1(0xff), address(this), salt, codeHash)))));
        vm.expectRevert(bytes("constructor"), predicted, uint64(2));
        new RevertingConstructor{salt: salt}(target);
        new RevertingConstructor{salt: salt}(target);
        require(target.value() == 0, "constructor state leaked");
    }
    function testNestedCountedReverter() public {
        RevertTarget outer = new RevertTarget();
        vm.expectRevert(bytes("expected"), address(outer), uint64(2));
        outer.nested(target);
        outer.nested(target);
    }
    function testCheatcode() public {
        NativeCheatVm(address(vm))._expectCheatcodeRevert(bytes("new nonce must be equal to or higher than the account's current nonce"));
        vm.setNonce(address(this), 0);
    }
    function testZeroCount() public {
        vm.expectRevert(uint64(0));
        target.write();
        require(target.value() == 11, "successful call not committed");
    }
    function testMissing() public { vm.expectRevert(); }
    function testInternalDisabled() public {
        vm.expectRevert(bytes("internal"));
        revert("internal");
    }
    function testWrongReason() public {
        vm.expectRevert(bytes("wrong"));
        target.fail();
    }
    function testTooFew() public {
        vm.expectRevert(uint64(2));
        target.fail();
    }
    function testSelectorIsExact() public {
        vm.expectRevert(RevertTarget.Custom.selector);
        target.custom();
    }
}

/// forge-config: default.allow_internal_expect_revert = true
contract NativeInternalRevertTest is Test {
    function testInternal() public {
        vm.expectRevert(bytes("internal"));
        revert("internal");
    }
    function testSuccessfulCallBeforeInternal() public {
        vm.expectRevert(bytes("internal"));
        (bool success,) = address(777).call("");
        require(success, "empty account call");
        revert("internal");
    }
}
"#,
    );
    cmd.args([
        "test",
        "--match-contract",
        "^NativeExpectedRevertsTest$",
        "--no-match-test",
        "test(Missing|WrongReason|TooFew|SelectorIsExact|InternalDisabled)",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 13 tests passed, 0 failed, 0 skipped (13 total tests)
...
"#]]);
    cmd.forge_fuse()
        .args(["test", "--match-contract", "^NativeInternalRevertTest$"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)
...
"#]]);
    cmd.forge_fuse()
        .args([
            "test",
            "--match-contract",
            "^NativeExpectedRevertsTest$",
            "--match-test",
            "testTooFew",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: next call did not revert as expected] [..] ([GAS])
...
Encountered a total of 1 failing tests, 0 tests succeeded
...
"#]]);
    cmd.forge_fuse()
        .args([
            "test",
            "--match-contract",
            "^NativeExpectedRevertsTest$",
            "--match-test",
            "testWrongReason",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: Error != expected error: expected != wrong] testWrongReason() ([GAS])
...
Encountered a total of 1 failing tests, 0 tests succeeded
...
"#]]);
    cmd.forge_fuse()
        .args([
            "test",
            "--match-contract",
            "^NativeExpectedRevertsTest$",
            "--match-test",
            "test(Missing|InternalDisabled)",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: call didn't revert at a lower depth than cheatcode call depth] testInternalDisabled() ([GAS])
[FAIL: call didn't revert at a lower depth than cheatcode call depth] testMissing() ([GAS])
...
Encountered a total of 2 failing tests, 0 tests succeeded
...
"#]]);
    cmd.forge_fuse()
        .args([
            "test",
            "--match-contract",
            "^NativeExpectedRevertsTest$",
            "--match-test",
            "testSelectorIsExact",
        ])
        .assert_failure();
}

#[forgetest_init]
fn ethereum_native_existing_expected_revert_fixtures(prj: _, cmd: _) {
    prj.add_test(
        "ExpectRevert.t.sol",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/default/cheats/ExpectRevert.t.sol"
        )),
    );
    prj.add_source(
        "utils/Test.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Test.sol")),
    );
    prj.add_source(
        "utils/DSTest.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/DSTest.sol")),
    );
    prj.add_source(
        "utils/Vm.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Vm.sol")),
    );
    prj.add_source(
        "utils/console.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/console.sol")),
    );
    cmd.args([
        "test", "--remappings", "utils/=src/utils/", "--match-contract",
        "^(ExpectRevertTest|ExpectRevertCount|ExpectRevertCountWithReverter|ExpectRevertPrecompileTest|ExpectRevertWithErrorTest)$",
        "--no-match-test", "test(expectCheatcodeRevert|ExpectRevertExactMatchAnyRevertData|ExpectPartialRevertMatchesAnySuffix)",
    ]).assert_success().stdout_eq(str![[r#"
...
Ran 5 test suites [ELAPSED]: 26 tests passed, 0 failed, 0 skipped (26 total tests)
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_existing_state_snapshot_fixtures(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.legacy_assertions = true;
        config.fuzz.runs = 16;
    });
    prj.add_test(
        "StateSnapshots.t.sol",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/default/cheats/StateSnapshots.t.sol"
        )),
    );
    prj.add_source(
        "utils/Test.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Test.sol")),
    );
    prj.add_source(
        "utils/DSTest.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/DSTest.sol")),
    );
    prj.add_source(
        "utils/Vm.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Vm.sol")),
    );
    prj.add_source(
        "utils/console.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/console.sol")),
    );
    cmd.args([
        "test", "--remappings", "utils/=src/utils/", "--match-contract",
        "^(StateSnapshotTest|StateSnapshotDeleteFromSetUpTest|StateSnapshotNestedRevertTest|NestedRestoreFrameRevertNonIsolatedTest|DeprecatedStateSnapshotTest)$",
        "--no-match-test", "test(RawTransaction|ExecuteTransaction)",
    ]).assert_success().stdout_eq(str![[r#"
...
Ran 5 test suites [ELAPSED]: 30 tests passed, 0 failed, 0 skipped (30 total tests)
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_snapshot_accepted_state_and_environment(prj: _, cmd: _) {
    prj.add_test("NativeSnapshots.t.sol", r#"
import {Test} from "forge-std/Test.sol";
import {console} from "forge-std/console.sol";

contract SnapshotTarget {
    uint256 public value = 7;
    function set(uint256 next) external { value = next; }
}

contract SnapshotRestorer is Test {
    function restoreAndRevert(uint256 id) external {
        require(vm.revertToState(id), "restore failed");
        revert("restored");
    }
}

contract NativeSnapshotsTest is Test {
    uint256 id;
    SnapshotRestorer helper;
    SnapshotTarget target;
    function setUp() public {
        helper = new SnapshotRestorer();
        id = vm.snapshotState();
        target = new SnapshotTarget();
    }
    function testRevertingRestoreKeepsUnloadedAcceptedCode() public {
        uint256 saved = id;
        SnapshotTarget accepted = target;
        vm.expectRevert(bytes("restored"));
        helper.restoreAndRevert(saved);
        require(address(accepted).code.length != 0, "accepted code removed");
        require(accepted.value() == 7, "accepted storage removed");
    }
    function testEnvironment() public {
        vm.warp(123);
        vm.roll(456);
        vm.chainId(789);
        vm.fee(10);
        vm.txGasPrice(11);
        uint256 saved = vm.snapshotState();
        vm.warp(321);
        vm.roll(654);
        vm.chainId(987);
        vm.fee(20);
        vm.txGasPrice(21);
        console.log("before restoration");
        require(vm.revertToState(saved), "restore failed");
        require(block.timestamp == 123 && block.number == 456, "block not restored");
        require(block.chainid == 789 && block.basefee == 10 && tx.gasprice == 11, "context not restored");
        console.log("after restoration");
    }
    function testDeleteDoesNotDeleteYoungerSnapshots() public {
        uint256 first = vm.snapshotState();
        uint256 second = vm.snapshotState();
        require(vm.deleteStateSnapshot(first), "delete failed");
        require(!vm.revertToState(first), "deleted snapshot restored");
        require(vm.revertToState(second), "younger snapshot deleted");
    }
}

/// forge-config: default.assertions_revert = false
contract NativeSnapshotFailureTest is Test {
    function testCannotEraseAssertionFailure() public {
        uint256 id = vm.snapshotState();
        assertTrue(false);
        require(vm.revertToState(id), "restore failed");
    }
}
"#);
    cmd.args(["test", "--match-contract", "^NativeSnapshotsTest$", "-vv"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] testEnvironment() ([GAS])
Logs:
  before restoration
  after restoration
...
Ran 1 test suite [ELAPSED]: 3 tests passed, 0 failed, 0 skipped (3 total tests)
...
"#]]);
    cmd.forge_fuse()
        .args(["test", "--match-contract", "^NativeSnapshotFailureTest$"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: assertion failed] testCannotEraseAssertionFailure() ([GAS])
...
Encountered a total of 1 failing tests, 0 tests succeeded
...
"#]]);
}

#[forgetest_init]
async fn ethereum_native_fork_pinned_state_and_test_independence(prj: _, cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()))).await;
    let provider = ProviderBuilder::new().connect_http(handle.http_endpoint().parse().unwrap());
    let sender = handle.dev_accounts().next().unwrap();
    // Empty calldata increments slot zero; getter calls return it. The constructor initializes 7.
    let init = bytes!(
        "6007600055601a6011600039601a6000f336600e57600054600101600055005b60005460005260206000f3"
    );
    let receipt = provider
        .send_transaction(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Create),
            input: TransactionInput::new(init),
            gas: Some(200_000),
            gas_price: Some(2_000_000_000),
            ..Default::default()
        })
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let target = receipt.contract_address.unwrap();
    let pinned = receipt.block_number.unwrap();
    let timestamp =
        provider.get_block_by_number(pinned.into()).await.unwrap().unwrap().header.timestamp;
    // Advance the upstream chain so a latest-state read would see 8 instead of 7.
    provider
        .send_transaction(TransactionRequest {
            from: Some(sender),
            to: Some(target.into()),
            gas: Some(100_000),
            gas_price: Some(2_000_000_000),
            ..Default::default()
        })
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    prj.add_test(
        "NativeFork.t.sol",
        &format!(
            r#"
import {{Test}} from "forge-std/Test.sol";
interface Remote {{ function value() external view returns (uint256); }}
contract NativeForkTest is Test {{
    address constant TARGET = {target};
    function setUp() public {{
        require(Remote(TARGET).value() == 7, "unpinned source storage");
        require(block.number == {pinned}, "wrong block number");
        require(block.timestamp == {timestamp}, "wrong block timestamp");
        require(block.chainid == 31337, "wrong execution chain");
    }}
    function testLocalWritesAndSnapshot() public {{
        uint256 id = vm.snapshotState();
        vm.store(TARGET, bytes32(0), bytes32(uint256(42)));
        require(Remote(TARGET).value() == 42, "local overlay missing");
        require(vm.revertToState(id), "restore failed");
        require(Remote(TARGET).value() == 7, "snapshot lost pinned source");
        vm.store(TARGET, bytes32(0), bytes32(uint256(99)));
    }}
    function testOtherTestKeepsPinnedSource() public view {{
        require(Remote(TARGET).value() == 7, "test state leaked");
    }}
}}
"#
        ),
    );
    cmd.args([
        "test",
        "--fork-url",
        &handle.http_endpoint(),
        "--fork-block-number",
        &pinned.to_string(),
        "--match-contract",
        "NativeForkTest",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)
...
"#]]);
    assert_eq!(provider.get_storage_at(target, U256::ZERO).await.unwrap(), U256::from(8));
    prj.add_test(
        "NativeForkSpec.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
contract NativeForkSpecTest is Test {
    function testLondon() public {
        vm.etch(address(0x1234), hex"5f60005260206000f3");
        (bool success,) = address(0x1234).call("");
        require(!success, "source hardfork enabled PUSH0");
    }
    function testShanghai() public {
        vm.etch(address(0x1234), hex"5f60005260206000f3");
        (bool success, bytes memory data) = address(0x1234).call("");
        require(success && abi.decode(data, (uint256)) == 0, "configured PUSH0 unavailable");
    }
}
"#,
    );
    for (version, test) in [("london", "testLondon"), ("shanghai", "testShanghai")] {
        cmd.forge_fuse()
            .args([
                "test",
                "--fork-url",
                &handle.http_endpoint(),
                "--fork-block-number",
                &pinned.to_string(),
                "--evm-version",
                version,
                "--match-contract",
                "NativeForkSpecTest",
                "--match-test",
                test,
            ])
            .assert_success()
            .stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)
...
"#]]);
    }
    // Source state remains intact after execution, including the code used by later reads.
    assert!(!provider.get_code_at(target).await.unwrap().is_empty());
    drop(api);
}

#[forgetest_init]
fn ethereum_native_existing_expected_call_fixtures(prj: _, cmd: _) {
    prj.add_test(
        "ExpectCall.t.sol",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/default/cheats/ExpectCall.t.sol"
        )),
    );
    prj.add_source(
        "utils/Test.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Test.sol")),
    );
    prj.add_source(
        "utils/DSTest.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/DSTest.sol")),
    );
    prj.add_source(
        "utils/Vm.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Vm.sol")),
    );
    prj.add_source(
        "utils/console.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/console.sol")),
    );
    cmd.args([
        "test",
        "--remappings",
        "utils/=src/utils/",
        "--match-contract",
        "^ExpectCall.*Test$",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 3 test suites [ELAPSED]: 32 tests passed, 0 failed, 0 skipped (32 total tests)
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_expected_call_failure_controls(prj: _, cmd: _) {
    prj.add_test("NativeCallExpectations.t.sol", r#"
import {Test} from "forge-std/Test.sol";
interface NativeVm { function _expectCheatcodeRevert(bytes calldata data) external; }
contract NativeCallTarget {
    function ping() external payable {}
    function fail() external pure { revert("original revert"); }
}
contract NativeCallExpectationsTest is Test {
    NativeCallTarget target;
    function setUp() public { target = new NativeCallTarget(); }
    function testMissing() public { vm.expectCall(address(target), abi.encodeCall(target.ping, ())); }
    function testTooMany() public {
        vm.expectCall(address(target), abi.encodeCall(target.ping, ()), 1);
        target.ping(); target.ping();
    }
    function testWrongValue() public {
        vm.expectCall(address(target), 1, abi.encodeCall(target.ping, ()));
        target.ping();
    }
    function testWrongScheme() public {
        vm.expectDelegateCall(address(target), abi.encodeCall(target.ping, ()));
        target.ping();
    }
    function testPreservesOriginalRevert() public {
        vm.expectCall(address(target), abi.encodeCall(target.ping, ()));
        target.fail();
    }
    function testCountedOverwrite() public {
        bytes memory data = abi.encodeCall(target.ping, ());
        vm.expectCall(address(target), data, 1);
        NativeVm(address(vm))._expectCheatcodeRevert(bytes("counted expected calls can only bet set once"));
        vm.expectCall(address(target), data, 2);
        target.ping();
    }
    function testNonCountedOverwrite() public {
        bytes memory data = abi.encodeCall(target.ping, ());
        vm.expectCall(address(target), data, 1);
        NativeVm(address(vm))._expectCheatcodeRevert(bytes("cannot overwrite a counted expectCall with a non-counted expectCall"));
        vm.expectCall(address(target), data);
        target.ping();
    }
    function testSatisfiedBeforeExpectedRevert() public {
        vm.expectCall(address(target), abi.encodeCall(target.fail, ()));
        vm.expectRevert(bytes("original revert"));
        target.fail();
    }
}
"#);
    cmd.args([
        "test",
        "--match-contract",
        "NativeCallExpectationsTest",
        "--match-test",
        "test(CountedOverwrite|NonCountedOverwrite|SatisfiedBeforeExpectedRevert)",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 3 tests passed, 0 failed, 0 skipped (3 total tests)
...
"#]]);
    cmd.forge_fuse()
        .args([
            "test",
            "--match-contract",
            "NativeCallExpectationsTest",
            "--match-test",
            "test(Missing|TooMany|WrongValue|WrongScheme)",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
...
Encountered a total of 4 failing tests, 0 tests succeeded
...
"#]]);
    cmd.forge_fuse()
        .args([
            "test",
            "--match-contract",
            "NativeCallExpectationsTest",
            "--match-test",
            "testPreservesOriginalRevert",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: original revert] testPreservesOriginalRevert() ([GAS])
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_existing_mock_call_fixtures(prj: _, cmd: _) {
    prj.add_test(
        "MockCall.t.sol",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/default/cheats/MockCall.t.sol"
        )),
    );
    prj.add_source(
        "utils/Test.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Test.sol")),
    );
    prj.add_source(
        "utils/DSTest.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/DSTest.sol")),
    );
    prj.add_source(
        "utils/Vm.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Vm.sol")),
    );
    prj.add_source(
        "utils/console.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/console.sol")),
    );
    cmd.args([
        "test",
        "--remappings",
        "utils/=src/utils/",
        "--match-contract",
        "^MockCall(Revert)?Test$",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 2 test suites [ELAPSED]: 26 tests passed, 0 failed, 0 skipped (26 total tests)
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_mock_call_queues_and_rollback(prj: _, cmd: _) {
    prj.add_test("NativeMockQueues.t.sol", r#"
import {Test} from "forge-std/Test.sol";
contract NativeMockQueuesTest is Test {
    address constant TARGET = address(0x1234);
    bytes constant DATA = hex"12345678";
    function setUp() public {
        bytes[] memory mocks = new bytes[](2);
        mocks[0] = abi.encode(uint256(7));
        mocks[1] = abi.encode(uint256(9));
        vm.mockCalls(TARGET, DATA, mocks);
    }
    function value() internal returns (uint256) {
        (bool ok, bytes memory data) = TARGET.call(DATA);
        require(ok, "mock failed");
        return abi.decode(data, (uint256));
    }
    function testQueueKeepsLast() public {
        require(value() == 7 && value() == 9 && value() == 9, "queue order");
    }
    function testOtherTestStartsAtFirst() public { require(value() == 7, "queue leaked"); }
    function testFailedTransferKeepsQueue() public {
        bytes[] memory mocks = new bytes[](2);
        mocks[0] = abi.encode(uint256(11)); mocks[1] = abi.encode(uint256(13));
        vm.mockCalls(TARGET, 1 ether, DATA, mocks);
        vm.deal(address(this), 0);
        (bool ok,) = TARGET.call{value: 1 ether}(DATA);
        require(!ok, "unfunded mock succeeded");
        vm.deal(address(this), 2 ether);
        bytes memory data;
        (ok, data) = TARGET.call{value: 1 ether}(DATA);
        require(ok && abi.decode(data, (uint256)) == 11, "failed transfer consumed queue");
        (ok, data) = TARGET.call{value: 1 ether}(DATA);
        require(ok && abi.decode(data, (uint256)) == 13, "funded queue order");
        require(TARGET.balance == 2 ether && address(this).balance == 0, "balance transfer");
    }
    function testMockRevertRestoresValue() public {
        vm.deal(address(this), 1 ether);
        vm.mockCallRevert(TARGET, 1 ether, DATA, bytes("mock revert"));
        (bool ok, bytes memory data) = TARGET.call{value: 1 ether}(DATA);
        require(!ok && keccak256(data) == keccak256(bytes("mock revert")), "revert output");
        require(TARGET.balance == 0 && address(this).balance == 1 ether, "revert transferred value");
    }
    function revertingChild() external {
        TARGET.call{value: 1 ether}(DATA);
        revert("child revert");
    }
    function testParentRevertRestoresMockTransfer() public {
        vm.deal(address(this), 1 ether);
        vm.expectRevert(bytes("child revert"));
        this.revertingChild();
        require(TARGET.balance == 0 && address(this).balance == 1 ether, "parent revert kept transfer");
    }
    function testEmptyQueueFallsThrough() public {
        bytes[] memory empty = new bytes[](0);
        vm.mockCalls(TARGET, DATA, empty);
        (bool ok, bytes memory data) = TARGET.call(DATA);
        require(ok && data.length == 0, "empty queue did not execute target");
    }
    function testExpectCallObservesMock() public {
        vm.expectCall(TARGET, DATA, 1);
        require(value() == 7, "expectation changed mock");
    }
}
"#);
    cmd.args(["test", "--match-contract", "NativeMockQueuesTest"]).assert_success().stdout_eq(
        str![[r#"
...
Ran 1 test suite [ELAPSED]: 7 tests passed, 0 failed, 0 skipped (7 total tests)
...
"#]],
    );
}

#[forgetest_init]
fn ethereum_native_existing_mock_function_fixtures(prj: _, cmd: _) {
    prj.add_test(
        "MockFunction.t.sol",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/default/cheats/MockFunction.t.sol"
        )),
    );
    prj.add_source(
        "utils/Test.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Test.sol")),
    );
    prj.add_source(
        "utils/DSTest.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/DSTest.sol")),
    );
    prj.add_source(
        "utils/Vm.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Vm.sol")),
    );
    prj.add_source(
        "utils/console.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/console.sol")),
    );
    cmd.args([
        "test",
        "--remappings",
        "utils/=src/utils/",
        "--match-contract",
        "^MockFunctionTest$",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 9 tests passed, 0 failed, 0 skipped (9 total tests)
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_mock_function_context(prj: _, cmd: _) {
    prj.add_test("NativeMockFunction.t.sol", r#"
import {Test} from "forge-std/Test.sol";
contract OriginalFunction {
    uint256 public value = 3;
    function write(uint256 x) external payable returns (address, uint256, address, uint256) {
        value = x;
        return (address(this), value, msg.sender, msg.value);
    }
    function read() external view returns (uint256) { return value; }
    function fail() external { value = 44; revert("original"); }
}
contract ReplacementFunction {
    uint256 public value = 900;
    uint256 immutable offset;
    constructor(uint256 delta) { offset = delta; }
    function write(uint256 x) external payable returns (address, uint256, address, uint256) {
        value += x + offset;
        return (address(this), value, msg.sender, msg.value);
    }
    function read() external view returns (uint256) { return value + offset; }
    function fail() external { value = 55; revert("replacement"); }
}
contract NativeMockFunctionTest is Test {
    OriginalFunction original;
    ReplacementFunction replacement;
    function setUp() public {
        original = new OriginalFunction();
        replacement = new ReplacementFunction(100);
    }
    function testPreservesContextAndValue() public {
        bytes32 codeHash = address(original).codehash;
        vm.mockFunction(address(original), address(replacement), abi.encodeWithSelector(original.write.selector));
        vm.deal(address(0x1234), 10);
        vm.prank(address(0x1234));
        (address context, uint256 value, address caller, uint256 amount) = original.write{value: 4}(7);
        require(context == address(original) && value == 110, "wrong storage context");
        require(caller == address(0x1234) && amount == 4, "wrong caller/value");
        require(original.value() == 110 && replacement.value() == 900, "implementation storage changed");
        require(address(original).balance == 4 && address(replacement).balance == 0, "wrong transfer target");
        require(address(original).codehash == codeHash, "original code replaced");
    }
    function testStaticReadsOriginalStorage() public {
        vm.mockFunction(address(original), address(replacement), abi.encodeCall(original.read, ()));
        require(original.read() == 103, "static call used implementation storage");
    }
    function testExactArgumentsOverrideSelector() public {
        ReplacementFunction exact = new ReplacementFunction(200);
        vm.mockFunction(address(original), address(replacement), abi.encodeWithSelector(original.write.selector));
        vm.mockFunction(address(original), address(exact), abi.encodeCall(original.write, (7)));
        (, uint256 first,,) = original.write(7);
        (, uint256 second,,) = original.write(8);
        require(first == 210 && second == 318, "wrong exact/selector precedence");
    }
    function testUnmatchedArgumentsUseOriginal() public {
        vm.mockFunction(address(original), address(replacement), abi.encodeCall(original.write, (7)));
        (, uint256 value,,) = original.write(8);
        require(value == 8, "unmatched calldata redirected");
    }
    function testReplacementRevertRollsBackOriginal() public {
        vm.mockFunction(address(original), address(replacement), abi.encodeCall(original.fail, ()));
        vm.expectRevert(bytes("replacement"));
        original.fail();
        require(original.value() == 3 && replacement.value() == 900, "revert kept writes");
    }
    function testClearCallMocksKeepsFunctionMock() public {
        vm.mockFunction(address(original), address(replacement), abi.encodeCall(original.read, ()));
        vm.clearMockedCalls();
        require(original.read() == 103, "clearMockedCalls removed function redirect");
    }
    function testCallExpectationSeesReplacement() public {
        bytes memory data = abi.encodeCall(original.read, ());
        vm.mockFunction(address(original), address(replacement), data);
        vm.expectCall(address(replacement), data, 1);
        require(original.read() == 103, "expectation changed redirect");
    }
    function testCallMockOverridesReplacement() public {
        bytes memory data = abi.encodeCall(original.read, ());
        vm.mockFunction(address(original), address(replacement), data);
        vm.mockCall(address(replacement), data, abi.encode(uint256(42)));
        require(original.read() == 42, "call mock did not intercept replacement");
    }
}
"#);
    cmd.args(["test", "--match-contract", "NativeMockFunctionTest"]).assert_success().stdout_eq(
        str![[r#"
...
Ran 1 test suite [ELAPSED]: 8 tests passed, 0 failed, 0 skipped (8 total tests)
...
"#]],
    );
}

#[forgetest_init]
fn ethereum_native_existing_expected_emit_fixtures(prj: _, cmd: _) {
    prj.add_test(
        "ExpectEmit.t.sol",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/default/cheats/ExpectEmit.t.sol"
        )),
    );
    prj.add_source(
        "utils/Test.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Test.sol")),
    );
    prj.add_source(
        "utils/DSTest.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/DSTest.sol")),
    );
    prj.add_source(
        "utils/Vm.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Vm.sol")),
    );
    prj.add_source(
        "utils/console.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/console.sol")),
    );
    cmd.args([
        "test",
        "--remappings",
        "utils/=src/utils/",
        "--match-contract",
        "^ExpectEmit(Count)?Test$",
        "--no-match-test",
        r"^testExpectEmit(Nested)?\(",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 2 test suites [ELAPSED]: 22 tests passed, 0 failed, 0 skipped (22 total tests)
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_emit_failures_and_rollback(prj: _, cmd: _) {
    prj.add_test("NativeEmit.t.sol", r#"
import {Test} from "forge-std/Test.sol";
contract NativeEmitter {
    uint256 public value;
    event Item(uint256 indexed key, uint256 data);
    event AnonymousItem(uint256 indexed key, uint256 data) anonymous;
    event Empty() anonymous;
    function item(uint256 key, uint256 data) external { value = 7; emit Item(key, data); value = 9; }
    function anonymousItem() external { emit AnonymousItem(3, 4); }
    function empty() external { emit Empty(); }
    function nothing() external { value = 9; }
    function failed() external { revert("original emit failure"); }
}
contract NativeEmitTest is Test {
    NativeEmitter emitter;
    event Item(uint256 indexed key, uint256 data);
    event AnonymousItem(uint256 indexed key, uint256 data) anonymous;
    event Empty() anonymous;
    function setUp() public { emitter = new NativeEmitter(); }
    function testAnonymousWithFlags() public {
        vm.expectEmitAnonymous(true, false, false, false, true, address(emitter));
        emit AnonymousItem(3, 4);
        emitter.anonymousItem();
    }
    function testAnonymousWithoutEmitter() public {
        vm.expectEmitAnonymous(true, false, false, false, true);
        emit AnonymousItem(3, 4);
        emitter.anonymousItem();
    }
    function testAnonymousEmpty() public {
        vm.expectEmitAnonymous(); emit Empty(); emitter.empty();
        vm.expectEmitAnonymous(address(emitter)); emit Empty(); emitter.empty();
    }
    function testCountZeroStopsAndRollsBack() public {
        vm.expectEmit(address(emitter), 0); emit Item(3, 4);
        (bool ok, bytes memory data) = address(emitter).call(abi.encodeCall(emitter.item, (3, 4)));
        require(!ok, "zero-count call succeeded");
        assertEq(data, abi.encodeWithSignature("CheatcodeError(string)", "log emitted but expected 0 times"));
        require(emitter.value() == 0, "failed log kept state");
    }
    function validationChild() external {
        vm.expectEmit(address(emitter)); emit Item(3, 4);
        (bool ok,) = address(emitter).call(abi.encodeCall(emitter.nothing, ()));
        require(!ok, "missing event did not reject call");
        require(emitter.value() == 0, "rejected call kept writes");
        revert("child inspected");
    }
    function testCallEndValidationRollsBack() public {
        vm.expectRevert(bytes("child inspected")); this.validationChild();
        require(emitter.value() == 0, "validation child leaked state");
    }
    function testMissing() public { vm.expectEmit(); emit Item(3, 4); }
    function testWrongData() public { vm.expectEmit(); emit Item(3, 4); emitter.item(3, 5); }
    function testWrongEmitter() public { vm.expectEmit(address(0x1234)); emit Item(3, 4); emitter.item(3, 4); }
    function testOriginalRevert() public { vm.expectEmit(); emit Item(3, 4); emitter.failed(); }
}
"#);
    cmd.args([
        "test",
        "--match-contract",
        "NativeEmitTest",
        "--match-test",
        "test(Anonymous|CountZero)",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 4 tests passed, 0 failed, 0 skipped (4 total tests)
...
"#]]);
    cmd.forge_fuse()
        .args([
            "test",
            "--match-contract",
            "NativeEmitTest",
            "--match-test",
            "test(Missing|WrongData|WrongEmitter)",
        ])
        .assert_failure()
        .stdout_eq(str![[r#"
...
Encountered a total of 3 failing tests, 0 tests succeeded
...
"#]]);
    cmd.forge_fuse().args(["test", "--match-contract", "NativeEmitTest", "--match-test", "testCallEndValidationRollsBack"])
        .assert_failure().stdout_eq(str![[r#"
...
[FAIL: expected an emit, but no logs were emitted afterwards. you might have mismatched events or not enough events were emitted] testCallEndValidationRollsBack() ([GAS])
...
"#]]);
    cmd.forge_fuse()
        .args(["test", "--match-contract", "NativeEmitTest", "--match-test", "testOriginalRevert"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: original emit failure] testOriginalRevert() ([GAS])
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_fuzz_owned_cases(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.runs = 32;
        config.fuzz.seed = Some(U256::from(1));
        config.fuzz.failure_persist_dir = None;
        config.fuzz.max_test_rejects = 1024;
    });
    prj.add_test("NativeFuzz.t.sol", r#"
import {Test} from "forge-std/Test.sol";
contract NativeFuzzTest is Test {
    uint256 value;
    uint256[] public fixture_amount;
    enum Choice { A, B, C }
    struct Nested { Choice choice; uint256[] numbers; }
    function setUp() public { value = 7; fixture_amount.push(0); fixture_amount.push(type(uint256).max); }
    function fixture_owner() public returns (address[] memory owners) {
        value = 99;
        owners = new address[](2); owners[0] = address(0x1234); owners[1] = address(0x5678);
    }
    function testIndependent(uint256 amount, address owner) public {
        require(value == 7, "case or fixture state leaked");
        value = amount;
        vm.deal(owner, amount);
        require(owner.balance == amount, "live fuzz state");
    }
    function testDynamic(bytes memory input, string memory text, uint256[] memory numbers, int128 signedValue) public {
        (bytes memory decodedInput, string memory decodedText, uint256[] memory decodedNumbers, int128 decodedSigned) =
            abi.decode(abi.encode(input, text, numbers, signedValue), (bytes, string, uint256[], int128));
        assertEq(decodedInput, input);
        assertEq(decodedText, text);
        assertEq(decodedNumbers, numbers);
        assertEq(decodedSigned, signedValue);
    }
    function testEnums(Choice choice, Choice[] memory choices, Nested memory nested) public pure {
        require(uint256(choice) < 3 && uint256(nested.choice) < 3, "invalid enum");
        for (uint256 i; i < choices.length; ++i) require(uint256(choices[i]) < 3, "invalid array enum");
    }
    function testAssume(bool accepted) public { vm.assume(accepted); require(value == 7, "rejected case leaked"); value = 9; }
    function testExpectedRevert(uint256 amount) public {
        vm.expectRevert(bytes("fuzz revert")); this.fail(amount);
    }
    function fail(uint256) external pure { revert("fuzz revert"); }
    function testFailure(uint8 amount) public pure { require(amount > 255, "native fuzz failure"); }
    function testRejectAll(uint256) public { vm.assume(false); }
}

"#);
    cmd.args([
        "test",
        "--match-contract",
        "NativeFuzzTest",
        "--no-match-test",
        "test(Failure|RejectAll)",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 5 tests passed, 0 failed, 0 skipped (5 total tests)
...
"#]]);
    cmd.forge_fuse()
        .args(["test", "--match-contract", "NativeFuzzTest", "--match-test", "testFailure"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: native fuzz failure; counterexample: [..]] testFailure(uint8) (runs: 0, [..])
...
"#]]);
    cmd.forge_fuse()
        .args(["test", "--match-contract", "NativeFuzzTest", "--match-test", "testRejectAll"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: Too many global rejects] testRejectAll(uint256) (runs: 0, [..])
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_fuzz_failure_cache(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.runs = 1;
        config.fuzz.seed = Some(U256::ONE);
        config.fuzz.dictionary.dictionary_weight = 0;
    });
    prj.add_test(
        "NativeFuzzCache.t.sol",
        r#"
contract NativeFuzzCacheTest {
    function testFuzz_value(uint256 value) public pure {
        require(value == 0, "nonzero fuzz value");
    }
}
"#,
    );
    cmd.args(["test", "--mc", "NativeFuzzCacheTest"]).assert_failure();
    let path = prj.root().join("cache/fuzz/failures/NativeFuzzCacheTest/testFuzz_value");
    let mut failure =
        foundry_common::fs::read_json_file::<foundry_evm::fuzz::BaseCounterExample>(&path).unwrap();
    let generated = failure.calldata.clone();
    assert_ne!(U256::from_be_slice(&generated[4..]), U256::ZERO);
    // A stale successful replay must leave the seeded fresh run available.
    failure.calldata = [generated[..4].to_vec(), vec![0; 32]].concat().into();
    foundry_common::fs::write_json_file(&path, &failure).unwrap();
    cmd.forge_fuse().args(["test", "--mc", "NativeFuzzCacheTest"]).assert_failure();
    let mut failure =
        foundry_common::fs::read_json_file::<foundry_evm::fuzz::BaseCounterExample>(&path).unwrap();
    assert_eq!(failure.calldata, generated);
    assert_eq!(failure.fuzz.run, Some(1));
    // A still-failing cache entry takes precedence over freshly generated calldata.
    failure.calldata =
        [generated[..4].to_vec(), U256::from(42).to_be_bytes::<32>().to_vec()].concat().into();
    foundry_common::fs::write_json_file(&path, &failure).unwrap();
    cmd.forge_fuse().args(["test", "--mc", "NativeFuzzCacheTest"]).assert_failure();
    let replayed =
        foundry_common::fs::read_json_file::<foundry_evm::fuzz::BaseCounterExample>(&path).unwrap();
    assert_eq!(replayed.calldata, failure.calldata);
    assert_eq!(replayed.fuzz.seed, failure.fuzz.seed);
    assert_eq!(replayed.fuzz.run, failure.fuzz.run);
    assert_eq!(replayed.fuzz.worker, failure.fuzz.worker);
}

#[forgetest_init]
fn ethereum_native_fuzz_dictionary_sources(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.runs = 16;
        config.fuzz.seed = Some(U256::ONE);
        config.fuzz.max_test_rejects = 4096;
        config.fuzz.dictionary.dictionary_weight = 100;
        config.fuzz.dictionary.max_fuzz_dictionary_literals = 0;
        config.fuzz.dictionary.include_push_bytes = false;
        config.fuzz.dictionary.include_storage = true;
    });
    prj.add_test("NativeDictionary.t.sol", r#"
interface VmDictionary { function assume(bool condition) external pure; }
contract NativeDictionaryTest {
    VmDictionary constant vm = VmDictionary(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 seeded;
    function setUp() public { seeded = uint256(keccak256("native storage dictionary")); }
    function testStorage(uint256 value) public view {
        vm.assume(value == seeded);
        require(value == uint256(keccak256("native storage dictionary")), "wrong storage seed");
    }
    function testPush(uint256 value) public pure {
        vm.assume(value == 0x123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0);
    }
}
"#);
    cmd.args(["test", "--mc", "NativeDictionaryTest", "--mt", "testStorage"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] testStorage(uint256) (runs: 16, [..])
...
"#]]);
    prj.update_config(|config| {
        config.fuzz.dictionary.include_push_bytes = true;
        config.fuzz.dictionary.include_storage = false;
    });
    cmd.forge_fuse()
        .args(["test", "--mc", "NativeDictionaryTest", "--mt", "testPush"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] testPush(uint256) (runs: 16, [..])
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_fuzz_revert_policy(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.runs = 8;
        config.fuzz.seed = Some(U256::ONE);
        config.fuzz.fail_on_revert = false;
    });
    prj.add_test(
        "NativeFuzzRevert.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
contract NativeRevertingChild {
    function fail(uint256) external pure { revert("child revert"); }
}
contract NativeFuzzRevertTest is Test {
    NativeRevertingChild child;
    function setUp() public {
        child = new NativeRevertingChild();
        (bool success,) = address(child).call(abi.encodeCall(child.fail, (0)));
        require(!success, "setup revert control");
    }
    function testChild(uint256 value) public view { child.fail(value); }
    function testTarget(uint256) public pure { revert("target revert"); }
    function testCheatcode(uint256) public pure { vm.assertTrue(false, "cheatcode revert"); }
    function testHalt(uint256) public pure { assembly { invalid() } }
    function testExpectation(uint256) public { vm.expectCall(address(child), abi.encodeCall(child.fail, (0))); }
}
"#,
    );
    cmd.args(["test", "--mc", "NativeFuzzRevertTest", "--mt", "testChild"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] testChild(uint256) (runs: 8, [..])
...
"#]]);
    for test in ["testTarget", "testCheatcode", "testHalt", "testExpectation"] {
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeFuzzRevertTest", "--mt", test])
            .assert_failure();
    }
    prj.update_config(|config| config.fuzz.fail_on_revert = true);
    cmd.forge_fuse()
        .args(["test", "--mc", "NativeFuzzRevertTest", "--mt", "testChild"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: child revert; counterexample: [..]] testChild(uint256) (runs: 0, [..])
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_fuzz_legacy_assertions(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.runs = 8;
        config.fuzz.seed = Some(U256::ONE);
    });
    prj.add_test(
        "NativeFuzzLegacy.t.sol",
        r#"
interface VmLegacy { function load(address target, bytes32 slot) external view returns (bytes32); }
contract NativeFuzzOrdinaryTest {
    bool internal flag;
    VmLegacy constant vm = VmLegacy(address(uint160(uint256(keccak256("hevm cheat code")))));
    function failed() public view returns (bool) { return vm.load(address(this), bytes32(0)) != 0; }
    function testFlag(uint256) public { flag = true; }
    function testIndependent(uint256) public { require(!flag, "case state leaked"); flag = true; }
}
/// forge-config: default.legacy_assertions = true
contract NativeFuzzLegacyTest is NativeFuzzOrdinaryTest {}
"#,
    );
    cmd.args(["test", "--mc", "^NativeFuzzOrdinaryTest$"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)
...
"#]]);
    cmd.forge_fuse().args(["test", "--mc", "^NativeFuzzLegacyTest$"]).assert_failure().stdout_eq(
        str![[r#"
...
Ran 1 test suite [ELAPSED]: 0 tests passed, 2 failed, 0 skipped (2 total tests)
...
"#]],
    );
}

#[forgetest_init]
fn ethereum_native_existing_artifact_deployment_fixtures(prj: _, cmd: _) {
    prj.update_config(|config| config.legacy_assertions = true);
    prj.add_test(
        "DeployCode.t.sol",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/default/cheats/DeployCode.t.sol"
        ))
        .replace("cheats/DeployCode.t.sol", "DeployCode.t.sol")
        .as_str(),
    );
    prj.add_source(
        "utils/Test.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Test.sol")),
    );
    prj.add_source(
        "utils/DSTest.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/DSTest.sol")),
    );
    prj.add_source(
        "utils/Vm.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Vm.sol")),
    );
    prj.add_source(
        "utils/console.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/console.sol")),
    );
    cmd.args([
        "test",
        "--remappings",
        "utils/=src/utils/",
        "--mc",
        "^DeployCodeNonIsolatedTest$",
        "--no-match-test",
        "testDeployCodeWithRemapping",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 10 tests passed, 0 failed, 0 skipped (10 total tests)
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_artifact_owned_creation(prj: _, cmd: _) {
    prj.update_config(|config| config.isolate = false);
    prj.add_source("NativeArtifactChild.sol", r#"
interface VmArtifactChild {
    function warp(uint256 timestamp) external;
    function deployCode(string calldata path) external returns (address);
}
contract NativeArtifactLeaf {}
contract NativeArtifactChild {
    address public sender;
    address public origin;
    uint256 public value;
    address public leaf;
    constructor(uint256 timestamp) payable {
        sender = msg.sender; origin = tx.origin; value = msg.value;
        VmArtifactChild vm = VmArtifactChild(address(uint160(uint256(keccak256("hevm cheat code")))));
        vm.warp(timestamp);
        leaf = vm.deployCode("src/NativeArtifactChild.sol:NativeArtifactLeaf");
    }
}
contract NativeArtifactReverter { constructor() { revert("artifact constructor revert"); } }
"#);
    prj.add_test("NativeArtifact.t.sol", r#"
import {Test} from "forge-std/Test.sol";
import {NativeArtifactChild} from "src/NativeArtifactChild.sol";
contract NativeArtifactTest is Test {
    function testContextAndNestedCreation() public {
        NativeArtifactChild child = NativeArtifactChild(vm.deployCode("src/NativeArtifactChild.sol:NativeArtifactChild", abi.encode(123), 5));
        assertEq(child.sender(), address(this));
        assertEq(child.origin(), tx.origin);
        assertEq(child.value(), 5);
        assertEq(address(child).balance, 5);
        assertEq(block.timestamp, 123);
        assertGt(child.leaf().code.length, 0);
        assertEq(address(child).code, vm.getDeployedCode("src/NativeArtifactChild.sol:NativeArtifactChild"));
        assertGt(vm.getCode("src/NativeArtifactChild.sol:NativeArtifactChild").length, address(child).code.length);
    }
    function testPrankAndOrigin() public {
        vm.prank(address(123), address(456));
        NativeArtifactChild child = NativeArtifactChild(vm.deployCode("src/NativeArtifactChild.sol:NativeArtifactChild", abi.encode(42)));
        assertEq(child.sender(), address(123));
        assertEq(child.origin(), address(456));
        assertEq(tx.origin, address(0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38));
    }
    function testExpectedConstructorRevert() public {
        vm.expectRevert(bytes("artifact constructor revert"));
        vm.deployCode("src/NativeArtifactChild.sol:NativeArtifactReverter");
    }
    function testParentRollback() public {
        uint64 nonce = vm.getNonce(address(this));
        assertEq(nonce, 1);
        address deployed = address(uint160(uint256(keccak256(abi.encodePacked(hex"d694", address(this), hex"01")))));
        vm.expectRevert(bytes("parent rollback"));
        this.deployThenRevert();
        assertEq(vm.getNonce(address(this)), nonce);
        assertEq(deployed.code.length, 0);
        assertEq(block.timestamp, 123);
    }
    function setUp() public { vm.warp(7); }
    function deployThenRevert() external {
        vm.deployCode("src/NativeArtifactChild.sol:NativeArtifactChild", abi.encode(123));
        revert("parent rollback");
    }
}
"#);
    cmd.args(["test", "--mc", "^NativeArtifactTest$"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 4 tests passed, 0 failed, 0 skipped (4 total tests)
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_existing_create2_factory_fixtures(prj: _, cmd: _) {
    prj.update_config(|config| config.legacy_assertions = true);
    prj.add_test(
        "DeployCode.t.sol",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/default/cheats/DeployCode.t.sol"
        ))
        .replace("cheats/DeployCode.t.sol", "DeployCode.t.sol")
        .as_str(),
    );
    prj.add_source(
        "utils/Test.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Test.sol")),
    );
    prj.add_source(
        "utils/DSTest.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/DSTest.sol")),
    );
    prj.add_source(
        "utils/Vm.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Vm.sol")),
    );
    prj.add_source(
        "utils/console.sol",
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/console.sol")),
    );
    cmd.args([
        "test",
        "--remappings",
        "utils/=src/utils/",
        "--mc",
        "^DeployCodeCreate2FactoryNonIsolatedTest$",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 8 tests passed, 0 failed, 0 skipped (8 total tests)
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_create2_factory_value_and_pranks(prj: _, cmd: _) {
    prj.update_config(|config| config.always_use_create_2_factory = true);
    prj.add_test("NativeFactory.t.sol", r#"
interface VmNativeFactory {
    function deal(address target, uint256 balance) external;
    function etch(address target, bytes calldata code) external;
    function prank(address sender, address origin) external;
    function getNonce(address target) external view returns (uint64);
    function computeCreateAddress(address deployer, uint256 nonce) external pure returns (address);
    function deployCode(string calldata artifact, uint256 value, bytes32 salt) external returns (address);
}
contract NativeFactoryChild {
    address public sender;
    address public origin;
    uint256 public value;
    constructor() payable {
        sender = msg.sender;
        origin = tx.origin;
        value = msg.value;
    }
}
contract NativeFactoryTest {
    VmNativeFactory constant vm = VmNativeFactory(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant factory = 0x4e59b44847b379578588920cA78FbF26c0B4956C;
    string constant artifact = "NativeFactory.t.sol:NativeFactoryChild";
    function check(NativeFactoryChild child, address sender, address origin, uint256 value) internal view {
        require(child.sender() == sender, "sender");
        require(child.origin() == origin, "origin");
        require(child.value() == value && address(child).balance == value, "value");
    }
    /// forge-config: default.always_use_create_2_factory = false
    function testFunctionFactoryOverride() public {
        NativeFactoryChild child = new NativeFactoryChild{salt: bytes32("direct")}();
        check(child, address(this), tx.origin, 0);
    }
    function testFactoryNewValue() public {
        vm.deal(address(this), 7);
        NativeFactoryChild child = new NativeFactoryChild{salt: bytes32("new"), value: 7}();
        check(child, factory, tx.origin, 7);
        require(address(this).balance == 0 && factory.balance == 0, "balances");
    }
    function testFactoryDeployCodeValue() public {
        vm.deal(address(this), 9);
        NativeFactoryChild child = NativeFactoryChild(vm.deployCode(artifact, 9, bytes32("artifact")));
        check(child, factory, tx.origin, 9);
        require(address(this).balance == 0 && factory.balance == 0, "balances");
    }
    function testMissingFactory() public {
        bytes memory code = factory.code;
        vm.etch(factory, "");
        try vm.deployCode(artifact, 0, bytes32("missing")) {
            revert("expected missing factory");
        } catch (bytes memory reason) {
            require(keccak256(reason) == keccak256(bytes("missing CREATE2 deployer: 0x4e59b44847b379578588920cA78FbF26c0B4956C")), "missing diagnostic");
        }
        vm.etch(factory, code);
        check(NativeFactoryChild(vm.deployCode(artifact, 0, bytes32("recovered"))), factory, tx.origin, 0);
    }
    function testInvalidFactory() public {
        vm.etch(factory, hex"00");
        try vm.deployCode(artifact, 0, bytes32("invalid")) {
            revert("expected invalid factory");
        } catch (bytes memory reason) {
            require(keccak256(reason) == keccak256(bytes("invalid CREATE2 deployer bytecode")), "invalid diagnostic");
        }
    }
    function testDirectCreatePrank() public {
        address origin = tx.origin;
        address sender = address(1234);
        address expected = vm.computeCreateAddress(sender, vm.getNonce(sender));
        vm.prank(sender, address(5678));
        NativeFactoryChild child = new NativeFactoryChild();
        require(address(child) == expected, "pranked create address");
        check(child, sender, address(5678), 0);
        require(tx.origin == origin, "origin cleanup");
        check(new NativeFactoryChild(), address(this), origin, 0);
    }
    function testSaltedCreatePrank() public {
        address origin = tx.origin;
        vm.deal(address(1234), 11);
        vm.prank(address(1234), address(5678));
        NativeFactoryChild child = new NativeFactoryChild{salt: bytes32("prank"), value: 11}();
        check(child, factory, address(5678), 11);
        require(tx.origin == origin && address(1234).balance == 0, "prank cleanup");
        check(new NativeFactoryChild(), address(this), origin, 0);
    }
}
"#);
    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 7 tests passed, 0 failed, 0 skipped (7 total tests)
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_isolation_existing_lifecycle(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.isolate = true;
        config.legacy_assertions = true;
    });
    for (path, source) in [
        (
            "Nonce.t.sol",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../testdata/default/cheats/Nonce.t.sol"
            )),
        ),
        (
            "Fee.t.sol",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../testdata/default/cheats/Fee.t.sol"
            )),
        ),
        (
            "StateSnapshots.t.sol",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../testdata/default/cheats/StateSnapshots.t.sol"
            )),
        ),
        (
            "DeployCode.t.sol",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../testdata/default/cheats/DeployCode.t.sol"
            )),
        ),
    ] {
        prj.add_test(path, source.replace("cheats/DeployCode.t.sol", "DeployCode.t.sol").as_str());
    }
    for (path, source) in [
        (
            "utils/Test.sol",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Test.sol")),
        ),
        (
            "utils/DSTest.sol",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/DSTest.sol")),
        ),
        (
            "utils/Vm.sol",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/Vm.sol")),
        ),
        (
            "utils/console.sol",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/utils/console.sol")),
        ),
    ] {
        prj.add_source(path, source);
    }
    cmd.args([
        "test", "--remappings", "utils/=src/utils/", "--fuzz-runs", "16", "--mc",
        "^(NonceIsolatedTest|IsolatedFeeTest|IsolatedFeeSnapshotRevertTest|NestedFeeSnapshotRevertTest|StateSnapshotIsolationTest|NestedRestoreFrameRevertIsolatedTest|DeployCodeTest|DeployCodeCreate2FactoryTest)$",
        "--no-match-test", "test(RawTransaction|ExecuteTransaction|DeployCodeWithRemapping)",
    ]).assert_success().stdout_eq(str![[r#"
...
Ran 8 test suites [ELAPSED]: 37 tests passed, 0 failed, 0 skipped (37 total tests)
...
"#]]);
}

#[forgetest_init]
async fn ethereum_native_fork_switching_and_persistence(prj: _, cmd: _) {
    let (first, first_handle) = anvil::spawn(
        NodeConfig::test()
            .with_chain_id(Some(31337u64))
            .with_hardfork(Some(EthereumHardfork::Cancun.into())),
    )
    .await;
    let (second, second_handle) = anvil::spawn(
        NodeConfig::test()
            .with_chain_id(Some(31338u64))
            .with_hardfork(Some(EthereumHardfork::Cancun.into())),
    )
    .await;
    let target = alloy_primitives::Address::with_last_byte(0xc0);
    let fresh = alloy_primitives::Address::with_last_byte(0xc1);
    // Both sources expose the same address and code, but different values and balances.
    let code = bytes!("36600e57600054600101600055005b60005460005260206000f3");
    for (api, value) in [(&first, 7u64), (&second, 19u64)] {
        api.anvil_set_code(target, code.clone()).await.unwrap();
        api.anvil_set_storage_at(target, U256::ZERO, U256::from(value).into()).await.unwrap();
        api.anvil_set_balance(fresh, U256::from(value)).await.unwrap();
        api.anvil_mine(Some(U256::from(2)), None).await.unwrap();
    }
    let provider =
        ProviderBuilder::new().connect_http(first_handle.http_endpoint().parse().unwrap());
    provider
        .send_transaction(TransactionRequest {
            from: Some(first_handle.dev_accounts().next().unwrap()),
            to: Some(target.into()),
            gas: Some(100_000),
            gas_price: Some(2_000_000_000),
            ..Default::default()
        })
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    prj.add_test("NativeSwitch.t.sol", &format!(r#"
import {{Test}} from "forge-std/Test.sol";
import {{Vm}} from "forge-std/Vm.sol";
interface Remote {{ function value() external view returns (uint256); }}
contract PersistentCounter {{ uint256 public value; constructor() {{ value = 41; }} function set(uint256 v) external {{ value = v; }} }}
contract ForkCheatCaller {{
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    constructor() {{ vm.warp(123); }}
    function warp() external {{ vm.warp(321); }}
}}
contract ForkCheatFactory {{
    function deploy() external returns (address) {{ return address(new ForkCheatCaller()); }}
}}
contract NativeSwitchTest is Test {{
    address constant TARGET = {target};
    address constant FRESH = {fresh};
    uint256 first;
    uint256 second;
    uint256 localValue;
    function setUp() public {{
        first = vm.createSelectFork("{}", 2);
        second = vm.createFork("{}", 2);
        localValue = 33;
        require(Remote(TARGET).value() == 7, "first source");
        vm.selectFork(second);
        require(block.chainid == 31338, "second chain");
    }}
    function testForkCheatcodePermissionIsSeparateFromPersistence() public {{
        vm.etch(FRESH, type(ForkCheatCaller).runtimeCode);
        vm.makePersistent(FRESH);
        (bool allowed, bytes memory reason) = FRESH.call(abi.encodeCall(ForkCheatCaller.warp, ()));
        require(!allowed, "remote caller obtained cheatcode access");
        assertEq(reason, abi.encodeWithSignature("CheatcodeError(string)", string.concat(
            "cheatcodes are not enabled for {fresh}; see `vm.allowCheatcodes(address)`"
        )));
        vm.allowCheatcodes(FRESH);
        ForkCheatCaller(FRESH).warp();
        require(block.timestamp == 321, "explicit permission ignored");
        vm.selectFork(first);
        ForkCheatCaller(FRESH).warp();
        require(block.timestamp == 321, "permission lost across forks");
    }}
    function testConstructorPermissionsFollowDeployer() public {{
        ForkCheatCaller trusted = new ForkCheatCaller();
        require(block.timestamp == 123, "trusted constructor denied");
        trusted.warp();
        require(block.timestamp == 321, "trusted child denied");
        vm.etch(FRESH, type(ForkCheatFactory).runtimeCode);
        (bool allowed,) = FRESH.call(abi.encodeCall(ForkCheatFactory.deploy, ()));
        require(!allowed, "remote constructor obtained cheatcode access");
        vm.allowCheatcodes(FRESH);
        address child = ForkCheatFactory(FRESH).deploy();
        require(block.timestamp == 123, "allowed constructor denied");
        ForkCheatCaller(child).warp();
        require(block.timestamp == 321, "allowed child denied");
    }}
    function testSelectedSourceSurvivesSetup() public view {{
        require(FRESH.balance == 19, "executor lost selected backing");
        require(Remote(TARGET).value() == 19, "second source");
        require(localValue == 33, "test contract did not persist");
    }}
    function testForkWritesRemainSeparate() public {{
        require(FRESH.balance == 19, "initial second backing");
        vm.store(TARGET, bytes32(0), bytes32(uint256(29)));
        vm.warp(12345);
        vm.selectFork(first);
        require(block.chainid == 31337, "first chain");
        require(FRESH.balance == 7, "first backing");
        require(Remote(TARGET).value() == 7, "writes crossed forks");
        vm.store(TARGET, bytes32(0), bytes32(uint256(11)));
        vm.selectFork(second);
        require(block.timestamp == 12345, "fork timestamp lost");
        require(Remote(TARGET).value() == 29, "second writes lost");
        vm.selectFork(first);
        require(Remote(TARGET).value() == 11, "first writes lost");
    }}
    function testExplicitPersistenceAndRevocation() public {{
        PersistentCounter counter = new PersistentCounter();
        vm.makePersistent(address(counter));
        require(vm.isPersistent(address(counter)), "persistent flag");
        vm.selectFork(first);
        require(counter.value() == 41, "persistent creation lost");
        counter.set(43);
        vm.selectFork(second);
        require(counter.value() == 43, "persistent storage lost");
        vm.revokePersistent(address(counter));
        counter.set(47);
        vm.selectFork(first);
        require(counter.value() == 43, "revoked writes crossed forks");
    }}
    function testForkRegistryDoesNotLeakBetweenTests() public {{
        require(Remote(TARGET).value() == 19, "other test changed second state");
        vm.selectFork(first);
        require(Remote(TARGET).value() == 7, "other test changed first state");
        require(block.timestamp != 12345, "other test changed environment");
    }}
    function testSwitchInsideIsolatedCall() public {{
        this.switchAndWrite();
        require(vm.activeFork() == first, "child selection lost");
        require(Remote(TARGET).value() == 23, "child write lost");
        require(FRESH.balance == 7, "parent retained old fork account");
    }}
    function switchAndWrite() external {{
        require(FRESH.balance == 19, "child initial source");
        vm.selectFork(first);
        vm.store(TARGET, bytes32(0), bytes32(uint256(23)));
    }}
    function testSnapshotRestoresForkSource() public {{
        uint256 snapshot = vm.snapshotState();
        vm.selectFork(first);
        require(Remote(TARGET).value() == 7, "first before restore");
        require(vm.revertToState(snapshot), "fork snapshot restore");
        require(vm.activeFork() == second, "snapshot fork id");
        require(block.chainid == 31338, "snapshot chain");
        require(FRESH.balance == 19, "snapshot backing");
        require(Remote(TARGET).value() == 19, "snapshot state");
    }}
    function testFailedCrossForkRestoreThenOuterRestore() public {{
        uint256 secondSnapshot = vm.snapshotState();
        vm.selectFork(first);
        uint256 firstSnapshot = vm.snapshotState();
        vm.selectFork(second);
        vm.store(TARGET, bytes32(0), bytes32(uint256(29)));
        (bool success,) = address(this).call(abi.encodeCall(this.restoreForkAndRevert, (firstSnapshot)));
        require(!success, "child did not revert");
        require(vm.revertToState(secondSnapshot), "outer snapshot restore");
        require(vm.activeFork() == second, "outer fork id");
        require(block.chainid == 31338, "outer chain");
        require(FRESH.balance == 19, "outer backing");
        require(Remote(TARGET).value() == 19, "outer storage");
        vm.selectFork(first);
        require(Remote(TARGET).value() == 7, "failed child polluted first fork");
    }}
    function restoreForkAndRevert(uint256 snapshot) external {{
        require(vm.revertToState(snapshot), "child snapshot restore");
        require(vm.activeFork() == first, "child fork id");
        vm.store(TARGET, bytes32(0), bytes32(uint256(99)));
        revert("child reverted");
    }}
    function testRollActiveAndInactiveForks() public {{
        vm.rollFork(first, 3);
        require(vm.activeFork() == second, "inactive roll switched fork");
        require(Remote(TARGET).value() == 19, "inactive roll changed state");
        vm.selectFork(first);
        require(block.number == 3, "rolled block");
        require(Remote(TARGET).value() == 8, "rolled source");
        vm.store(TARGET, bytes32(0), bytes32(uint256(99)));
        localValue = 55;
        vm.rollFork(2);
        require(block.number == 2, "active rolled block");
        require(Remote(TARGET).value() == 7, "roll retained nonpersistent writes");
        require(localValue == 55, "roll lost persistent test state");
    }}
    function testRollInsideIsolatedCallReplacesLoadedParentState() public {{
        vm.selectFork(first);
        vm.store(TARGET, bytes32(0), bytes32(uint256(99)));
        this.rollFirst();
        require(Remote(TARGET).value() == 7, "parent retained rolled state");
    }}
    function rollFirst() external {{ vm.rollFork(2); }}
}}
"#, first_handle.http_endpoint(), second_handle.http_endpoint()));
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse().args(["test", "--mc", "NativeSwitchTest"]).assert_success().stdout_eq(
            str![[r#"
...
Ran 1 test suite [ELAPSED]: 11 tests passed, 0 failed, 0 skipped (11 total tests)
...
"#]],
        );
    }
}

#[forgetest_init]
fn ethereum_native_script_deployment_setup_and_broadcast_collection(prj: _, cmd: _) {
    prj.add_script(
        "NativeScript.s.sol",
        r#"
import {Script} from "forge-std/Script.sol";
import {console} from "forge-std/console.sol";
contract NativeScriptCounter {
    uint256 public value;
    address public sender;
    constructor() { sender = msg.sender; }
    function set(uint256 v) external { value = v; }
}

contract NativeScript is Script {
    uint256 ready;
    function setUp() public { ready = 7; }
    function run() public returns (uint256) {
        require(ready == 7, "script setup missing");
        address signer = address(0x1234);
        vm.startBroadcast(signer);
        NativeScriptCounter counter = new NativeScriptCounter();
        counter.set(ready + 2);
        require(counter.sender() == signer, "constructor sender");
        require(counter.value() == 9, "broadcast state missing");
        vm.stopBroadcast();
        require(tx.origin != signer, "broadcast origin leaked");
        console.log("native script completed");
        return counter.value();
    }
}
contract NativeDefaultScript is Script {
    function run() public returns (uint256) {
        vm.startBroadcast();
        NativeScriptCounter counter = new NativeScriptCounter();
        counter.set(9);
        vm.stopBroadcast();
        console.log("native script completed");
        return counter.value();
    }
}
"#,
    );
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        for contract in [
            "script/NativeScript.s.sol:NativeScript",
            "script/NativeScript.s.sol:NativeDefaultScript",
        ] {
            cmd.forge_fuse().args(["script", contract]).assert_success().stdout_eq(str![[r#"
...
Script ran successfully.
...
== Return ==
0: uint256 9
...
== Logs ==
  native script completed
...
"#]]);
        }
    }
}

#[forgetest_init]
async fn ethereum_native_script_rpc_simulation_and_local_broadcast(prj: _, cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()))).await;
    api.anvil_set_code(
        alloy_primitives::Address::with_last_byte(0xc0),
        bytes!("5f60005260206000f3"),
    )
    .await
    .unwrap();
    prj.update_config(|config| config.evm_version = EvmVersion::London);
    let provider = ProviderBuilder::new().connect_http(handle.http_endpoint().parse().unwrap());
    let sender = handle.dev_accounts().next().unwrap();
    let target = sender.create(1);
    prj.add_script(
        "NativeRpcScript.s.sol",
        r#"
import {Script} from "forge-std/Script.sol";
contract NativeRpcCounter {
    uint256 public value;
    constructor() { value = 7; }
    function set(uint256 v) external { value = v; }
}
library NativeRpcMath {
    function twice(uint256 v) external pure returns (uint256) { return v * 2; }
}
contract NativeRpcScript is Script {
    uint256 ready;
    function setUp() public { ready = 7; }
    function run() public {
        require(ready == 7, "setup missing");
        (bool push0Allowed,) = address(0xc0).staticcall("");
        require(!push0Allowed, "RPC hardfork overrode configured London execution");
        vm.startBroadcast();
        NativeRpcCounter counter = new NativeRpcCounter();
        require(counter.value() == 7, "constructor missing");
        counter.set(NativeRpcMath.twice(ready) - 5);
        vm.stopBroadcast();
        require(counter.value() == 9, "collected calls missing");
    }
}
"#,
    );
    let rpc = handle.http_endpoint();
    let sender = sender.to_string();
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse().args(["script", "script/NativeRpcScript.s.sol:NativeRpcScript", "--rpc-url", &rpc, "--sender", &sender]).assert_success().stdout_eq(str![[r#"
...
Script ran successfully.
...
SIMULATION COMPLETE. To broadcast these transactions, add --broadcast and wallet configuration(s) to the previous command. See forge script --help for more.
...
"#]]);
        let sequence = foundry_common::fs::read_json_file::<
            forge_script_sequence::ScriptSequence<alloy_network::Ethereum>,
        >(
            &prj.root().join("broadcast/NativeRpcScript.s.sol/31337/dry-run/run-latest.json"),
        )
        .unwrap();
        assert_eq!(sequence.transactions.len(), 3);
        assert_eq!(sequence.transactions[0].transaction.nonce(), Some(0));
        assert_eq!(sequence.transactions[1].transaction.nonce(), Some(1));
        assert_eq!(sequence.transactions[2].transaction.nonce(), Some(2));
        assert_eq!(sequence.transactions[1].contract_address, Some(target));
        assert_eq!(sequence.transactions[2].transaction.to(), Some(target));
        assert!(provider.get_code_at(target).await.unwrap().is_empty());
    }
    cmd.forge_fuse()
        .args([
            "script",
            "script/NativeRpcScript.s.sol:NativeRpcScript",
            "--rpc-url",
            &rpc,
            "--sender",
            &sender,
            "--sender-nonce",
            "5",
        ])
        .assert_success();
    let sequence =
        foundry_common::fs::read_json_file::<
            forge_script_sequence::ScriptSequence<alloy_network::Ethereum>,
        >(&prj.root().join("broadcast/NativeRpcScript.s.sol/31337/dry-run/run-latest.json"))
        .unwrap();
    assert_eq!(sequence.transactions[0].transaction.nonce(), Some(5));
    assert_eq!(sequence.transactions[1].transaction.nonce(), Some(6));
    assert_eq!(sequence.transactions[2].transaction.nonce(), Some(7));
    assert_eq!(
        sequence.transactions[1].contract_address,
        Some(sender.parse::<alloy_primitives::Address>().unwrap().create(6))
    );
    cmd.forge_fuse()
        .args([
            "script",
            "script/NativeRpcScript.s.sol:NativeRpcScript",
            "--rpc-url",
            &rpc,
            "--sender",
            &sender,
            "--unlocked",
            "--broadcast",
        ])
        .assert_success();
    assert_eq!(provider.get_storage_at(target, U256::ZERO).await.unwrap(), U256::from(9));
    assert_eq!(provider.get_transaction_count(sender.parse().unwrap()).await.unwrap(), 3);
}

#[forgetest_init]
async fn ethereum_native_transaction_position_forks(prj: _, cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let provider = ProviderBuilder::new().connect_http(handle.http_endpoint().parse().unwrap());
    let sender = handle.dev_accounts().next().unwrap();
    let receipt = provider
        .send_transaction(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Create),
            input: TransactionInput::new(bytes!(
                "6007600055601a6011600039601a6000f336600e57600054600101600055005b60005460005260206000f3"
            )),
            gas: Some(200_000),
            gas_price: Some(2_000_000_000),
            ..Default::default()
        })
        .await.unwrap().get_receipt().await.unwrap();
    let target = receipt.contract_address.unwrap();
    let reverter = alloy_primitives::Address::with_last_byte(0xee);
    api.anvil_set_code(reverter, bytes!("60006000fd")).await.unwrap();
    api.anvil_set_auto_mine(false).await.unwrap();
    let mut hashes = Vec::new();
    for (nonce, to) in [(1, target), (2, reverter), (3, target)] {
        let pending = provider
            .send_transaction(TransactionRequest {
                from: Some(sender),
                to: Some(to.into()),
                nonce: Some(nonce),
                gas: Some(100_000),
                gas_price: Some(2_000_000_000),
                ..Default::default()
            })
            .await
            .unwrap();
        hashes.push(*pending.tx_hash());
    }
    api.anvil_mine(Some(U256::ONE), None).await.unwrap();
    api.anvil_set_auto_mine(true).await.unwrap();
    let block =
        provider.get_transaction_receipt(hashes[2]).await.unwrap().unwrap().block_number.unwrap();
    assert!(!provider.get_transaction_receipt(hashes[1]).await.unwrap().unwrap().status());
    prj.add_test("NativeTransactionFork.t.sol", &format!(r#"
import {{Test}} from "forge-std/Test.sol";
interface Remote {{ function value() external view returns (uint256); }}
contract NativeTransactionForkTest is Test {{
    address constant TARGET = {target};
    address constant SENDER = {sender};
    function testCreateAndSelectBeforeTarget() public {{
        uint256 first = vm.createFork("{rpc}", bytes32({first}));
        uint256 last = vm.createSelectFork("{rpc}", bytes32({last}));
        require(block.number == {block}, "target block");
        require(Remote(TARGET).value() == 8, "prefix or target boundary");
        require(vm.getNonce(SENDER) == 3, "reverted prefix nonce missing");
        vm.selectFork(first);
        require(Remote(TARGET).value() == 7, "first transaction was executed");
        require(vm.getNonce(SENDER) == 1, "first prefix nonce");
        vm.selectFork(last);
        require(Remote(TARGET).value() == 8, "saved prefix lost");
    }}
    function testActiveAndInactiveTransactionRoll() public {{
        uint256 first = vm.createSelectFork("{rpc}", bytes32({first}));
        uint256 second = vm.createFork("{rpc}", bytes32({first}));
        vm.rollFork(second, bytes32({last}));
        require(vm.activeFork() == first, "inactive roll selected source");
        require(Remote(TARGET).value() == 7, "inactive roll changed active state");
        vm.selectFork(second);
        require(Remote(TARGET).value() == 8, "inactive prefix missing");
        vm.rollFork(bytes32({first}));
        require(Remote(TARGET).value() == 7, "active roll retained prefix");
        this.rollLast();
        require(Remote(TARGET).value() == 8, "nested roll retained parent state");
        require(vm.getNonce(SENDER) == 3, "nested prefix nonce");
        (bool ok,) = address(vm).call(abi.encodeWithSignature("rollFork(bytes32)", bytes32(type(uint256).max)));
        require(!ok, "missing transaction accepted");
        require(Remote(TARGET).value() == 8, "failed roll changed state");
    }}
    function testTransactUsesCurrentForkStateAndRetainsOuterContext() public {{
        uint256 first = vm.createSelectFork("{rpc}", bytes32({first}));
        uint256 second = vm.createFork("{rpc}", bytes32({first}));
        uint256 timestamp = block.timestamp;
        address origin = tx.origin;
        vm.transact(second, bytes32({first}));
        require(vm.activeFork() == first, "inactive replay selected fork");
        require(Remote(TARGET).value() == 7, "inactive replay changed active state");
        require(vm.getNonce(SENDER) == 1, "inactive replay changed sender");
        vm.selectFork(second);
        require(Remote(TARGET).value() == 8, "inactive replay missing");
        require(vm.getNonce(SENDER) == 2, "inactive replay nonce missing");
        vm.transact(bytes32({reverted}));
        require(Remote(TARGET).value() == 8, "reverted replay changed storage");
        require(vm.getNonce(SENDER) == 3, "reverted replay nonce missing");
        this.replayLast();
        require(Remote(TARGET).value() == 9, "nested replay lost state");
        require(vm.getNonce(SENDER) == 4, "nested replay lost nonce");
        require(block.timestamp == timestamp, "replay environment leaked");
        require(tx.origin == origin, "replay origin leaked");
        (bool ok,) = address(vm).call(abi.encodeWithSignature("transact(bytes32)", bytes32(type(uint256).max)));
        require(!ok, "missing replay accepted");
        require(Remote(TARGET).value() == 9, "failed replay changed state");
    }}
    function replayLast() external {{ vm.transact(bytes32({last})); }}
    function rollLast() external {{ vm.rollFork(bytes32({last})); }}
}}
"#, rpc=handle.http_endpoint(), first=hashes[0], last=hashes[2], reverted=hashes[1]));
    for (isolate, custom_sender) in [(false, false), (true, false), (false, true), (true, true)] {
        prj.update_config(|config| config.isolate = isolate);
        let command = cmd.forge_fuse();
        command.args(["test", "--mc", "NativeTransactionForkTest"]);
        if custom_sender {
            command.args(["--sender", &sender.to_string()]);
        }
        command.assert_success().stdout_eq(str![[r#"
...
Ran 1 test suite [ELAPSED]: 3 tests passed, 0 failed, 0 skipped (3 total tests)
...
"#]]);
    }
    assert_eq!(provider.get_storage_at(target, U256::ZERO).await.unwrap(), U256::from(9));
    assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 4);
}

#[forgetest_init]
async fn ethereum_native_script_multi_rpc_value_and_resume(prj: _, cmd: _) {
    let (first, first_handle) = anvil::spawn(
        NodeConfig::test()
            .with_chain_id(Some(31337u64))
            .with_hardfork(Some(EthereumHardfork::Cancun.into())),
    )
    .await;
    let (second, second_handle) = anvil::spawn(
        NodeConfig::test()
            .with_chain_id(Some(31338u64))
            .with_hardfork(Some(EthereumHardfork::Cancun.into())),
    )
    .await;
    let sender = first_handle.dev_accounts().next().unwrap();
    second.anvil_set_nonce(sender, U256::from(5)).await.unwrap();
    first.mine_one().await.unwrap();
    second.mine_one().await.unwrap();
    let first_rpc = first_handle.http_endpoint();
    let second_rpc = second_handle.http_endpoint();
    let first_provider = ProviderBuilder::new().connect_http(first_rpc.parse().unwrap());
    let second_provider = ProviderBuilder::new().connect_http(second_rpc.parse().unwrap());
    let first_counter = sender.create(0);
    let second_counter = sender.create(5);
    prj.add_script("NativeMultiRpc.s.sol", &format!(r#"
import {{Script}} from "forge-std/Script.sol";
contract MultiRpcCounter {{
    uint256 public value;
    constructor() payable {{ require(msg.value == 11, "deployment value"); }}
    function set(uint256 v) external payable {{ require(msg.value == 13, "call value"); value = v; }}
}}
contract NativeMultiRpcScript is Script {{
    function run() public {{
        uint256 first = vm.createSelectFork("{first_rpc}");
        require(block.chainid == 31337, "first chain");
        vm.startBroadcast();
        MultiRpcCounter a = new MultiRpcCounter{{value: 11}}();
        a.set{{value: 13}}(7);
        vm.stopBroadcast();
        vm.makePersistent(address(a));
        vm.createSelectFork("{second_rpc}");
        require(block.chainid == 31338, "second chain");
        vm.startBroadcast();
        MultiRpcCounter b = new MultiRpcCounter{{value: 11}}();
        b.set{{value: 13}}(29);
        vm.stopBroadcast();
        require(a != b, "fork sender nonce lost");
        vm.selectFork(first);
        require(block.chainid == 31337, "return chain");
        require(a.value() == 7, "first state lost");
        vm.startBroadcast();
        a.set{{value: 13}}(17);
        vm.stopBroadcast();
        require(a.value() == 17, "first final state");
    }}
}}
"#));
    let sender_arg = sender.to_string();
    let common = [
        "script",
        "NativeMultiRpcScript",
        "--rpc-url",
        first_rpc.as_str(),
        "--sender",
        sender_arg.as_str(),
        "--unlocked",
        "--non-interactive",
    ];
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse().args(common).assert_success();
        assert!(first_provider.get_code_at(first_counter).await.unwrap().is_empty());
        assert!(second_provider.get_code_at(second_counter).await.unwrap().is_empty());
        assert_eq!(first_provider.get_transaction_count(sender).await.unwrap(), 0);
        assert_eq!(second_provider.get_transaction_count(sender).await.unwrap(), 5);
    }
    cmd.forge_fuse().args(common).arg("--broadcast").assert_success();
    for (provider, counter, value, balance, nonce) in [
        (&first_provider, first_counter, 17u64, 37u64, 3u64),
        (&second_provider, second_counter, 29u64, 24u64, 7u64),
    ] {
        assert_eq!(provider.get_storage_at(counter, U256::ZERO).await.unwrap(), U256::from(value));
        assert_eq!(provider.get_balance(counter).await.unwrap(), U256::from(balance));
        assert_eq!(provider.get_transaction_count(sender).await.unwrap(), nonce);
    }
    // Resume consumes the published sequence rather than executing the script again.
    cmd.forge_fuse().args(common).args(["--broadcast", "--resume", "--multi"]).assert_success();
    assert_eq!(first_provider.get_transaction_count(sender).await.unwrap(), 3);
    assert_eq!(second_provider.get_transaction_count(sender).await.unwrap(), 7);
}

#[forgetest_init]
async fn ethereum_native_transact_enforces_fork_permissions(prj: _, cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let provider = ProviderBuilder::new().connect_http(handle.http_endpoint().parse().unwrap());
    let sender = handle.dev_accounts().next().unwrap();
    let target = alloy_primitives::Address::with_last_byte(0xed);
    // Calls warp(123), bubbles a denied call, and writes slot zero only after success.
    let mut code = vec![0x63];
    code.extend_from_slice(&alloy_primitives::keccak256(b"warp(uint256)")[..4]);
    code.extend_from_slice(&[0x60, 0xe0, 0x1b, 0x60, 0, 0x52, 0x60, 123, 0x60, 4, 0x52]);
    code.extend_from_slice(&[0x60, 0, 0x60, 0, 0x60, 36, 0x60, 0, 0x60, 0, 0x73]);
    code.extend_from_slice(foundry_evm::core::constants::CHEATCODE_ADDRESS.as_slice());
    code.extend_from_slice(&[0x5a, 0xf1, 0x60, 0, 0x57]);
    let jump_operand = code.len() - 2;
    code.extend_from_slice(&[0x3d, 0x60, 0, 0x60, 0, 0x3e, 0x3d, 0x60, 0, 0xfd]);
    code[jump_operand] = u8::try_from(code.len()).unwrap();
    code.extend_from_slice(&[0x5b, 0x60, 42, 0x60, 0, 0x55, 0]);
    api.anvil_set_code(target, code.clone().into()).await.unwrap();
    let receipt = provider
        .send_transaction(TransactionRequest {
            from: Some(sender),
            to: Some(target.into()),
            gas: Some(100_000),
            gas_price: Some(2_000_000_000),
            ..Default::default()
        })
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    code.pop();
    code.extend_from_slice(&[0x60, 1, 0x60, 0, 0xf3]);
    let creation = provider
        .send_transaction(TransactionRequest {
            from: Some(sender),
            to: Some(TxKind::Create),
            gas: Some(200_000),
            input: TransactionInput::new(code.into()),
            gas_price: Some(2_000_000_000),
            ..Default::default()
        })
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let created = creation.contract_address.unwrap();
    prj.add_test(
        "NativeReplayPermissions.t.sol",
        &format!(
            r#"
import {{Test}} from "forge-std/Test.sol";
contract NativeReplayPermissionsTest is Test {{
    function testReplayCannotAuthorizeItsRemoteCaller() public {{
        vm.createSelectFork("{rpc}", bytes32({hash}));
        uint256 timestamp = block.timestamp;
        vm.transact(bytes32({hash}));
        require(vm.load({target}, bytes32(0)) == bytes32(0), "replay bypassed permissions");
        require(vm.getNonce({sender}) == 1, "denied transaction nonce missing");
        vm.allowCheatcodes({target});
        this.replay();
        require(vm.load({target}, bytes32(0)) == bytes32(uint256(42)), "authorized replay denied");
        require(vm.getNonce({sender}) == 2, "nested replay nonce missing");
        require(block.timestamp == timestamp, "replay warp leaked to outer call");
    }}
    function testReplayConstructorDoesNotGainTopLevelPrivilege() public {{
        vm.createSelectFork("{rpc}", bytes32({creation_hash}));
        vm.transact(bytes32({creation_hash}));
        require({created}.code.length == 0, "remote constructor gained access");
        require(vm.load({created}, bytes32(0)) == bytes32(0), "denied constructor wrote state");
        require(vm.getNonce({sender}) == 2, "denied creation nonce missing");
        vm.allowCheatcodes({sender});
        vm.transact(bytes32({creation_hash}));
        require({created}.code.length == 1, "authorized constructor denied");
        require(vm.load({created}, bytes32(0)) == bytes32(uint256(42)), "authorized constructor state missing");
        require(vm.getNonce({sender}) == 3, "authorized creation nonce missing");
    }}
    function replay() external {{ vm.transact(bytes32({hash})); }}
}}
"#,
            rpc = handle.http_endpoint(),
            hash = receipt.transaction_hash,
            creation_hash = creation.transaction_hash
        ),
    );
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse().args(["test", "--mc", "NativeReplayPermissionsTest"]).assert_success();
    }
    assert_eq!(provider.get_storage_at(target, U256::ZERO).await.unwrap(), U256::from(42));
    assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 2);
    assert_eq!(provider.get_storage_at(created, U256::ZERO).await.unwrap(), U256::from(42));
}

#[forgetest_init]
fn ethereum_native_invariant_targeting_and_run_lifecycle(prj: _, cmd: _) {
    prj.add_test("NativeInvariant.t.sol", r#"
import {Test} from "forge-std/Test.sol";
contract NativeInvariantHandler {
    uint256 public count;
    address immutable sender;
    constructor(address allowed) { sender = allowed; }
    function step() public {
        require(msg.sender == sender, "sender selection ignored");
        require(count < 8, "state leaked across runs");
        count++;
    }
    function forbidden() public pure { revert("excluded selector called"); }
}
contract NativeInvariantCampaignTest is Test {
    NativeInvariantHandler handler;
    function setUp() public {
        handler = new NativeInvariantHandler(address(0x123));
        targetContract(address(handler));
        targetSender(address(0x123));
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = NativeInvariantHandler.step.selector;
        targetSelector(FuzzSelector(address(handler), selectors));
    }
    function invariant_countSafe() public view { require(handler.count() <= 8, "count invalid"); }
    function afterInvariant() public view { require(handler.count() == 8, "state did not persist across calls"); }
}
contract NativeInvariantExcludedSelectorTest is Test {
    NativeInvariantHandler handler;
    function setUp() public {
        handler = new NativeInvariantHandler(address(0x123));
        targetContract(address(handler));
        targetSender(address(0x123));
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = NativeInvariantHandler.forbidden.selector;
        excludeSelector(FuzzSelector(address(handler), selectors));
    }
    function invariant_countSafe() public view { require(handler.count() <= 8, "count invalid"); }
    function afterInvariant() public view { require(handler.count() == 8, "excluded selector was called"); }
}
"#);
    prj.update_config(|config| {
        config.invariant.runs = 4;
        config.invariant.depth = 8;
        config.invariant.fail_on_revert = true;
    });
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeInvariant.*Test"])
            .assert_success()
            .stdout_eq(str![[r#"
...
[PASS] invariant_countSafe() (runs: 4, calls: 32, reverts: 0)
...
[PASS] invariant_countSafe() (runs: 4, calls: 32, reverts: 0)
...
Ran 2 test suites [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)
...
"#]]);
    }
}

#[forgetest_init]
fn ethereum_native_invariant_failure_replay_and_minimization(prj: _, cmd: _) {
    let source = r#"
import {Test} from "forge-std/Test.sol";
contract NativeFailingHandler {
    uint256 public count;
    function step() public { count++; }
}
contract NativeFailingInvariantTest is Test {
    NativeFailingHandler handler;
    function setUp() public {
        handler = new NativeFailingHandler();
        targetContract(address(handler));
    }
    function invariant_countSafe() public view { require(handler.count() < 2, "counter invalid"); }
}
"#;
    prj.add_test("NativeFailingInvariant.t.sol", source);
    let root = prj.root().join("native-failures");
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 8;
        config.invariant.check_interval = 8;
        config.invariant.failure_persist_dir = Some(root.clone());
    });
    cmd.args(["test", "--mc", "NativeFailingInvariantTest"]).assert_failure().stdout_eq(str![[
        r#"
...
[FAIL: counter invalid]
	[Sequence] (original: 8, shrunk: 2)
...
 invariant_countSafe() (runs: 1, calls: 8, reverts: 0)
...
"#
    ]]);
    let path = root.join("failures/NativeFailingInvariantTest/invariants/invariant_countSafe");
    let failure =
        foundry_common::fs::read_json_file::<NativeInvariantFailureRecord>(&path).unwrap();
    assert_eq!(failure.call_sequence.len(), 2);
    assert!(!failure.assertion_failure);
    for call in &failure.call_sequence {
        assert_eq!(call.signature.as_deref(), Some("step()"));
    }
    // Zero new runs makes a second failure evidence of concrete persisted replay.
    prj.update_config(|config| config.invariant.runs = 0);
    cmd.forge_fuse()
        .args(["test", "--mc", "NativeFailingInvariantTest"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: counter invalid]
	[Sequence] (original: 2, shrunk: 2)
...
 invariant_countSafe() (runs: 0, calls: 0, reverts: 0, failed corpus replays: 1)
...
"#]]);
    let replayed =
        foundry_common::fs::read_json_file::<NativeInvariantFailureRecord>(&path).unwrap();
    assert_eq!(replayed.call_sequence.len(), 2);
    // A fixed predicate must retire the old failing replay and execute a fresh campaign.
    prj.add_test(
        "NativeFailingInvariant.t.sol",
        &source.replace("handler.count() < 2", "handler.count() < 20"),
    );
    prj.update_config(|config| config.invariant.runs = 2);
    cmd.forge_fuse()
        .args(["test", "--mc", "NativeFailingInvariantTest"])
        .assert_success()
        .stdout_eq(str![[r#"
...
[PASS] invariant_countSafe() (runs: 2, calls: 16, reverts: 0)
...
"#]]);
}

#[forgetest_init]
fn ethereum_native_invariant_assertion_and_revert_policy(prj: _, cmd: _) {
    prj.add_test("NativeInvariantAssertions.t.sol", r#"
import {Test} from "forge-std/Test.sol";
contract NativeAssertionHandler {
    uint256 count;
    function step() public { count++; assert(false); }
}
contract NativeRevertingHandler {
    uint256 count;
    function step() public { count++; revert("ordinary revert"); }
}
contract NativePredicateAssertionTest is Test {
    NativeRevertingHandler handler;
    function setUp() public { handler = new NativeRevertingHandler(); targetContract(address(handler)); }
    function invariant_assertion() public { assertEq(uint256(1), uint256(2)); }
}
contract NativeHandlerAssertionTest is Test {
    function setUp() public { targetContract(address(new NativeAssertionHandler())); }
    function invariant_safe() public pure {}
}
contract NativeRevertPolicyTest is Test {
    function setUp() public { targetContract(address(new NativeRevertingHandler())); }
    function invariant_safe() public pure {}
}
"#);
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 4;
        config.invariant.fail_on_revert = false;
        config.assertions_revert = false;
    });
    cmd.args(["test", "--mc", "NativePredicateAssertionTest"]).assert_failure().stdout_eq(str![[
        r#"
...
[FAIL: assertion failed] invariant_assertion() (runs: 0, calls: 0, reverts: 0)
...
"#
    ]]);
    cmd.forge_fuse()
        .args(["test", "--mc", "NativeHandlerAssertionTest"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
[FAIL: panic: assertion failed (0x01)]
	[Sequence] (original: 1, shrunk: 1)
...
 invariant_safe() (runs: 1, calls: 1, reverts: 1)
...
"#]]);
    cmd.forge_fuse().args(["test", "--mc", "NativeRevertPolicyTest"]).assert_success().stdout_eq(
        str![[r#"
...
[PASS] invariant_safe() (runs: 1, calls: 4, reverts: 4)
...
"#]],
    );
    prj.update_config(|config| config.invariant.fail_on_revert = true);
    cmd.forge_fuse().args(["test", "--mc", "NativeRevertPolicyTest"]).assert_failure().stdout_eq(
        str![[r#"
...
[FAIL: ordinary revert]
	[Sequence] (original: 1, shrunk: 1)
...
 invariant_safe() (runs: 1, calls: 1, reverts: 1)
...
"#]],
    );
}

#[forgetest_init]
fn ethereum_native_invariant_after_hook_replay(prj: _, cmd: _) {
    prj.add_test("NativeAfterInvariant.t.sol", r#"
import {Test} from "forge-std/Test.sol";
contract NativeAfterHandler {
    uint256 public count;
    function step() public { count++; }
}
contract NativeAfterInvariantTest is Test {
    NativeAfterHandler handler;
    function setUp() public { handler = new NativeAfterHandler(); targetContract(address(handler)); }
    function invariant_safe() public view { require(handler.count() <= 8, "predicate changed"); }
    function afterInvariant() public view { require(handler.count() < 2, "after invalid"); }
}
"#);
    let root = prj.root().join("native-after-failures");
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 8;
        config.invariant.failure_persist_dir = Some(root.clone());
    });
    cmd.args(["test", "--mc", "NativeAfterInvariantTest"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: after invalid]
	[Sequence] (original: 8, shrunk: 2)
...
 invariant_safe() (runs: 1, calls: 8, reverts: 0)
...
"#]]);
    let path = root.join("failures/NativeAfterInvariantTest/invariants/invariant_safe");
    let failure =
        foundry_common::fs::read_json_file::<NativeInvariantFailureRecord>(&path).unwrap();
    assert_eq!(failure.call_sequence.len(), 2);
    prj.update_config(|config| config.invariant.runs = 0);
    cmd.forge_fuse().args(["test", "--mc", "NativeAfterInvariantTest"]).assert_failure().stdout_eq(
        str![[r#"
...
[FAIL: after invalid]
	[Sequence] (original: 2, shrunk: 2)
...
 invariant_safe() (runs: 0, calls: 0, reverts: 0, failed corpus replays: 1)
...
"#]],
    );
    // Changing replay policy invalidates the stored failure instead of replaying stale settings.
    prj.update_config(|config| config.invariant.fail_on_revert = true);
    cmd.forge_fuse().args(["test", "--mc", "NativeAfterInvariantTest"]).assert_success().stdout_eq(
        str![[r#"
...
[PASS] invariant_safe() (runs: 0, calls: 0, reverts: 0)
...
"#]],
    );
}

#[forgetest_init]
async fn ethereum_native_script_signed_authorizations(prj: _, cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Prague.into()))).await;
    let provider = ProviderBuilder::new().connect_http(handle.http_endpoint().parse().unwrap());
    let sender = handle.dev_accounts().next().unwrap();
    let implementation = sender.create(0);
    // Public Anvil development key, scoped to the disposable node.
    let private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    prj.add_script(
        "NativeAuthorization.s.sol",
        r#"
import {Script} from "forge-std/Script.sol";
contract NativeDelegatedCounter {
    uint256 public count;
    function step() external payable { require(msg.value == 13, "delegated call value"); count++; }
}
contract NativeAuthorizationScript is Script {
    uint256 constant KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    function run() public {
        vm.startBroadcast(KEY);
        NativeDelegatedCounter implementation = new NativeDelegatedCounter();
        address authority = vm.addr(KEY);
        vm.signAndAttachDelegation(address(implementation), KEY);
        NativeDelegatedCounter(authority).step{value: 13}();
        require(vm.getNonce(authority) == 3, "authorization nonce");
        NativeDelegatedCounter(authority).step{value: 13}();
        require(NativeDelegatedCounter(authority).count() == 2, "delegated state");
        require(vm.getNonce(authority) == 4, "delegation consumed twice");
        vm.stopBroadcast();
    }
}
"#,
    );
    let rpc = handle.http_endpoint();
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse()
            .args([
                "script",
                "NativeAuthorizationScript",
                "--rpc-url",
                &rpc,
                "--evm-version",
                "prague",
                "--non-interactive",
            ])
            .assert_success();
        assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 0);
        assert!(provider.get_code_at(implementation).await.unwrap().is_empty());
    }
    cmd.forge_fuse()
        .args([
            "script",
            "NativeAuthorizationScript",
            "--rpc-url",
            &rpc,
            "--evm-version",
            "prague",
            "--non-interactive",
            "--broadcast",
            "--slow",
        ])
        .assert_success();
    assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 4);
    assert_eq!(provider.get_storage_at(sender, U256::ZERO).await.unwrap(), U256::from(2));
    assert_eq!(
        provider.get_code_at(sender).await.unwrap(),
        evm2::bytecode::Bytecode::new_eip7702(implementation).original_bytes()
    );
    // Completed signed resume must not re-run or re-sign the original sequence.
    cmd.forge_fuse()
        .args([
            "script",
            "NativeAuthorizationScript",
            "--rpc-url",
            &rpc,
            "--evm-version",
            "prague",
            "--non-interactive",
            "--broadcast",
            "--resume",
            "--private-key",
            private_key,
        ])
        .assert_success();
    assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 4);
    api.anvil_set_auto_mine(true).await.unwrap();
}

#[forgetest_init]
async fn ethereum_native_script_fixed_gas_and_value(prj: _, cmd: _) {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Cancun.into()))).await;
    let provider = ProviderBuilder::new().connect_http(handle.http_endpoint().parse().unwrap());
    let sender = handle.dev_accounts().next().unwrap();
    let target = sender.create(0);
    prj.add_script(
        "NativeFixedGas.s.sol",
        r#"
import {Script} from "forge-std/Script.sol";
contract NativeGasTarget {
    uint256 public receivedGas;
    uint256 public value;
    function record(uint256 v) external payable { receivedGas = gasleft(); value = v; }
}
contract NativeFixedGasScript is Script {
    function run() public {
        vm.startBroadcast();
        NativeGasTarget target = new NativeGasTarget();
        target.record{gas: 100000, value: 13}(42);
        vm.stopBroadcast();
    }
}
"#,
    );
    let rpc = handle.http_endpoint();
    let sender_arg = sender.to_string();
    let args = [
        "script",
        "NativeFixedGasScript",
        "--rpc-url",
        &rpc,
        "--sender",
        &sender_arg,
        "--unlocked",
        "--non-interactive",
    ];
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse().args(args).assert_success();
        let path = prj.root().join("broadcast/NativeFixedGas.s.sol/31337/dry-run/run-latest.json");
        let sequence =
            foundry_common::fs::read_json_file::<ScriptSequence<Ethereum>>(&path).unwrap();
        assert!(!sequence.transactions[0].is_fixed_gas_limit);
        assert!(sequence.transactions[1].is_fixed_gas_limit);
        assert_eq!(sequence.transactions[1].tx().gas(), Some(102300));
        assert_eq!(sequence.transactions[1].tx().value(), Some(U256::from(13)));
        assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 0);
    }
    cmd.forge_fuse().args(args).arg("--broadcast").assert_success();
    assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 2);
    assert_eq!(provider.get_balance(target).await.unwrap(), U256::from(13));
    assert_eq!(provider.get_storage_at(target, U256::ONE).await.unwrap(), U256::from(42));
    let received = provider.get_storage_at(target, U256::ZERO).await.unwrap();
    assert!(received > U256::from(75000) && received < U256::from(85000));
    // Repeat through a locally signed legacy envelope, preserving the same value/gas intent.
    cmd.forge_fuse()
        .args([
            "script",
            "NativeFixedGasScript",
            "--rpc-url",
            &rpc,
            "--sender",
            &sender_arg,
            "--non-interactive",
            "--broadcast",
            "--legacy",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        ])
        .assert_success();
    let signed_target = sender.create(2);
    assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 4);
    assert_eq!(provider.get_balance(signed_target).await.unwrap(), U256::from(13));
    assert_eq!(provider.get_storage_at(signed_target, U256::ONE).await.unwrap(), U256::from(42));
    let sequence = foundry_common::fs::read_json_file::<ScriptSequence<Ethereum>>(
        &prj.root().join("broadcast/NativeFixedGas.s.sol/31337/run-latest.json"),
    )
    .unwrap();
    assert!(sequence.transactions[1].is_fixed_gas_limit);
    assert_eq!(sequence.transactions[1].tx().gas(), Some(102300));
}

#[forgetest_init]
fn ethereum_native_invariant_runtime_output_dictionary(prj: _, cmd: _) {
    prj.add_test("NativeRuntimeDictionary.t.sol", r#"
import {Test} from "forge-std/Test.sol";
contract NativeReturnDictionaryHandler {
    uint256 secret;
    bool public matched;
    function prime() external returns (uint256) {
        secret = uint256(keccak256(abi.encode(block.timestamp, address(this))));
        return secret;
    }
    function consume(uint256 value) external { if (secret != 0 && value == secret) matched = true; }
}
contract NativeLogDictionaryHandler {
    event Secret(uint256 value);
    uint256 secret;
    bool public matched;
    function prime() external {
        secret = uint256(keccak256(abi.encode(block.timestamp, address(this))));
        emit Secret(secret);
    }
    function consume(uint256 value) external { if (secret != 0 && value == secret) matched = true; }
}
contract NativeReturnDictionaryTest is Test {
    NativeReturnDictionaryHandler handler;
    function setUp() public { handler = new NativeReturnDictionaryHandler(); targetContract(address(handler)); }
    function invariant_safe() public pure {}
    function afterInvariant() public view { require(handler.matched(), "return dictionary missing"); }
}
contract NativeLogDictionaryTest is Test {
    NativeLogDictionaryHandler handler;
    function setUp() public { handler = new NativeLogDictionaryHandler(); targetContract(address(handler)); }
    function invariant_safe() public pure {}
    function afterInvariant() public view { require(handler.matched(), "event dictionary missing"); }
}
"#);
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(42));
        config.invariant.runs = 2;
        config.invariant.depth = 128;
        config.invariant.dictionary.dictionary_weight = 100;
        config.invariant.dictionary.include_storage = false;
        config.invariant.dictionary.include_push_bytes = false;
    });
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse()
            .args(["test", "--mc", "Native(Return|Log)DictionaryTest"])
            .assert_success();
    }
}

#[forgetest_init]
async fn ethereum_native_invariant_fork_run_lifecycle(prj: _, cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let remote = alloy_primitives::Address::with_last_byte(0x91);
    api.anvil_set_storage_at(remote, U256::ZERO, alloy_primitives::B256::from(U256::from(11)))
        .await
        .unwrap();
    api.anvil_set_code(remote, bytes!("00")).await.unwrap();
    api.mine_one().await.unwrap();
    let provider = ProviderBuilder::new().connect_http(handle.http_endpoint().parse().unwrap());
    prj.add_test(
        "NativeForkInvariant.t.sol",
        &format!(
            r#"
import {{Test}} from "forge-std/Test.sol";
import {{Vm}} from "forge-std/Vm.sol";
contract NativeForkInvariantHandler {{
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant REMOTE = {remote};
    uint256 public count;
    function step() external {{
        require(count < 8, "handler state leaked across runs");
        require(block.timestamp == 100 + count, "fork environment lost");
        require(uint256(vm.load(REMOTE, bytes32(0))) == 11 + count, "fork storage lost");
        count++;
        vm.store(REMOTE, bytes32(0), bytes32(11 + count));
        vm.warp(100 + count);
    }}
}}
contract NativeForkInvariantTest is Test {{
    NativeForkInvariantHandler handler;
    function setUp() public {{
        vm.createSelectFork("{rpc}");
        vm.warp(100);
        handler = new NativeForkInvariantHandler();
        targetContract(address(handler));
    }}
    function invariant_safe() public view {{ require(handler.count() <= 8, "count invalid"); }}
    function afterInvariant() public view {{
        require(handler.count() == 8, "calls did not persist");
        require(block.timestamp == 108, "warp did not persist");
        require(uint256(vm.load({remote}, bytes32(0))) == 19, "remote overlay did not persist");
    }}
}}
"#,
            rpc = handle.http_endpoint()
        ),
    );
    prj.update_config(|config| {
        config.invariant.runs = 3;
        config.invariant.depth = 8;
        config.invariant.fail_on_revert = true;
    });
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeForkInvariantTest"])
            .assert_success()
            .stdout_eq(str![[r#"
...
[PASS] invariant_safe() (runs: 3, calls: 24, reverts: 0)
...
"#]]);
        assert_eq!(provider.get_storage_at(remote, U256::ZERO).await.unwrap(), U256::from(11));
    }
}

#[forgetest_init]
fn ethereum_native_invariant_runtime_state_dictionary(prj: _, cmd: _) {
    prj.add_test("NativeStateDictionary.t.sol", r#"
import {Test} from "forge-std/Test.sol";
contract NativeStorageDictionaryHandler {
    uint256 secret;
    bool public matched;
    function prime() external { secret = uint256(keccak256(abi.encode(block.timestamp, address(this)))); }
    function consume(uint256 value) external { if (secret != 0 && value == secret) matched = true; }
}
contract NativeBytecodeDictionaryHandler {
    uint256 secret;
    bool public matched;
    address cached;
    function prime() external {
        if (cached != address(0)) return;
        secret = uint256(keccak256(abi.encode(block.timestamp, address(this))));
        bytes memory initcode = abi.encodePacked(hex"6022600c60003960226000f3", hex"7f", bytes32(secret), hex"00");
        address deployed;
        assembly { deployed := create(0, add(initcode, 32), mload(initcode)) }
        require(deployed.code.length == 34, "runtime deployment failed");
        cached = deployed;
    }
    function consume(uint256[32] calldata values) external {
        for (uint256 i; i < values.length; ++i) {
            if (secret != 0 && values[i] == secret) matched = true;
        }
    }
}
contract NativeStorageDictionaryTest is Test {
    NativeStorageDictionaryHandler handler;
    function setUp() public { handler = new NativeStorageDictionaryHandler(); targetContract(address(handler)); }
    function invariant_safe() public pure {}
    function afterInvariant() public view { require(handler.matched(), "storage dictionary missing"); }
}
contract NativeBytecodeDictionaryTest is Test {
    NativeBytecodeDictionaryHandler handler;
    function setUp() public { handler = new NativeBytecodeDictionaryHandler(); targetContract(address(handler)); }
    function invariant_safe() public pure {}
    function afterInvariant() public view { require(handler.matched(), "runtime bytecode dictionary missing"); }
}
"#);
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(42));
        config.invariant.runs = 2;
        config.invariant.depth = 1024;
        config.invariant.dictionary.dictionary_weight = 100;
    });
    for isolate in [false, true] {
        for (contract, storage) in
            [("NativeStorageDictionaryTest", true), ("NativeBytecodeDictionaryTest", false)]
        {
            prj.update_config(|config| {
                config.isolate = isolate;
                config.invariant.dictionary.include_storage = storage;
                config.invariant.dictionary.include_push_bytes = !storage;
            });
            cmd.forge_fuse().args(["test", "--mc", contract]).assert_success();
        }
    }
}

#[forgetest_init]
async fn ethereum_native_fork_inactive_account_finalization(prj: _, cmd: _) {
    let (api, handle) =
        anvil::spawn(NodeConfig::test().with_hardfork(Some(EthereumHardfork::Berlin.into()))).await;
    let victim = alloy_primitives::Address::with_last_byte(0x91);
    let beneficiary = alloy_primitives::Address::with_last_byte(0x92);
    let mut code = vec![0x73];
    code.extend_from_slice(beneficiary.as_slice());
    code.push(0xff);
    api.anvil_set_code(victim, code.into()).await.unwrap();
    api.anvil_set_nonce(victim, U256::from(3)).await.unwrap();
    api.anvil_set_balance(victim, U256::from(17)).await.unwrap();
    api.anvil_set_storage_at(victim, U256::ZERO, U256::from(5).into()).await.unwrap();
    api.anvil_mine(Some(U256::ONE), None).await.unwrap();
    prj.add_test(
        "NativeForkFinalization.t.sol",
        &format!(r#"
import {{Test}} from "forge-std/Test.sol";
contract NativeForkFinalizationTest is Test {{
    address constant VICTIM = address(0x91);
    address constant BENEFICIARY = address(0x92);
    address constant EMPTY = address(0x93);
    uint256 first;
    uint256 second;
    function setUp() public {{
        first = vm.createSelectFork("{rpc}");
        require(VICTIM.code.length == 22, "missing remote victim");
        (bool success,) = VICTIM.call("");
        require(success, "selfdestruct failed");
        vm.etch(EMPTY, hex"00");
        vm.etch(EMPTY, hex"");
        vm.deal(EMPTY, 0);
        second = vm.createSelectFork("{rpc}");
    }}
    function testInactiveForkFinalizesLifetime() public {{
        vm.selectFork(first);
        require(VICTIM.code.length == 0, "inactive selfdestruct retained code");
        require(vm.getNonce(VICTIM) == 0, "inactive selfdestruct retained nonce");
        require(vm.load(VICTIM, bytes32(0)) == bytes32(0), "inactive selfdestruct retained storage");
        require(VICTIM.balance == 0, "inactive selfdestruct retained balance");
        require(BENEFICIARY.balance == 17, "inactive selfdestruct lost transfer");
        require(EMPTY.codehash == bytes32(0), "inactive empty account survived");
        vm.selectFork(second);
        require(VICTIM.code.length == 22 && VICTIM.balance == 17, "finalization crossed forks");
    }}
}}
"#, rpc = handle.http_endpoint()),
    );
    prj.update_config(|config| config.evm_version = EvmVersion::Berlin);
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse().args(["test", "--mc", "NativeForkFinalizationTest"]).assert_success();
    }
}

#[forgetest_init]
fn ethereum_native_snapshot_transaction_boundary(prj: _, cmd: _) {
    prj.add_test(
        "NativeSnapshotBoundary.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
contract NativeSnapshotBoundaryTest is Test {
    uint256 saved;
    uint256 value;
    function setUp() public {
        value = 5;
        assembly { tstore(0, 17) }
        saved = vm.snapshotState();
    }
    function testRestoreResetsTransactionScratch() public {
        require(vm.revertToState(saved), "snapshot missing");
        uint256 transientValue;
        uint256 loaded;
        uint256 beforeRead = gasleft();
        assembly { loaded := sload(value.slot) }
        uint256 readGas = beforeRead - gasleft();
        assembly { transientValue := tload(0) }
        require(transientValue == 0, "snapshot carried transient storage");
        require(loaded == 5, "snapshot lost durable state");
        require(readGas > 2000, "snapshot carried slot warmth");
        uint256 beforeWrite = gasleft();
        value = 6;
        require(beforeWrite - gasleft() > 2800, "snapshot carried stale original");
        uint256 beforeBalance = gasleft();
        uint256 accountBalance;
        assembly { accountBalance := balance(address()) }
        uint256 balanceGas = beforeBalance - gasleft();
        require(accountBalance == address(this).balance && balanceGas < 500, "recipient not prewarmed");
    }
    function testSameTransactionRestoreKeepsScratch() public {
        value = 11;
        assembly { tstore(0, 19) }
        uint256 sameTransaction = vm.snapshotState();
        value = 22;
        assembly { tstore(0, 23) }
        require(vm.revertToState(sameTransaction), "snapshot missing");
        uint256 transientValue;
        assembly { transientValue := tload(0) }
        require(value == 11 && transientValue == 19, "same transaction scratch lost");
    }
    function testFuzzRestoreResetsTransactionScratch(uint8) public {
        testRestoreResetsTransactionScratch();
    }
}
"#,
    );
    prj.update_config(|config| {
        config.evm_version = EvmVersion::Cancun;
        config.fuzz.runs = 4;
    });
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse().args(["test", "--mc", "NativeSnapshotBoundaryTest"]).assert_success();
    }
}

#[forgetest_init]
fn ethereum_native_storage_callbacks(prj: _, cmd: _) {
    prj.add_test(
        "NativeStorageCallbacks.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
contract NativeHookStore {
    uint256 public value = 7;
    function set(uint256 next) external { value = next; }
    function read() external view returns (uint256) { return value; }
}
contract NativeStorageCallbacksTest is Test {
    NativeHookStore target;
    NativeHookStore baselineStore;
    uint256 stores;
    uint256 loads;
    bool shouldFail;
    function setUp() public { target = new NativeHookStore(); baselineStore = new NativeHookStore(); }
    function onStore(address account, bytes32 slot, bytes32 old, bytes32 next) external {
        require(msg.sender == address(vm), "callback sender");
        require(account == address(target) && slot == bytes32(0), "callback location");
        require(old == bytes32(uint256(7)) && next == bytes32(uint256(9)), "store values");
        require(address(0xdead01).balance == 0, "warm callback account");
        stores++;
        if (shouldFail) revert("callback failed");
    }
    function onLoad(address account, bytes32 slot, bytes32 value) external {
        require(msg.sender == address(vm), "callback sender");
        require(account == address(target) && slot == bytes32(0), "callback location");
        require(value == bytes32(uint256(7)), "load value");
        loads++;
        target.set(11);
    }
    function testStoreCallbackPreservesGasAndWrites() public {
        uint256 beforeGas = gasleft();
        baselineStore.set(9);
        uint256 baseline = beforeGas - gasleft();
        vm.registerSstoreHook(address(target), this.onStore.selector);
        beforeGas = gasleft();
        target.set(9);
        require(beforeGas - gasleft() < baseline + 5000, "callback charged parent gas");
        require(stores == 1 && target.value() == 9, "store callback state");
        uint256 beforeBalance = gasleft();
        uint256 accountBalance;
        assembly { accountBalance := balance(0xdead01) }
        require(accountBalance == 0 && beforeBalance - gasleft() > 2000, "callback warmth leaked");
    }
    function testLoadCallbackPreservesLoadedResult() public {
        vm.registerSloadHook(address(target), this.onLoad.selector);
        require(target.read() == 7, "callback replaced SLOAD result");
        require(loads == 1 && vm.load(address(target), bytes32(0)) == bytes32(uint256(11)), "load callback writes");
    }
    function testStoreCallbackRevertPropagatesAndRollsBack() public {
        shouldFail = true;
        vm.registerSstoreHook(address(target), this.onStore.selector);
        vm.expectRevert(bytes("callback failed"));
        target.set(9);
        require(stores == 0 && target.value() == 7, "callback revert leaked state");
    }
}
"#,
    );
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse().args(["test", "--mc", "NativeStorageCallbacksTest"]).assert_success();
    }
}

#[forgetest_init]
async fn ethereum_native_fork_speculative_fixture_publication(prj: _, cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let remote = Address::with_last_byte(0x91);
    api.anvil_set_code(remote, bytes!("00")).await.unwrap();
    api.anvil_set_storage_at(remote, U256::ZERO, U256::from(7).into()).await.unwrap();
    api.anvil_mine(Some(U256::from(2)), None).await.unwrap();
    let source = r#"
import {Test} from "forge-std/Test.sol";
contract NativeSpeculativeFixtureTest is Test {
    address constant REMOTE = address(0x91);
    uint256 first;
    uint256 second;
    uint256 saved;
    uint256 marker;
    function setUp() public {
        first = vm.createSelectFork("RPC_ENDPOINT");
        second = vm.createFork("RPC_ENDPOINT");
        vm.warp(1000);
        vm.roll(3);
        vm.chainId(31337);
        marker = 7;
        saved = vm.snapshotState();
    }
    function mutate() internal {
        vm.store(REMOTE, bytes32(0), bytes32(uint256(77)));
        marker = 99;
        vm.makePersistent(REMOTE);
        vm.createFork("RPC_ENDPOINT");
        vm.selectFork(second);
        vm.warp(1111);
        vm.roll(777);
        vm.chainId(4242);
        vm.snapshotState();
        require(vm.deleteStateSnapshot(saved), "fixture snapshot missing");
    }
    function fixtureValues() public returns (uint256[] memory values) {
        mutate();
        values = new uint256[](1);
        values[0] = 777;
    }
    function fixtureRejected(uint256) public returns (uint256) {
        mutate();
        revert("speculative fixture revert");
    }
    function testFuzzSpeculationDoesNotPublish(uint256 values) public {
        vm.assume(values == 777);
        require(vm.activeFork() == first && marker == 7, "fixture fork/state published");
        require(block.timestamp == 1000 && block.number == 3 && block.chainid == 31337, "fixture environment published");
        require(!vm.isPersistent(REMOTE), "fixture inspector metadata published");
        require(vm.load(REMOTE, bytes32(0)) == bytes32(uint256(7)), "fixture storage published");
        require(vm.snapshotState() == saved + 1, "fixture snapshot allocation published");
        require(vm.revertToState(saved), "fixture snapshot deletion published");
        require(vm.createFork("RPC_ENDPOINT") == 2, "fixture registry published");
        require(marker == 7 && vm.activeFork() == first, "setup snapshot changed");
        require(vm.load(REMOTE, bytes32(0)) == bytes32(uint256(7)), "accepted backing changed");
    }
}
"#;
    prj.add_test(
        "NativeSpeculativeFixture.t.sol",
        &source.replace("RPC_ENDPOINT", &handle.http_endpoint()),
    );
    prj.update_config(|config| {
        config.fuzz.runs = 2;
        config.fuzz.seed = Some(U256::from(42));
        config.fuzz.max_test_rejects = 64;
        config.fuzz.dictionary.dictionary_weight = 0;
    });
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse().args(["test", "--mc", "NativeSpeculativeFixtureTest"]).assert_success();
    }
}

#[forgetest_init]
async fn ethereum_native_fork_speculative_unloaded_accepted_state(prj: _, cmd: _) {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let remote = Address::with_last_byte(0x91);
    api.anvil_set_code(remote, bytes!("00")).await.unwrap();
    api.anvil_set_storage_at(remote, U256::ZERO, U256::from(7).into()).await.unwrap();
    let unread = Address::with_last_byte(0x92);
    api.anvil_set_code(unread, bytes!("00")).await.unwrap();
    api.anvil_set_storage_at(unread, U256::ZERO, U256::from(9).into()).await.unwrap();
    api.anvil_mine(Some(U256::from(2)), None).await.unwrap();
    let source = r#"
import {Test} from "forge-std/Test.sol";
contract NativeSpeculativeBackingTest is Test {
    address constant REMOTE = address(0x91);
    address constant UNREAD = address(0x92);
    uint256 first;
    uint256 second;
    uint256 saved;
    uint256 marker;
    function setUp() public {
        first = vm.createSelectFork("RPC_ENDPOINT");
        second = vm.createFork("RPC_ENDPOINT");
        vm.warp(1000);
        vm.roll(3);
        vm.chainId(31337);
        vm.store(UNREAD, bytes32(0), bytes32(uint256(55)));
        marker = 7;
        saved = vm.snapshotState();
    }
    function mutate() internal {
        vm.store(REMOTE, bytes32(0), bytes32(uint256(77)));
        marker = 99;
        vm.makePersistent(REMOTE);
        vm.createFork("RPC_ENDPOINT");
        vm.selectFork(second);
        vm.warp(1111);
        vm.roll(777);
        vm.chainId(4242);
        vm.snapshotState();
        require(vm.deleteStateSnapshot(saved), "fixture snapshot missing");
    }
    function fixtureValues() public returns (uint256[] memory values) {
        vm.selectFork(second);
        vm.selectFork(first);
        require(vm.load(UNREAD, bytes32(0)) == bytes32(uint256(55)), "unloaded accepted backing lost");
        mutate();
        values = new uint256[](1);
        values[0] = 777;
    }
    function fixtureRejected(uint256) public returns (uint256) {
        mutate();
        revert("speculative fixture revert");
    }
    function testFuzzSpeculationDoesNotPublish(uint256 values) public {
        vm.assume(values == 777);
        require(vm.activeFork() == first && marker == 7, "fixture fork/state published");
        require(block.timestamp == 1000 && block.number == 3 && block.chainid == 31337, "fixture environment published");
        require(!vm.isPersistent(REMOTE), "fixture inspector metadata published");
        require(vm.load(REMOTE, bytes32(0)) == bytes32(uint256(7)), "fixture storage published");
        require(vm.snapshotState() == saved + 1, "fixture snapshot allocation published");
        require(vm.revertToState(saved), "fixture snapshot deletion published");
        require(vm.createFork("RPC_ENDPOINT") == 2, "fixture registry published");
        require(marker == 7 && vm.activeFork() == first, "setup snapshot changed");
        require(vm.load(REMOTE, bytes32(0)) == bytes32(uint256(7)), "accepted backing changed");
    }
}
"#;
    prj.add_test(
        "NativeSpeculativeBacking.t.sol",
        &source.replace("RPC_ENDPOINT", &handle.http_endpoint()),
    );
    prj.update_config(|config| {
        config.fuzz.runs = 2;
        config.fuzz.seed = Some(U256::from(42));
        config.fuzz.max_test_rejects = 64;
        config.fuzz.dictionary.dictionary_weight = 0;
    });
    for isolate in [false, true] {
        prj.update_config(|config| config.isolate = isolate);
        cmd.forge_fuse().args(["test", "--mc", "NativeSpeculativeBackingTest"]).assert_success();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ethereum_native_fork_receipt_log_ordering() {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let round_trip = vec![
        Vm::createSelectFork_0Call { urlOrAlias: handle.http_endpoint() }.abi_encode(),
        Vm::createSelectFork_0Call { urlOrAlias: handle.http_endpoint() }.abi_encode(),
        Vm::selectForkCall { forkId: U256::ZERO }.abi_encode(),
    ];
    let create = || Vm::createSelectFork_0Call { urlOrAlias: handle.http_endpoint() }.abi_encode();
    let cases = [
        round_trip,
        vec![create(), create()],
        vec![
            create(),
            Vm::snapshotStateCall {}.abi_encode(),
            create(),
            Vm::revertToStateCall { snapshotId: U256::ZERO }.abi_encode(),
        ],
        vec![
            create(),
            Vm::rollFork_0Call { blockNumber: U256::ZERO }.abi_encode(),
            create(),
            Vm::selectForkCall { forkId: U256::ZERO }.abi_encode(),
        ],
    ];
    for calls in cases {
        let expected_topics = (1..=calls.len()).map(U256::from).collect::<Vec<_>>();
        let calls = calls
            .into_iter()
            .enumerate()
            .map(|(index, call)| {
                (CHEATCODE_ADDRESS, call, Some(u8::try_from(index + 1).unwrap()), true)
            })
            .collect::<Vec<_>>();
        let (code, input) = receipt_log_program(&calls);
        let contract = foundry_evm::constants::TEST_CONTRACT_ADDRESS;
        let spec = SpecId::CANCUN;
        let mut version = Version::new(spec);
        version.features.remove(
            EvmFeatures::NONCE_CHECK | EvmFeatures::EIP3607 | EvmFeatures::BLOCK_GAS_LIMIT_CHECK,
        );
        for isolate in [false, true] {
            for revert in [false, true] {
                let mut runtime = code.clone();
                if revert {
                    runtime.extend([0x5f, 0x5f, 0xfd]);
                } else {
                    runtime.push(0x00);
                }
                let mut executor = Executor::new(
                    EmptyDB::default(),
                    spec,
                    ExecutionConfig::for_spec_and_version(spec, version),
                    BlockEnvExt::default(),
                );
                executor
                    .apply_state_overrides(
                        [(
                            contract,
                            AccountOverride { code: Some(runtime.into()), ..Default::default() },
                        )]
                        .into_iter()
                        .collect(),
                    )
                    .unwrap();
                let tx = Recovered::new_unchecked(
                    evm2::ethereum::TxEnvelope::Legacy(TxLegacy {
                        to: TxKind::Call(contract),
                        input: Bytes::copy_from_slice(&input),
                        gas_limit: 10_000_000,
                        ..Default::default()
                    }),
                    Address::with_last_byte(0xca),
                );
                let cheats =
                    Cheatcodes::new(Arc::new(CheatsConfig { isolate, ..Default::default() }));
                let (result, cheats) = executor.inspect_transact(&tx, cheats);
                let result = result.unwrap();
                assert_eq!(result.status, !revert, "output={:?}", result.output);
                let topics = |logs: &[alloy_primitives::Log]| {
                    logs.iter()
                        .map(|log| U256::from_be_bytes(log.topics()[0].0))
                        .collect::<Vec<_>>()
                };
                assert_eq!(topics(&cheats.logs), expected_topics);
                let expected = if revert { vec![] } else { expected_topics.clone() };
                assert_eq!(topics(&result.logs), expected, "isolate={isolate}, revert={revert}");
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ethereum_native_fork_receipt_logs_caught_child_revert() {
    let (_, handle) = anvil::spawn(NodeConfig::test()).await;
    let child = Address::with_last_byte(0xc1);
    let create = Vm::createSelectFork_0Call { urlOrAlias: handle.http_endpoint() }.abi_encode();
    let (mut child_code, child_input) =
        receipt_log_program(&[(CHEATCODE_ADDRESS, create.clone(), Some(2), true)]);
    child_code.extend([0x5f, 0x5f, 0xfd]);
    let (mut code, input) = receipt_log_program(&[
        (CHEATCODE_ADDRESS, Vm::makePersistent_0Call { account: child }.abi_encode(), None, true),
        (CHEATCODE_ADDRESS, Vm::allowCheatcodesCall { account: child }.abi_encode(), None, true),
        (CHEATCODE_ADDRESS, create, Some(1), true),
        (child, child_input, None, false),
        (CHEATCODE_ADDRESS, Vm::selectForkCall { forkId: U256::ZERO }.abi_encode(), Some(3), true),
    ]);
    code.push(0x00);
    let contract = foundry_evm::constants::TEST_CONTRACT_ADDRESS;
    let spec = SpecId::CANCUN;
    let mut version = Version::new(spec);
    version.features.remove(
        EvmFeatures::NONCE_CHECK | EvmFeatures::EIP3607 | EvmFeatures::BLOCK_GAS_LIMIT_CHECK,
    );
    for isolate in [false, true] {
        let mut executor = Executor::new(
            EmptyDB::default(),
            spec,
            ExecutionConfig::for_spec_and_version(spec, version),
            BlockEnvExt::default(),
        );
        executor
            .apply_state_overrides(
                [
                    (
                        contract,
                        AccountOverride {
                            code: Some(Bytes::copy_from_slice(&code)),
                            ..Default::default()
                        },
                    ),
                    (
                        child,
                        AccountOverride {
                            code: Some(Bytes::copy_from_slice(&child_code)),
                            ..Default::default()
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            )
            .unwrap();
        let tx = Recovered::new_unchecked(
            evm2::ethereum::TxEnvelope::Legacy(TxLegacy {
                to: TxKind::Call(contract),
                input: Bytes::copy_from_slice(&input),
                gas_limit: 10_000_000,
                ..Default::default()
            }),
            Address::with_last_byte(0xca),
        );
        let cheats = Cheatcodes::new(Arc::new(CheatsConfig { isolate, ..Default::default() }));
        let (result, cheats) = executor.inspect_transact(&tx, cheats);
        let result = result.unwrap();
        assert!(result.status, "isolate={isolate}, output={:?}", result.output);
        let topics = |logs: &[alloy_primitives::Log]| {
            logs.iter().map(|log| U256::from_be_bytes(log.topics()[0].0)).collect::<Vec<_>>()
        };
        assert_eq!(topics(&cheats.logs), vec![U256::ONE, U256::from(2), U256::from(3)]);
        assert_eq!(topics(&result.logs), vec![U256::ONE, U256::from(3)], "isolate={isolate}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ethereum_native_fork_receipt_logs_transact_switch() {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let provider = ProviderBuilder::new().connect_http(handle.http_endpoint().parse().unwrap());
    let sender = handle.dev_accounts().next().unwrap();
    let remote = Address::with_last_byte(0xe1);
    for revert in [false, true] {
        let (mut remote_code, remote_input) = receipt_log_program(&[(
            CHEATCODE_ADDRESS,
            Vm::selectForkCall { forkId: U256::ONE }.abi_encode(),
            Some(2),
            true,
        )]);
        if revert {
            remote_code.extend([0x5f, 0x5f, 0xfd]);
        } else {
            remote_code.push(0x00);
        }
        api.anvil_set_code(remote, remote_code.into()).await.unwrap();
        let receipt = provider
            .send_transaction(TransactionRequest {
                from: Some(sender),
                to: Some(remote.into()),
                gas: Some(3_000_000),
                gas_price: Some(2_000_000_000),
                input: TransactionInput::new(remote_input.into()),
                ..Default::default()
            })
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        assert_eq!(receipt.status(), !revert);
        let (mut code, input) = receipt_log_program(&[
            (
                CHEATCODE_ADDRESS,
                Vm::createSelectFork_0Call { urlOrAlias: handle.http_endpoint() }.abi_encode(),
                Some(1),
                true,
            ),
            (
                CHEATCODE_ADDRESS,
                Vm::createFork_0Call { urlOrAlias: handle.http_endpoint() }.abi_encode(),
                None,
                true,
            ),
            (
                CHEATCODE_ADDRESS,
                Vm::allowCheatcodesCall { account: remote }.abi_encode(),
                None,
                true,
            ),
            (
                CHEATCODE_ADDRESS,
                Vm::transact_0Call { txHash: receipt.transaction_hash }.abi_encode(),
                Some(3),
                true,
            ),
        ]);
        code.push(0x00);
        let contract = foundry_evm::constants::TEST_CONTRACT_ADDRESS;
        let spec = SpecId::CANCUN;
        let mut version = Version::new(spec);
        version.features.remove(
            EvmFeatures::NONCE_CHECK | EvmFeatures::EIP3607 | EvmFeatures::BLOCK_GAS_LIMIT_CHECK,
        );
        for isolate in [false, true] {
            let mut executor = Executor::new(
                EmptyDB::default(),
                spec,
                ExecutionConfig::for_spec_and_version(spec, version),
                BlockEnvExt::default(),
            );
            executor
                .apply_state_overrides(
                    [(
                        contract,
                        AccountOverride {
                            code: Some(Bytes::copy_from_slice(&code)),
                            ..Default::default()
                        },
                    )]
                    .into_iter()
                    .collect(),
                )
                .unwrap();
            let tx = Recovered::new_unchecked(
                evm2::ethereum::TxEnvelope::Legacy(TxLegacy {
                    to: TxKind::Call(contract),
                    input: Bytes::copy_from_slice(&input),
                    gas_limit: 10_000_000,
                    ..Default::default()
                }),
                Address::with_last_byte(0xca),
            );
            let cheats = Cheatcodes::new(Arc::new(CheatsConfig { isolate, ..Default::default() }));
            let (result, cheats) = executor.inspect_transact(&tx, cheats);
            let result = result.unwrap();
            assert!(result.status, "isolate={isolate}, output={:?}", result.output);
            let topics = |logs: &[alloy_primitives::Log]| {
                logs.iter().map(|log| U256::from_be_bytes(log.topics()[0].0)).collect::<Vec<_>>()
            };
            let expected = vec![U256::ONE, U256::from(2), U256::from(3)];
            assert_eq!(topics(&cheats.logs), expected);
            let expected_receipt = if revert { vec![U256::ONE, U256::from(3)] } else { expected };
            assert_eq!(
                topics(&result.logs),
                expected_receipt,
                "isolate={isolate}, revert={revert}"
            );
        }
    }
}

// Require each call's intended outcome, then emit an optional ordinal as a log topic.
fn receipt_log_program(calls: &[(Address, Vec<u8>, Option<u8>, bool)]) -> (Vec<u8>, Vec<u8>) {
    let mut code = Vec::new();
    let mut input = Vec::new();
    for (callee, call, topic, succeeds) in calls {
        let len = u16::try_from(call.len()).unwrap().to_be_bytes();
        let offset = u16::try_from(input.len()).unwrap().to_be_bytes();
        code.extend([0x61, len[0], len[1], 0x61, offset[0], offset[1], 0x5f, 0x37]);
        code.extend([0x5f, 0x5f, 0x61, len[0], len[1], 0x5f, 0x5f, 0x73]);
        code.extend(callee.as_slice());
        code.extend([0x62, 0x0f, 0x42, 0x40, 0xf1]);
        let success = u16::try_from(code.len() + 14).unwrap().to_be_bytes();
        code.extend([
            0x60,
            u8::from(*succeeds),
            0x14,
            0x61,
            success[0],
            success[1],
            0x57,
            0x3d,
            0x5f,
            0x5f,
            0x3e,
            0x3d,
            0x5f,
            0xfd,
            0x5b,
        ]);
        if let Some(topic) = topic {
            code.extend([0x60, *topic, 0x5f, 0x5f, 0xa1]);
        }
        input.extend(call);
    }
    (code, input)
}

#[forgetest_init]
fn ethereum_native_invariant_grouped_predicates(prj: _, cmd: _) {
    let source = r#"
contract NativeGroupedHandler {
    uint256 public count;
    function increment() public { count++; }
}
contract NativeGroupedInvariantTest {
    NativeGroupedHandler handler;
    uint256 predicateWrite;
    function setUp() public { handler = new NativeGroupedHandler(); }
    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1); targets[0] = address(handler);
    }
    function invariant_first() public {
        require(predicateWrite == 0, "predicate state published"); predicateWrite = 1;
        require(handler.count() < 4, "first broken");
    }
    function invariant_second() public view {
        require(predicateWrite == 0, "predicate state published");
        require(handler.count() < 6, "second broken");
    }
}
"#;
    for isolate in [false, true] {
        let failure_dir = prj.root().join(format!("grouped-{isolate}"));
        prj.add_test("NativeGrouped.t.sol", source);
        prj.update_config(|config| {
            config.isolate = isolate;
            config.invariant.runs = 2;
            config.invariant.depth = 8;
            config.invariant.shrink_run_limit = 128;
            config.invariant.failure_persist_dir = Some(failure_dir.clone());
            config.fuzz.seed = Some(U256::from(42));
        });
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeGroupedInvariantTest"])
            .assert_failure()
            .stdout_eq(str![[r#"
...
NativeGroupedInvariantTest invariants: 2/2 invariants broken
[FAIL: first broken] invariant_first
[FAIL: second broken] invariant_second
 NativeGroupedInvariantTest invariants (runs: 1, calls: 6, reverts: 0)
...
"#]]);
        for (name, length) in [("invariant_first", 4), ("invariant_second", 6)] {
            let path =
                failure_dir.join("failures/NativeGroupedInvariantTest/invariants").join(name);
            let record: NativeInvariantFailureRecord =
                foundry_common::fs::read_json_file(&path).unwrap();
            assert_eq!(record.call_sequence.len(), length);
            assert!(!record.assertion_failure);
        }
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeGroupedInvariantTest"])
            .assert_failure()
            .stdout_eq(str![[r#"
...
NativeGroupedInvariantTest invariants: 2/2 invariants broken
[FAIL: first broken] invariant_first
[FAIL: second broken] invariant_second
 NativeGroupedInvariantTest invariants (runs: 0, calls: 0, reverts: 0, failed corpus replays: 2)
...
"#]]);
        // A filter selects one predicate and its own persisted failure, not an excluded failure.
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeGroupedInvariantTest", "--mt", "invariant_second"])
            .assert_failure()
            .stdout_eq(str![[r#"
...
 invariant_second() (runs: 0, calls: 0, reverts: 0, failed corpus replays: 1)
...
"#]]);
        // A replayed failure remains counted when another predicate runs a fresh campaign.
        let partially_fixed = source.replace("handler.count() < 4", "handler.count() <= 8");
        prj.add_test("NativeGrouped.t.sol", &partially_fixed);
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeGroupedInvariantTest"])
            .assert_failure()
            .stdout_eq(str![[r#"
...
NativeGroupedInvariantTest invariants: 1/2 invariants broken
[PASS] invariant_first
[FAIL: second broken] invariant_second
 NativeGroupedInvariantTest invariants (runs: 2, calls: 16, reverts: 0, failed corpus replays: 1)
...
"#]]);
        // An old predicate replay must not be accepted as a new handler assertion.
        let handler_assertion = source.replace("count++;", "assert(false);");
        prj.add_test("NativeGrouped.t.sol", &handler_assertion);
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeGroupedInvariantTest"])
            .assert_failure()
            .stdout_eq(str![[r#"
...
NativeGroupedInvariantTest invariants: 0/2 invariants broken
[PASS] invariant_first
[PASS] invariant_second

Assertion Tests: 1 assertion bug(s) found
...
 NativeGroupedInvariantTest invariants (runs: 1, calls: 1, reverts: 1)
...
"#]]);
        let fixed = source
            .replace("handler.count() < 4", "handler.count() <= 8")
            .replace("handler.count() < 6", "handler.count() <= 8");
        prj.add_test("NativeGrouped.t.sol", &fixed);
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeGroupedInvariantTest"])
            .assert_success()
            .stdout_eq(str![[r#"
...
[PASS]
NativeGroupedInvariantTest invariants:
[PASS] invariant_first
[PASS] invariant_second
 NativeGroupedInvariantTest invariants (runs: 2, calls: 16, reverts: 0)
...
"#]]);
        // Different campaign policy keeps predicates in independent campaigns.
        let split = fixed.replace(
            "    function invariant_second()",
            "    /// forge-config: default.invariant.depth = 2\n    function invariant_second()",
        );
        prj.add_test("NativeGrouped.t.sol", &split);
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeGroupedInvariantTest"])
            .assert_success()
            .stdout_eq(str![[r#"
...
[PASS] invariant_first() (runs: 2, calls: 16, reverts: 0)
[PASS] invariant_second() (runs: 2, calls: 4, reverts: 0)
...
"#]]);
    }
}

#[forgetest_init]
fn ethereum_native_invariant_replay_failure_identity(prj: _, cmd: _) {
    let source = r#"
contract NativeIdentityHandler {
    uint256 public count;
    function increment() public { count++; }
}
contract NativeIdentityInvariantTest {
    NativeIdentityHandler handler;
    function setUp() public { handler = new NativeIdentityHandler(); }
    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1); targets[0] = address(handler);
    }
    function invariant_safe() public view { require(handler.count() < 4, "original reason"); }
}
"#;
    for isolate in [false, true] {
        prj.add_test("NativeIdentity.t.sol", source);
        prj.update_config(|config| {
            config.isolate = isolate;
            config.invariant.runs = 2;
            config.invariant.depth = 8;
            config.invariant.shrink_run_limit = 128;
            config.invariant.failure_persist_dir =
                Some(prj.root().join(format!("identity-{isolate}")));
            config.fuzz.seed = Some(U256::from(42));
        });
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeIdentityInvariantTest"])
            .assert_failure()
            .stdout_eq(str![[r#"
...
 invariant_safe() (runs: 1, calls: 4, reverts: 0)
...
"#]]);
        // Legacy records remain readable; successful replay upgrades their concrete witness.
        let path = prj.root().join(format!(
            "identity-{isolate}/failures/NativeIdentityInvariantTest/invariants/invariant_safe"
        ));
        let mut legacy: serde_json::Value = foundry_common::fs::read_json_file(&path).unwrap();
        assert!(legacy.as_object_mut().unwrap().remove("execution_failure").is_some());
        foundry_common::fs::write_json_file(&path, &legacy).unwrap();
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeIdentityInvariantTest"])
            .assert_failure()
            .stdout_eq(str![[r#"
...
 invariant_safe() (runs: 0, calls: 0, reverts: 0, failed corpus replays: 1)
...
"#]]);
        let changed = source.replace("original reason", "changed reason");
        prj.add_test("NativeIdentity.t.sol", &changed);
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeIdentityInvariantTest"])
            .assert_failure()
            .stdout_eq(str![[r#"
...
 invariant_safe() (runs: 1, calls: 4, reverts: 0)
...
"#]]);
        // The same reason at another execution phase is a different failure.
        let hook = changed.replace("handler.count() < 4", "handler.count() <= 8")
            .replace("    function invariant_safe()", "    function afterInvariant() public view { require(handler.count() == 0, \"changed reason\"); }\n    function invariant_safe()");
        prj.add_test("NativeIdentity.t.sol", &hook);
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeIdentityInvariantTest"])
            .assert_failure()
            .stdout_eq(str![[r#"
...
 invariant_safe() (runs: 1, calls: 8, reverts: 0)
...
"#]]);
        cmd.forge_fuse()
            .args(["test", "--mc", "NativeIdentityInvariantTest"])
            .assert_failure()
            .stdout_eq(str![[r#"
...
 invariant_safe() (runs: 0, calls: 0, reverts: 0, failed corpus replays: 1)
...
"#]]);
    }
}

#[forgetest_init]
async fn ethereum_native_script_blob_publication_and_resume(prj: _, cmd: _) {
    // Public development key, used only on each disposable local node.
    let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    prj.add_script(
        "NativeBlob.s.sol",
        r#"
import {Script} from "forge-std/Script.sol";
contract NativeBlobTarget {
    bytes32 public first;
    bytes32 public second;
    function record(bool primary) external {
        bytes32 hash;
        assembly { hash := blobhash(0) }
        if (primary) first = hash; else second = hash;
    }
}
contract NativeBlobScript is Script {
    function runConflict() public {
        vm.startBroadcast();
        NativeBlobTarget target = new NativeBlobTarget();
        vm.attachBlob(bytes("blob cannot coexist with a delegation"));
        vm.signAndAttachDelegation(address(target), 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80);
        target.record(true);
    }
    function run() public {
        vm.startBroadcast();
        vm.attachBlob(bytes("native blob transaction"));
        // CREATE must leave the attachment for the following CALL.
        NativeBlobTarget target = new NativeBlobTarget();
        target.record(true);
        target.record(false);
        vm.stopBroadcast();
    }
}
"#,
    );
    for (hardfork, version) in
        [(EthereumHardfork::Cancun, "cancun"), (EthereumHardfork::Osaka, "osaka")]
    {
        let (_api, handle) =
            anvil::spawn(NodeConfig::test().with_hardfork(Some(hardfork.into()))).await;
        let provider = ProviderBuilder::new().connect_http(handle.http_endpoint().parse().unwrap());
        let sender = handle.dev_accounts().next().unwrap();
        let target = sender.create(0);
        let rpc = handle.http_endpoint();
        let args = [
            "script",
            "NativeBlobScript",
            "--rpc-url",
            &rpc,
            "--evm-version",
            version,
            "--private-key",
            key,
            "--non-interactive",
        ];
        let mut expected = None;
        for isolate in [false, true] {
            prj.update_config(|config| config.isolate = isolate);
            cmd.forge_fuse().args(args).assert_success();
            let path = prj.root().join("broadcast/NativeBlob.s.sol/31337/dry-run/run-latest.json");
            let sequence =
                foundry_common::fs::read_json_file::<ScriptSequence<Ethereum>>(&path).unwrap();
            assert_eq!(sequence.transactions.len(), 3);
            let requests = sequence
                .transactions
                .iter()
                .map(|transaction| {
                    let TransactionMaybeSigned::Unsigned(request) = transaction.tx() else {
                        panic!("dry run must retain unsigned requests")
                    };
                    request
                })
                .collect::<Vec<_>>();
            assert!(requests[0].blob_sidecar().is_none());
            let blob = requests[1];
            let sidecar = blob.blob_sidecar().unwrap();
            assert_eq!(sidecar.as_eip4844().is_some(), version == "cancun");
            let hashes = blob.blob_versioned_hashes().unwrap();
            assert_eq!(hashes.len(), 1);
            expected = Some(U256::from_be_slice(hashes[0].as_slice()));
            assert!(requests[2].blob_sidecar().is_none());
            assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 0);
            assert!(provider.get_code_at(target).await.unwrap().is_empty());
            cmd.forge_fuse().args(args).args(["--sig", "runConflict()"]).assert_failure().stderr_eq(str![[r#"
Error: script failed: both delegation and blob are active; `attachBlob` and `attachDelegation` are not compatible

"#]]);
            assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 0);
        }
        cmd.forge_fuse().args(args).arg("--broadcast").arg("--slow").assert_success();
        assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 3);
        assert_eq!(provider.get_storage_at(target, U256::ZERO).await.unwrap(), expected.unwrap());
        assert_eq!(provider.get_storage_at(target, U256::ONE).await.unwrap(), U256::ZERO);
        cmd.forge_fuse().args(args).args(["--broadcast", "--resume"]).assert_success();
        assert_eq!(provider.get_transaction_count(sender).await.unwrap(), 3);
        assert_eq!(provider.get_storage_at(target, U256::ZERO).await.unwrap(), expected.unwrap());
    }
}

#[forgetest_init]
fn ethereum_native_coverage_unit_fuzz_invariant_and_reverts(prj: _, cmd: _) {
    prj.add_source(
        "NativeCoverageTarget.sol",
        r#"
contract NativeCoverageTarget {
    uint256 public value;
    constructor() {}
    function hit(uint256 input) public { value = input; }
    function reverted() public { value = 99; revert("expected"); }
}
"#,
    );
    prj.add_test(
        "NativeCoverage.t.sol",
        r#"
import {NativeCoverageTarget} from "../src/NativeCoverageTarget.sol";
contract NativeCoverageUnitTest {
    NativeCoverageTarget target;
    function setUp() public { target = new NativeCoverageTarget(); }
    function test_reverted_frame_is_observed() public {
        target.hit(1);
        (bool ok,) = address(target).call(abi.encodeWithSignature("reverted()"));
        require(!ok && target.value() == 1, "frame rollback");
    }
}
contract NativeCoverageFuzzTest {
    NativeCoverageTarget target;
    function setUp() public { target = new NativeCoverageTarget(); }
    function testFuzz_hit(uint256 input) public { target.hit(input); }
}
contract NativeCoverageHandler {
    NativeCoverageTarget target;
    uint256 public count;
    constructor(NativeCoverageTarget value) { target = value; }
    function step() public { target.hit(++count); }
}
contract NativeCoverageInvariantTest {
    NativeCoverageHandler handler;
    function setUp() public { handler = new NativeCoverageHandler(new NativeCoverageTarget()); }
    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1); targets[0] = address(handler);
    }
    function invariant_safe() public view { require(handler.count() <= 4, "fresh run state"); }
}
"#,
    );
    let mut first = None;
    for isolate in [false, true] {
        prj.update_config(|config| {
            config.isolate = isolate;
            config.fuzz.runs = 4;
            config.invariant.runs = 2;
            config.invariant.depth = 4;
            config.fuzz.seed = Some(U256::from(42));
        });
        cmd.forge_fuse()
            .args(["coverage", "--mc", "NativeCoverage.*Test", "--report", "lcov"])
            .assert_success();
        let report = foundry_common::fs::read_to_string(prj.root().join("lcov.info")).unwrap();
        let target = report
            .split("SF:src/NativeCoverageTarget.sol\n")
            .nth(1)
            .unwrap()
            .split("end_of_record")
            .next()
            .unwrap();
        let calls = target.lines().filter(|line| line.starts_with("FNDA:")).collect::<Vec<_>>();
        assert_eq!(
            calls,
            [
                "FNDA:3,NativeCoverageTarget.constructor",
                "FNDA:13,NativeCoverageTarget.hit",
                "FNDA:1,NativeCoverageTarget.reverted"
            ]
        );
        if let Some(first) = &first {
            assert_eq!(&report, first, "isolation must preserve coverage observations");
        } else {
            first = Some(report);
        }
    }
}
