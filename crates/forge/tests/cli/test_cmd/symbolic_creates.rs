use super::symbolic_helpers::assert_relevant_lines;
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, util::OutputExt};

use super::symbolic_helpers::z3_available;

forgetest_init!(symbolic_create_deploys_and_calls_helper, |prj, cmd| {
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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate"])
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
checkCreate(uint256)
"#]],
    );
    assert!(!stdout.contains("unsupported opcode: 0xf0"), "{stdout}");
});

forgetest_init!(symbolic_create_preserves_symbolic_constructor_args, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_preserves_symbolic_constructor_args because z3 is not available"
        );
        return;
    }

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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateConstructorArg"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreateConstructorArg(uint256)
"#]],
    );
    assert!(
        !stdout.contains("unsupported symbolic execution feature: symbolic CREATE initcode"),
        "{stdout}"
    );
});

forgetest_init!(symbolic_create_accepts_constrained_symbolic_initcode_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_accepts_constrained_symbolic_initcode_offset because z3 is not available"
        );
        return;
    }

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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateInitcodeOffset"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreateInitcodeOffset(uint16)
"#]],
    );
    assert!(!stdout.contains("symbolic CREATE initcode offset"), "{stdout}");
    assert!(!stdout.contains("symbolic bytecode opcode"), "{stdout}");
});

forgetest_init!(symbolic_create2_deploys_and_calls_helper, |prj, cmd| {
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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate2"])
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
checkCreate2(uint256)
"#]],
    );
    assert!(!stdout.contains("unsupported opcode: 0xf5"), "{stdout}");
});

forgetest_init!(symbolic_create2_preserves_symbolic_constructor_args, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create2_preserves_symbolic_constructor_args because z3 is not available"
        );
        return;
    }

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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate2ConstructorArg"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreate2ConstructorArg(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic CREATE2 initcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_create_matches_and_reports_missing, |prj, cmd| {
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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateExpectation"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreateExpectation(uint256)
"#]],
    );

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkCreate2Expectation"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreate2Expectation(uint256)
"#]],
    );

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCreateExpectation"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicCreateExpectation(address)
"#]],
    );

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMismatchedSymbolicCreateExpectation"])
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
checkMismatchedSymbolicCreateExpectation(address)
"#]],
    );

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMissingCreateExpectation"])
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
checkMissingCreateExpectation(uint256)
"#]],
    );
});

forgetest_init!(symbolic_create2_supports_symbolic_salt_and_self_address, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create2_supports_symbolic_salt_and_self_address because z3 is not available"
        );
        return;
    }

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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate2SelfAddress"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreate2SelfAddress(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic CREATE2 salt"), "{stdout}");
    assert!(!stdout.contains("symbolic CALL target"), "{stdout}");
});

forgetest_init!(symbolic_create2_collision_returns_zero, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create2_collision_returns_zero because z3 is not available"
        );
        return;
    }

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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate2Collision"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreate2Collision()
"#]],
    );
});

forgetest_init!(symbolic_create_failure_bumps_creator_nonce, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_failure_bumps_creator_nonce because z3 is not available"
        );
        return;
    }

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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateFailureNonce"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreateFailureNonce(uint256)
"#]],
    );
});

forgetest_init!(symbolic_compute_create_address_cheatcodes, |prj, cmd| {
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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicComputeCreateAddresses"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkComputeCreateAddress(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicComputeCreateAddress(uint64)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicComputeCreateAddressDeployer(address,uint64)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkComputeCreate2Address(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicComputeCreate2Address(bytes32,bytes32)
"#]],
    );
    assert!(
        stdout
            .contains("[PASS] checkSymbolicComputeCreate2AddressDeployer(address,bytes32,bytes32)"),
        "{stdout}"
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkComputeCreate2DefaultDeployer()
"#]],
    );
    assert!(!stdout.contains("symbolic vm.computeCreateAddress nonce"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.computeCreate2Address init code hash"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.computeCreateAddress deployer"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.computeCreate2Address deployer"), "{stdout}");
});

forgetest_init!(symbolic_vm_nonce_cheatcodes, |prj, cmd| {
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

    let stdout = cmd
        .args([
            "test",
            "--symbolic",
            "--match-test",
            "checkSetNonceCheatcodes|checkResetNonceCheatcode",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSetNonceCheatcodes(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkResetNonceCheatcode(uint256)
"#]],
    );
});

forgetest_init!(symbolic_vm_set_nonce_rejects_decrement, |prj, cmd| {
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

    let output = cmd
        .args(["test", "--symbolic", "--match-test", "checkSetNonceRejectsDecrement"])
        .assert_failure()
        .get_output()
        .clone();
    let output = format!("{}{}", output.stdout_lossy(), output.stderr_lossy());

    assert_relevant_lines(
        &output,
        foundry_test_utils::str![[r#"
[FAIL
"#]],
    );
    assert_relevant_lines(
        &output,
        foundry_test_utils::str![[r#"
checkSetNonceRejectsDecrement(uint256)
"#]],
    );
});

forgetest_init!(symbolic_create_transfers_value_and_checks_balance, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_transfers_value_and_checks_balance because z3 is not available"
        );
        return;
    }

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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreateValue()
"#]],
    );
});

forgetest_init!(symbolic_create_accepts_symbolic_value, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_accepts_symbolic_value because z3 is not available"
        );
        return;
    }

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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateSymbolicValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreateSymbolicValue(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic CREATE value"), "{stdout}");
});

forgetest_init!(symbolic_create_splits_symbolic_insufficient_value, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_splits_symbolic_insufficient_value because z3 is not available"
        );
        return;
    }

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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateInsufficientValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreateInsufficientValue(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic CREATE value"), "{stdout}");
    assert!(!stdout.contains("symbolic CREATE balance"), "{stdout}");
});

forgetest_init!(symbolic_create2_accepts_symbolic_value, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create2_accepts_symbolic_value because z3 is not available"
        );
        return;
    }

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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreate2SymbolicValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreate2SymbolicValue(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic CREATE value"), "{stdout}");
});

forgetest_init!(symbolic_create_accepts_bounded_symbolic_initcode_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_create_accepts_bounded_symbolic_initcode_size because z3 is not available"
        );
        return;
    }

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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateInitcodeSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCreateInitcodeSize(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic CREATE initcode size"), "{stdout}");
    assert!(!stdout.contains("symbolic bytecode opcode"), "{stdout}");
});

forgetest_init!(symbolic_staticcall_rejects_create, |prj, cmd| {
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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkStaticCreate"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkStaticCreate()
"#]],
    );
});

// CREATE whose constructor returns a symbolic-length runtime image must fail
// closed as Unsupported instead of silently installing a max-length padded
// bytecode (which would corrupt EXTCODESIZE, selector dispatch, and later
// execution). The constructor below returns `len` bytes; with symbolic `len`
// the engine must report the unsupported feature.
forgetest_init!(symbolic_create_with_symbolic_runtime_size_reports_unsupported, |prj, cmd| {
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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCreateSymbolicRuntimeLen"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    // The engine fails closed at the constructor's RETURN with symbolic size
    // (upstream of the CREATE installation step). Either failure mode proves
    // the runtime image is never silently installed as max-length bytecode.
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
unsupported symbolic execution feature: symbolic RETURN size
"#]],
    );
});
