use super::symbolic_helpers::{
    assert_symbolic, assert_symbolic_engine_witness, assert_symbolic_witness, z3_available,
};
use crate::skip_unless_z3;
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, snapbox::IntoData, str, util::OutputExt};

#[forgetest_init]
fn symbolic_create_contains_invalid_initcode_halt(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_contains_invalid_initcode_halt because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInvalidInitcode.t.sol",
        r#"
contract SymbolicInvalidInitcode {
    uint256 marker;

    function checkInvalidInitcode() public {
        marker = 19;
        address created;
        uint256 returnSize;
        assembly ("memory-safe") {
            mstore8(0, 0xfe)
            created := create(0, 0, 1)
            returnSize := returndatasize()
        }
        assert(created == address(0));
        assert(returnSize == 0);
        assert(marker == 19);
    }
}
"#,
    );

    cmd.args(["test", "--symbolic", "--match-test", "checkInvalidInitcode"]).assert_success();
}

#[forgetest_init]
fn symbolic_create_respects_configured_runtime_code_limit(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_respects_configured_runtime_code_limit because z3 is not available"
        );
        return;
    }

    prj.update_config(|config| config.code_size_limit = Some(24_576));
    prj.add_test(
        "SymbolicCreateCodeLimit.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCreateCodeLimit is Test {
    function checkRuntimeCodeLimit() public {
        address created;
        assembly ("memory-safe") {
            mstore(0, shl(208, 0x6160016000f3))
            created := create(0, 0, 6)
        }
        assert(created == address(0));
        assert(created.code.length == 0);
    }

    function checkConfiguredRuntimeCodeLimit() public {
        address created;
        assembly ("memory-safe") {
            mstore(0, shl(208, 0x6160006000f3))
            created := create(0, 0, 6)
        }
        assert(created == address(0));
        assert(created.code.length == 0);
    }

    function checkExpectRevertRuntimeCodeLimit() public {
        vm.expectRevert();
        new OversizedRuntime();

        vm.expectRevert();
        new OversizedRuntime{salt: bytes32(uint256(1))}();
    }
}

contract OversizedRuntime {
    constructor() {
        assembly ("memory-safe") {
            return(0, 24577)
        }
    }
}
"#,
    );

    cmd.args(["test", "--symbolic", "--match-test", "checkRuntimeCodeLimit"]).assert_success();

    cmd.forge_fuse();
    cmd.args(["test", "--symbolic", "--match-test", "checkExpectRevertRuntimeCodeLimit"])
        .assert_success();

    prj.update_config(|config| config.code_size_limit = Some(24_575));
    cmd.forge_fuse();
    cmd.args(["test", "--symbolic", "--match-test", "checkConfiguredRuntimeCodeLimit"])
        .assert_success();
}

#[forgetest_init]
fn symbolic_create_deploys_and_calls_helper(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_deploys_and_calls_helper because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreate.t.sol",
        r#"
contract CreatedHelper {
    function inc(uint256 x) external pure returns (uint256) {
        return x + 1;
    }
}

contract SymbolicCreate {
    function checkCreate(uint256 x) public {
        CreatedHelper helper = new CreatedHelper();
        assert(helper.inc(x) != 9);
    }
}
"#,
    );

    let stdout =
        assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkCreate"]))
            .failure()
            .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicCreate.t.sol:SymbolicCreate
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkCreate(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
            .get_output()
            .stdout_lossy();

    assert!(!stdout.contains("unsupported opcode: 0xf0"), "{stdout}");
}

#[forgetest_init]
fn symbolic_create_respects_eip3541_runtime_prefix(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_respects_eip3541_runtime_prefix because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreateEip3541.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCreateEip3541 is Test {
    function checkRejectedPrefix() public {
        assert(deployCreate(0xef) == address(0));
        assert(deployCreate2(0xef) == address(0));
    }

    function checkRejectedPrefixPreservesWarp() public {
        bytes memory initcode = abi.encodePacked(type(WarpThenReject).creationCode, abi.encode(address(this)));
        address created;
        assembly ("memory-safe") {
            created := create(0, add(initcode, 32), mload(initcode))
        }
        assert(created == address(0));
        assert(block.timestamp == 123);
    }

    function checkRejectedPrefixPreservesMockProgress() public {
        checkMockProgress(address(0x1001), false);
        checkMockProgress(address(0x1002), true);
    }

    function checkRejectedPrefixExpectedCallsAndRevert() public {
        checkExpectedCallAndRevert(address(0x2001), false);
        checkExpectedCallAndRevert(address(0x2002), true);
    }

    function warp(uint256 timestamp) external {
        vm.warp(timestamp);
    }

    function checkMockProgress(address target, bool useCreate2) internal {
        bytes[] memory returnValues = new bytes[](2);
        returnValues[0] = abi.encode(uint256(1));
        returnValues[1] = abi.encode(uint256(2));
        vm.mockCalls(target, abi.encodeCall(IMockSequenceTarget.value, ()), returnValues);

        bytes memory initcode = abi.encodePacked(type(ConsumeMockThenReject).creationCode, abi.encode(target));
        address created;
        if (useCreate2) {
            assembly ("memory-safe") {
                created := create2(0, add(initcode, 32), mload(initcode), 1)
            }
        } else {
            assembly ("memory-safe") {
                created := create(0, add(initcode, 32), mload(initcode))
            }
        }
        assert(created == address(0));
        assertEq(IMockSequenceTarget(target).value(), 2);
    }

    function checkExpectedCallAndRevert(address target, bool useCreate2) internal {
        bytes memory callData = abi.encodeCall(IMockSequenceTarget.value, ());
        vm.mockCall(target, callData, abi.encode(uint256(1)));
        vm.expectCall(target, callData);
        bytes memory rejectedRuntime = hex"ef";
        vm.expectRevert(rejectedRuntime);
        if (useCreate2) {
            new ConsumeMockThenReject{salt: bytes32(uint256(2))}(IMockSequenceTarget(target));
        } else {
            new ConsumeMockThenReject(IMockSequenceTarget(target));
        }
    }

    function checkAllowedPrefixBeforeLondon() public {
        address created = deployCreate(0xef);
        address created2 = deployCreate2(0xef);
        assert(created != address(0));
        assert(created2 != address(0));
        assert(created.code.length == 1);
        assert(created2.code.length == 1);
    }

    function checkAdjacentPrefixStillAllowed() public {
        address created = deployCreate(0xee);
        address created2 = deployCreate2(0xee);
        assert(created != address(0));
        assert(created2 != address(0));
        assert(created.code.length == 1);
        assert(created2.code.length == 1);
    }

    function deployCreate(uint256 runtimeByte) internal returns (address created) {
        assembly ("memory-safe") {
            mstore(0, shl(176, or(0x600060005360016000f3, shl(64, runtimeByte))))
            created := create(0, 0, 10)
        }
    }

    function deployCreate2(uint256 runtimeByte) internal returns (address created) {
        assembly ("memory-safe") {
            mstore(0, shl(176, or(0x600060005360016000f3, shl(64, runtimeByte))))
            created := create2(0, 0, 10, 123)
        }
    }
}

contract WarpThenReject {
    constructor(SymbolicCreateEip3541 test) {
        test.warp(123);
        assembly ("memory-safe") {
            mstore(0, shl(248, 0xef))
            return(0, 1)
        }
    }
}

interface IMockSequenceTarget {
    function value() external returns (uint256);
}

contract ConsumeMockThenReject {
    constructor(IMockSequenceTarget target) {
        require(target.value() == 1);
        assembly ("memory-safe") {
            mstore(0, shl(248, 0xef))
            return(0, 1)
        }
    }
}
"#,
    );

    cmd.args(["test", "--symbolic", "--match-test", "checkRejectedPrefix"]).assert_success();

    cmd.forge_fuse();
    cmd.args(["test", "--symbolic", "--match-test", "checkRejectedPrefixPreservesWarp"])
        .assert_success();

    cmd.forge_fuse();
    cmd.args(["test", "--symbolic", "--match-test", "checkRejectedPrefixPreservesMockProgress"])
        .assert_success();

    cmd.forge_fuse();
    cmd.args(["test", "--symbolic", "--match-test", "checkRejectedPrefixExpectedCallsAndRevert"])
        .assert_success();

    cmd.forge_fuse();
    cmd.args(["test", "--symbolic", "--match-test", "checkAdjacentPrefixStillAllowed"])
        .assert_success();

    cmd.forge_fuse();
    cmd.args([
        "test",
        "--symbolic",
        "--evm-version",
        "berlin",
        "--match-test",
        "checkAllowedPrefixBeforeLondon",
    ])
    .assert_success();
}

// CREATE and CREATE2 with symbolic constructor args, initcode offset or size, and salt are modeled.
#[forgetest_init]
fn symbolic_create_accepts_symbolic_operands(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_create_accepts_symbolic_operands");

    prj.add_test(
        "SymbolicCreateConstructorArgs.t.sol",
        r#"
contract CreatedStore {
    uint256 public value;

    constructor(uint256 x) {
        value = x;
    }
}

contract SymbolicCreateConstructorArgs {
    function checkCreateConstructorArg(uint256 x) public {
        CreatedStore store = new CreatedStore(x);
        assert(store.value() == x);
    }
}
"#,
    );

    prj.add_test(
        "SymbolicCreateInitcodeOffset.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCreateInitcodeOffset is Test {
    function checkCreateInitcodeOffset(uint16 offset) public {
        vm.assume(offset == 0x80);

        address created;
        uint256 size;
        assembly {
            mstore(0x80, 0x6001600c60003960016000f30000000000000000000000000000000000000000)
            created := create(0, offset, 13)
            size := extcodesize(created)
        }

        assert(created != address(0));
        assert(size == 1);
    }
}
"#,
    );

    prj.add_test(
        "SymbolicCreateInitcodeSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCreateInitcodeSize is Test {
    function checkCreateInitcodeSize(uint256 size) public {
        vm.assume(size == 0 || size == 13);
        bytes memory code = hex"6001600c60003960016000f300";

        address created;
        assembly {
            created := create(0, add(code, 0x20), size)
        }

        assert(created != address(0));
        assertEq(created.code.length, size == 13 ? 1 : 0);
    }
}
"#,
    );

    prj.add_test(
        "SymbolicCreate2Args.t.sol",
        r#"
contract CreatedImmutable {
    uint256 immutable value;

    constructor(uint256 value_) {
        value = value_;
    }

    function get() external view returns (uint256) {
        return value;
    }
}

contract SymbolicCreate2Args {
    function checkCreate2ConstructorArg(uint256 x) public {
        CreatedImmutable created = new CreatedImmutable{salt: bytes32(uint256(7))}(x);
        assert(created.get() == x);
        assert(address(created).code.length > 0);
    }
}
"#,
    );

    prj.add_test(
        "SymbolicCreate2SelfAddress.t.sol",
        r#"
contract CreatedSelfAddress {
    address public constructorSelf;

    constructor() {
        constructorSelf = address(this);
    }

    function runtimeSelf() external view returns (address) {
        return address(this);
    }
}

contract SymbolicCreate2SelfAddress {
    function checkCreate2SelfAddress(uint256 salt) public {
        CreatedSelfAddress created = new CreatedSelfAddress{salt: bytes32(salt)}();
        assert(created.constructorSelf() == address(created));
        assert(created.runtimeSelf() == address(created));
    }
}
"#,
    );

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkCreateConstructorArg|checkCreateInitcodeOffset|checkCreateInitcodeSize|checkCreate2ConstructorArg|checkCreate2SelfAddress)\\(",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicCreateConstructorArgs.t.sol:SymbolicCreateConstructorArgs
[PASS] checkCreateConstructorArg(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCreate2SelfAddress.t.sol:SymbolicCreate2SelfAddress
[PASS] checkCreate2SelfAddress(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCreateInitcodeSize.t.sol:SymbolicCreateInitcodeSize
[PASS] checkCreateInitcodeSize(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCreateInitcodeOffset.t.sol:SymbolicCreateInitcodeOffset
[PASS] checkCreateInitcodeOffset(uint16) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCreate2Args.t.sol:SymbolicCreate2Args
[PASS] checkCreate2ConstructorArg(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]].unordered())
    .get_output()
    .stdout_lossy();
    for reason in [
        "symbolic CALL target",
        "symbolic CREATE initcode offset",
        "symbolic CREATE initcode size",
        "symbolic CREATE2 initcode",
        "symbolic CREATE2 salt",
        "symbolic bytecode opcode",
        "unsupported symbolic execution feature: symbolic CREATE initcode",
    ] {
        assert!(!stdout.contains(reason), "{stdout}");
    }
}

#[forgetest_init]
fn symbolic_create_size_respects_path_width(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_size_respects_path_width because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreateSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCreateSize is Test {
    function checkCreateSizeRespectsPathWidth(uint256 size) public {
        vm.assume(size <= 2);
        bytes memory code = hex"5b5b";
        address created;
        assembly {
            created := create(0, add(code, 0x20), size)
        }
        assert(created != address(0));
    }
}
"#,
    );

    let stdout = assert_symbolic_engine_witness(cmd.args([
        "test",
        "--symbolic",
        "--symbolic-width",
        "2",
        "--match-test",
        "checkCreateSizeRespectsPathWidth",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicCreateSize.t.sol:SymbolicCreateSize
[FAIL: incomplete symbolic execution (Stuck): unsupported symbolic execution feature: symbolic path limit exceeded] checkCreateSizeRespectsPathWidth(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();
    // `[METRICS]` hides the path count that `--symbolic-width 2` must cap.
    assert!(stdout.contains("(paths: 2,"), "{stdout}");
}

#[forgetest_init]
fn symbolic_create2_deploys_and_calls_helper(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create2_deploys_and_calls_helper because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreate2.t.sol",
        r#"
contract CreatedHelper {
    function inc(uint256 x) external pure returns (uint256) {
        return x + 1;
    }
}

contract SymbolicCreate2 {
    function checkCreate2(uint256 x) public {
        CreatedHelper helper = new CreatedHelper{salt: bytes32(uint256(123))}();
        assert(helper.inc(x) != 11);
    }
}
"#,
    );

    let stdout =
        assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkCreate2"]))
            .failure()
            .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicCreate2.t.sol:SymbolicCreate2
[FAIL: panic: assertion failed (0x01); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkCreate2(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]])
            .get_output()
            .stdout_lossy();

    assert!(!stdout.contains("unsupported opcode: 0xf5"), "{stdout}");
}

#[forgetest_init]
fn symbolic_vm_expect_create_matches_and_reports_missing(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_create_matches_and_reports_missing because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectCreate.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedCreateTarget {
    function ping() external pure returns (uint256) {
        return 1;
    }
}

contract SymbolicExpectCreate is Test {
    function checkCreateExpectation(uint256) public {
        vm.expectCreate(type(SymbolicExpectedCreateTarget).runtimeCode, address(this));
        SymbolicExpectedCreateTarget target = new SymbolicExpectedCreateTarget();
        assertEq(target.ping(), 1);
    }

    function checkCreate2Expectation(uint256) public {
        vm.expectCreate2(type(SymbolicExpectedCreateTarget).runtimeCode, address(this));
        SymbolicExpectedCreateTarget target = new SymbolicExpectedCreateTarget{salt: bytes32(uint256(99))}();
        assertEq(target.ping(), 1);
    }

    function checkSymbolicCreateExpectation(address deployer) public {
        vm.assume(deployer == address(this));
        vm.expectCreate(type(SymbolicExpectedCreateTarget).runtimeCode, deployer);
        SymbolicExpectedCreateTarget target = new SymbolicExpectedCreateTarget();
        assertEq(target.ping(), 1);
    }

    function checkMismatchedSymbolicCreateExpectation(address deployer) public {
        vm.assume(deployer != address(this));
        vm.expectCreate(type(SymbolicExpectedCreateTarget).runtimeCode, deployer);
        new SymbolicExpectedCreateTarget();
    }

    function checkMissingCreateExpectation(uint256) public {
        vm.expectCreate(type(SymbolicExpectedCreateTarget).runtimeCode, address(this));
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkCreateExpectation",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicExpectCreate.t.sol:SymbolicExpectCreate
[PASS] checkCreateExpectation(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    assert_symbolic_witness(prj.forge_command().args([
        "test",
        "--symbolic",
        "--match-test",
        "checkCreate2Expectation",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicExpectCreate.t.sol:SymbolicExpectCreate
[PASS] checkCreate2Expectation(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    assert_symbolic_witness(prj.forge_command().args([
        "test",
        "--symbolic",
        "--match-test",
        "checkSymbolicCreateExpectation",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicExpectCreate.t.sol:SymbolicExpectCreate
[PASS] checkSymbolicCreateExpectation(address) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    assert_symbolic_witness(prj.forge_command().args([
        "test",
        "--symbolic",
        "--match-test",
        "checkMismatchedSymbolicCreateExpectation",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicExpectCreate.t.sol:SymbolicExpectCreate
[FAIL: expected CREATE call by address 0xffffffffffffffffffffffffffffffffffffffff for bytecode 0x[..] but not found; counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkMismatchedSymbolicCreateExpectation(address) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);

    assert_symbolic_witness(prj.forge_command().args([
        "test",
        "--symbolic",
        "--match-test",
        "checkMissingCreateExpectation",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicExpectCreate.t.sol:SymbolicExpectCreate
[FAIL: expected CREATE call by address 0x7fa9385be102ac3eac297483dd6233d62b3e1496 for bytecode 0x[..] but not found; counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkMissingCreateExpectation(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

// Failed creations: CREATE2 collisions, nonce bumps and revert data.
#[forgetest_init]
fn symbolic_create_failure_semantics(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_create_failure_semantics");

    prj.add_test(
        "SymbolicCreate2Collision.t.sol",
        r#"
import "forge-std/Test.sol";

contract CreatedHelper {}

contract SymbolicCreate2Collision is Test {
    function checkCreate2Collision() public {
        uint64 beforeNonce = vm.getNonce(address(this));
        bytes memory code = type(CreatedHelper).creationCode;
        address first;
        address second;
        assembly {
            first := create2(0, add(code, 0x20), mload(code), 1)
            second := create2(0, add(code, 0x20), mload(code), 1)
        }
        assert(first != address(0));
        assert(second == address(0));
        assert(vm.getNonce(address(this)) == beforeNonce + 2);
    }
}
"#,
    );

    prj.add_test(
        "SymbolicCreateFailureNonce.t.sol",
        r#"
import "forge-std/Test.sol";

contract RevertingCreate {
    constructor() {
        revert();
    }
}

contract SymbolicCreateFailureNonce is Test {
    function checkCreateFailureNonce(uint256) public {
        uint64 beforeNonce = vm.getNonce(address(this));
        try new RevertingCreate() {
            assert(false);
        } catch {}
        assert(vm.getNonce(address(this)) == beforeNonce + 1);
    }
}
"#,
    );

    prj.add_test(
        "SymbolicCreateRevertData.t.sol",
        r#"
contract SymbolicCreateRevertData {
    function checkCreateRevertData() public {
        bytes memory initcode = hex"61123460005260206000fd";
        address created;
        uint256 size;
        uint256 payload;
        assembly {
            created := create(0, add(initcode, 0x20), mload(initcode))
            size := returndatasize()
            returndatacopy(0x80, 0, size)
            payload := mload(0x80)
        }
        assert(created == address(0));
        assert(size == 32);
        assert(payload == 0x1234);
    }
}
"#,
    );

    assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkCreate2Collision|checkCreateFailureNonce|checkCreateRevertData)\\(",
    ]))
    .success()
    .stdout_eq(
        str![[r#"
...
Ran 1 test for test/SymbolicCreateRevertData.t.sol:SymbolicCreateRevertData
[PASS] checkCreateRevertData() ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCreate2Collision.t.sol:SymbolicCreate2Collision
[PASS] checkCreate2Collision() ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCreateFailureNonce.t.sol:SymbolicCreateFailureNonce
[PASS] checkCreateFailureNonce(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]
        .unordered(),
    );
}

#[forgetest_init]
fn symbolic_compute_create_address_cheatcodes(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_compute_create_address_cheatcodes because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicComputeCreateAddresses.t.sol",
        r#"
import "forge-std/Test.sol";

contract CreatedHelper {}

contract SymbolicComputeCreateAddresses is Test {
    address constant DEFAULT_CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

    function checkComputeCreateAddress(uint256) public {
        uint64 nonce = vm.getNonce(address(this));
        address expected = vm.computeCreateAddress(address(this), nonce);
        CreatedHelper created = new CreatedHelper();
        assert(address(created) == expected);
    }

    function checkSymbolicComputeCreateAddress(uint64 nonce) public {
        address first = vm.computeCreateAddress(address(this), nonce);
        address second = vm.computeCreateAddress(address(this), nonce);

        assert(first == second);
        assert(uint160(first) <= type(uint160).max);
    }

    function checkSymbolicComputeCreateAddressDeployer(address deployer, uint64 nonce) public {
        address first = vm.computeCreateAddress(deployer, nonce);
        address second = vm.computeCreateAddress(deployer, nonce);

        assert(first == second);
        assert(uint160(first) <= type(uint160).max);
    }

    function checkComputeCreate2Address(uint256 saltValue) public {
        bytes32 salt = bytes32(saltValue);
        bytes memory code = type(CreatedHelper).creationCode;
        address expected = vm.computeCreate2Address(salt, keccak256(code), address(this));
        address created;
        assembly {
            created := create2(0, add(code, 0x20), mload(code), salt)
        }
        assert(created == expected);
    }

    function checkSymbolicComputeCreate2Address(bytes32 salt, bytes32 initCodeHash) public {
        address first = vm.computeCreate2Address(salt, initCodeHash, address(this));
        address second = vm.computeCreate2Address(salt, initCodeHash, address(this));

        assert(first == second);
        assert(uint160(first) <= type(uint160).max);
    }

    function checkSymbolicComputeCreate2AddressDeployer(
        address deployer,
        bytes32 salt,
        bytes32 initCodeHash
    ) public {
        address first = vm.computeCreate2Address(salt, initCodeHash, deployer);
        address second = vm.computeCreate2Address(salt, initCodeHash, deployer);

        assert(first == second);
        assert(uint160(first) <= type(uint160).max);
    }

    function checkComputeCreate2DefaultDeployer() public {
        bytes memory code = type(CreatedHelper).creationCode;
        bytes32 salt = bytes32(uint256(1));
        bytes32 initCodeHash = keccak256(code);
        address expected = vm.computeCreate2Address(salt, initCodeHash);
        address manual = address(uint160(uint256(keccak256(abi.encodePacked(
            bytes1(0xff), DEFAULT_CREATE2_DEPLOYER, salt, initCodeHash
        )))));
        assert(expected == manual);
    }
}
"#,
    );

    let stdout = assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-contract",
        "SymbolicComputeCreateAddresses",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 7 tests for test/SymbolicComputeCreateAddresses.t.sol:SymbolicComputeCreateAddresses
[PASS] checkComputeCreate2Address(uint256) ([METRICS])
[PASS] checkComputeCreate2DefaultDeployer() ([METRICS])
[PASS] checkComputeCreateAddress(uint256) ([METRICS])
[PASS] checkSymbolicComputeCreate2Address(bytes32,bytes32) ([METRICS])
[PASS] checkSymbolicComputeCreate2AddressDeployer(address,bytes32,bytes32) ([METRICS])
[PASS] checkSymbolicComputeCreateAddress(uint64) ([METRICS])
[PASS] checkSymbolicComputeCreateAddressDeployer(address,uint64) ([METRICS])
Suite result: ok. 7 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]])
    .get_output()
    .stdout_lossy();

    assert!(
        stdout
            .contains("[PASS] checkSymbolicComputeCreate2AddressDeployer(address,bytes32,bytes32)"),
        "{stdout}"
    );
    assert!(!stdout.contains("symbolic vm.computeCreateAddress nonce"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.computeCreate2Address init code hash"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.computeCreateAddress deployer"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.computeCreate2Address deployer"), "{stdout}");
}

#[forgetest_init]
fn symbolic_vm_nonce_cheatcodes(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_vm_nonce_cheatcodes because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicNonceCheatcodes.t.sol",
        r#"
import "forge-std/Test.sol";

contract NonceTarget {}

contract SymbolicNonceCheatcodes is Test {
    function checkSetNonceCheatcodes(uint256) public {
        address account = address(0x1234);
        assertEq(vm.getNonce(account), 0);

        vm.setNonce(account, 7);
        assertEq(vm.getNonce(account), 7);

        vm.setNonceUnsafe(account, 2);
        assertEq(vm.getNonce(account), 2);
    }

    function checkResetNonceCheatcode(uint256) public {
        address account = address(0xbeef);
        vm.setNonce(account, 3);
        vm.resetNonce(account);
        assertEq(vm.getNonce(account), 0);

        NonceTarget target = new NonceTarget();
        vm.setNonce(address(target), 9);
        vm.resetNonce(address(target));
        assertEq(vm.getNonce(address(target)), 1);
    }

}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkSetNonceCheatcodes|checkResetNonceCheatcode",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 2 tests for test/SymbolicNonceCheatcodes.t.sol:SymbolicNonceCheatcodes
[PASS] checkResetNonceCheatcode(uint256) ([METRICS])
[PASS] checkSetNonceCheatcodes(uint256) ([METRICS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_vm_set_nonce_rejects_decrement(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_set_nonce_rejects_decrement because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSetNonceRejectsDecrement.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicSetNonceRejectsDecrement is Test {
    function checkSetNonceRejectsDecrement(uint256) public {
        address account = address(0xcafe);
        vm.setNonce(account, 4);
        vm.setNonce(account, 3);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkSetNonceRejectsDecrement",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicSetNonceRejectsDecrement.t.sol:SymbolicSetNonceRejectsDecrement
[FAIL: vm.setNonce: new nonce (3) must be strictly equal to or higher than the account's current nonce (4); counterexample: 		[SENDER] [SENDER] [CALLDATA] [ARGS]] checkSetNonceRejectsDecrement(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

// CREATE and CREATE2 with concrete, symbolic and insufficient value.
#[forgetest_init]
fn symbolic_create_value_transfers(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_create_value_transfers");

    prj.add_test(
        "SymbolicCreateValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract PayableCreated {
    constructor() payable {}
}

contract SymbolicCreateValue is Test {
    function checkCreateValue() public {
        vm.deal(address(this), 1);
        bytes memory code = type(PayableCreated).creationCode;
        address first;
        address second;
        assembly {
            first := create(1, add(code, 0x20), mload(code))
            second := create(2, add(code, 0x20), mload(code))
        }
        assert(first != address(0));
        assert(first.balance == 1);
        assert(second == address(0));
    }
}
"#,
    );

    prj.add_test(
        "SymbolicCreateSymbolicValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract PayableCreatedWithValue {
    uint256 public paid;

    constructor() payable {
        paid = msg.value;
    }
}

contract SymbolicCreateSymbolicValue is Test {
    function checkCreateSymbolicValue(uint256 amount) public {
        vm.assume(amount <= 5);
        vm.deal(address(this), 5);

        PayableCreatedWithValue created = new PayableCreatedWithValue{value: amount}();

        assertEq(created.paid(), amount);
        assertEq(address(created).balance, amount);
        assertEq(address(this).balance, 5 - amount);
    }
}
"#,
    );

    prj.add_test(
        "SymbolicCreateInsufficientValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract PayableCreatedForInsufficient {
    constructor() payable {}
}

contract SymbolicCreateInsufficientValue is Test {
    function checkCreateInsufficientValue(uint256 amount) public {
        vm.assume(amount <= 2);
        vm.deal(address(this), 1);
        bytes memory code = type(PayableCreatedForInsufficient).creationCode;

        address created;
        assembly {
            created := create(amount, add(code, 0x20), mload(code))
        }

        bool ok = created != address(0);
        assertEq(ok, amount <= 1);
        assertEq(address(this).balance, ok ? 1 - amount : 1);
        if (ok) {
            assertEq(created.balance, amount);
        }
    }
}
"#,
    );

    prj.add_test(
        "SymbolicCreate2SymbolicValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract PayableCreate2WithValue {
    uint256 public paid;

    constructor() payable {
        paid = msg.value;
    }
}

contract SymbolicCreate2SymbolicValue is Test {
    function checkCreate2SymbolicValue(uint256 amount) public {
        vm.assume(amount <= 5);
        vm.deal(address(this), 5);

        PayableCreate2WithValue created =
            new PayableCreate2WithValue{salt: bytes32(uint256(0x1234)), value: amount}();

        assertEq(created.paid(), amount);
        assertEq(address(created).balance, amount);
        assertEq(address(this).balance, 5 - amount);
    }
}
"#,
    );

    let stdout = assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "^(checkCreateValue|checkCreateSymbolicValue|checkCreateInsufficientValue|checkCreate2SymbolicValue)\\(",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicCreateValue.t.sol:SymbolicCreateValue
[PASS] checkCreateValue() ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCreate2SymbolicValue.t.sol:SymbolicCreate2SymbolicValue
[PASS] checkCreate2SymbolicValue(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCreateSymbolicValue.t.sol:SymbolicCreateSymbolicValue
[PASS] checkCreateSymbolicValue(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/SymbolicCreateInsufficientValue.t.sol:SymbolicCreateInsufficientValue
[PASS] checkCreateInsufficientValue(uint256) ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]].unordered())
    .get_output()
    .stdout_lossy();
    for reason in ["symbolic CREATE balance", "symbolic CREATE value"] {
        assert!(!stdout.contains(reason), "{stdout}");
    }
}

#[forgetest_init]
fn symbolic_staticcall_rejects_create(prj: _, cmd: _) {
    if !z3_available() {
        let _ =
            sh_eprintln!("skipping symbolic_staticcall_rejects_create because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicStaticCreate.t.sol",
        r#"
contract CreatedHelper {}

contract Creator {
    function deploy() external returns (address created) {
        bytes memory code = type(CreatedHelper).creationCode;
        assembly {
            created := create(0, add(code, 0x20), mload(code))
        }
    }
}

contract SymbolicStaticCreate {
    Creator creator;

    function setUp() public {
        creator = new Creator();
    }

    function checkStaticCreate() public view {
        (bool ok,) = address(creator).staticcall(abi.encodeWithSelector(Creator.deploy.selector));
        assert(!ok);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkStaticCreate"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicStaticCreate.t.sol:SymbolicStaticCreate
[PASS] checkStaticCreate() ([METRICS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

// CREATE whose constructor returns a symbolic-length runtime image must fail
// closed as Unsupported instead of silently installing a max-length padded
// bytecode (which would corrupt EXTCODESIZE, selector dispatch, and later
// execution). The constructor below returns `len` bytes; with symbolic `len`
// the engine must report the unsupported feature.
#[forgetest_init]
fn symbolic_create_with_symbolic_runtime_size_reports_unsupported(prj: _, cmd: _) {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_with_symbolic_runtime_size_reports_unsupported because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCreateRuntimeLen.t.sol",
        r#"
contract VariableLengthCtor {
    constructor(uint256 len) {
        assembly {
            // Write a STOP byte at memory 0 so the returned data is well-formed,
            // then return `len` bytes — a symbolic-length runtime image.
            mstore8(0, 0x00)
            return(0, len)
        }
    }
}

contract SymbolicCreateRuntimeLen {
    function checkCreateSymbolicRuntimeLen(uint256 len) public {
        new VariableLengthCtor(len);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkCreateSymbolicRuntimeLen",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicCreateRuntimeLen.t.sol:SymbolicCreateRuntimeLen
[FAIL: incomplete symbolic execution (Stuck): unsupported symbolic execution feature: symbolic RETURN size] checkCreateSymbolicRuntimeLen(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);

    // The engine fails closed at the constructor's RETURN with symbolic size
    // (upstream of the CREATE installation step). Either failure mode proves
    // the runtime image is never silently installed as max-length bytecode.
}
