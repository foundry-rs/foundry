use super::symbolic_helpers::assert_relevant_lines;
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, util::OutputExt};

use super::symbolic_helpers::z3_available;
use crate::skip_unless_z3;

forgetest_init!(symbolic_cheatcodes_accept_symbolic_address_targets, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_cheatcodes_accept_symbolic_address_targets because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicAddressCheatcodes.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicAddressCheatcodes is Test {
    function checkSymbolicDealStoreLoadAndNonce(address who, bytes32 value) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.deal(who, 11);
        assertEq(who.balance, 11);

        bytes32 slot = bytes32(uint256(1));
        vm.store(who, slot, value);
        assertEq(vm.load(who, slot), value);

        assertEq(vm.getNonce(who), 0);
        vm.setNonceUnsafe(who, 5);
        assertEq(vm.getNonce(who), 5);
        vm.resetNonce(who);
        assertEq(vm.getNonce(who), 0);
    }

    function checkSymbolicEtch(address who) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.etch(who, hex"00");
        assertEq(who.code.length, 1);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicAddressCheatcodes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicDealStoreLoadAndNonce(address,bytes32)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicEtch(address)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.deal target"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.store target"), "{stdout}");
    assert!(!stdout.contains("symbolic EXTCODESIZE target"), "{stdout}");
});

forgetest_init!(symbolic_prank_accepts_symbolic_sender, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_prank_accepts_symbolic_sender because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPrankSender.t.sol",
        r#"
import "forge-std/Test.sol";

contract SenderTarget {
    function sender() external view returns (address) {
        return msg.sender;
    }

    function origin() external view returns (address) {
        return tx.origin;
    }

    function context() external view returns (address, address) {
        return (msg.sender, tx.origin);
    }
}

contract SymbolicPrankSender is Test {
    SenderTarget target;

    function setUp() public {
        target = new SenderTarget();
    }

    function checkSymbolicPrank(address who) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.prank(who);
        assertEq(target.sender(), who);
    }

    function checkSymbolicStartPrank(address who) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.startPrank(who);
        assertEq(target.sender(), who);
        assertEq(target.sender(), who);
        vm.stopPrank();
    }

    function checkSymbolicPrankOrigin(address who, address origin) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.prank(who, origin);
        (address actualSender, address actualOrigin) = target.context();
        assertEq(actualSender, who);
        assertEq(actualOrigin, origin);
    }

    function checkSymbolicStartPrankOrigin(address who, address origin) public {
        vm.assume(who != address(0));
        vm.assume(who != address(this));
        vm.assume(who != address(vm));

        vm.startPrank(who, origin);
        assertEq(target.sender(), who);
        assertEq(target.origin(), origin);
        assertEq(target.sender(), who);
        assertEq(target.origin(), origin);
        vm.stopPrank();
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicPrankSender"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicPrank(address)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicStartPrank(address)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicPrankOrigin(address,address)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicStartPrankOrigin(address,address)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.prank"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.startPrank"), "{stdout}");
});

forgetest_init!(symbolic_balance_accepts_symbolic_target, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_balance_accepts_symbolic_target because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBalance.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBalance is Test {
    function checkSymbolicBalance(address who) public {
        address funded = address(0xBEEF);
        vm.deal(funded, 123);

        uint256 expected = who == funded ? 123 : 0;
        assertEq(who.balance, expected);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicBalance"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicBalance(address)
"#]],
    );
    assert!(!stdout.contains("symbolic BALANCE target"), "{stdout}");
});

forgetest_init!(symbolic_extcodesize_accepts_symbolic_target, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodesize_accepts_symbolic_target because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeSize is Test {
    function checkSymbolicCodeLength(address who) public {
        address coded = address(0xC0DE);
        vm.etch(coded, hex"60006000");

        uint256 expected = who == coded ? 4 : 0;
        assertEq(who.code.length, expected);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicExtcodeSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicCodeLength(address)
"#]],
    );
    assert!(!stdout.contains("symbolic EXTCODESIZE target"), "{stdout}");
});

forgetest_init!(symbolic_extcodehash_accepts_symbolic_target, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodehash_accepts_symbolic_target because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeHash.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeHash is Test {
    function checkSymbolicCodeHash(address who) public {
        address coded = address(0xC0DE);
        vm.etch(coded, hex"60006000");

        bytes32 expected = who == coded ? keccak256(hex"60006000") : bytes32(0);
        assertEq(who.codehash, expected);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicExtcodeHash"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicCodeHash(address)
"#]],
    );
    assert!(!stdout.contains("symbolic EXTCODEHASH target"), "{stdout}");
});

forgetest_init!(symbolic_extcodecopy_accepts_symbolic_target, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodecopy_accepts_symbolic_target because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeCopy.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeCopy is Test {
    function checkSymbolicExtcodeCopy(address who) public {
        address coded = address(0xC0DE);
        vm.etch(coded, hex"60016002");

        bytes32 copied;
        assembly {
            extcodecopy(who, 0x80, 0, 4)
            copied := mload(0x80)
        }

        bytes32 expected = who == coded ? bytes32(hex"6001600200000000000000000000000000000000000000000000000000000000") : bytes32(0);
        assertEq(copied, expected);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicExtcodeCopy"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicExtcodeCopy(address)
"#]],
    );
    assert!(!stdout.contains("symbolic EXTCODECOPY target"), "{stdout}");
});

forgetest_init!(symbolic_vm_prank_propagates_callers, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_prank_propagates_callers because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPrank.t.sol",
        r#"
import "forge-std/Test.sol";

contract CallerProbe {
    function callers() external view returns (address, address) {
        return (msg.sender, tx.origin);
    }
}

contract SymbolicPrank is Test {
    CallerProbe probe;

    function setUp() public {
        probe = new CallerProbe();
    }

    function checkPrank(uint256) public {
        address alice = address(0xA11CE);
        address bob = address(0xB0B);

        vm.prank(alice);
        (address sender,) = probe.callers();
        assertEq(sender, alice);

        (sender,) = probe.callers();
        assertEq(sender, address(this));

        vm.startPrank(alice, bob);
        address origin;
        (sender, origin) = probe.callers();
        assertEq(sender, alice);
        assertEq(origin, bob);

        vm.stopPrank();
        (sender,) = probe.callers();
        assertEq(sender, address(this));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkPrank"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkPrank(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_assert_cheatcodes_find_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_assert_cheatcodes_find_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicVmAssert.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicVmAssert is Test {
    function checkVmAssert(uint256 x) public {
        vm.assertNotEq(x, uint256(42));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkVmAssert"])
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
checkVmAssert(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
args=[42]
"#]],
    );
    assert!(!stdout.contains("counterexample did not replay"), "{stdout}");
});

forgetest_init!(symbolic_vm_recorded_logs_round_trip, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_recorded_logs_round_trip because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRecordedLogs.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicLogEmitter {
    event Helper(uint256 indexed topic, bytes data);

    function ok(uint256 topic, bytes memory data) external {
        emit Helper(topic, data);
    }

    function fail(uint256 topic, bytes memory data) external {
        emit Helper(topic, data);
        revert();
    }
}

contract SymbolicRecordedLogs is Test {
    event Local(uint256 indexed topic, bytes data);

    SymbolicLogEmitter emitter;

    function setUp() public {
        emitter = new SymbolicLogEmitter();
    }

    /// forge-config: default.symbolic.array_lengths = [2]
    function checkRecordedLogs(uint256 topic, bytes memory data) public {
        vm.recordLogs();

        emit Local(topic, data);
        emitter.ok(topic + 1, data);
        try emitter.fail(topic + 2, data) {} catch {}

        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertEq(logs.length, 2);

        assertEq(logs[0].topics.length, 2);
        assertEq(logs[0].topics[0], keccak256("Local(uint256,bytes)"));
        assertEq(logs[0].topics[1], bytes32(topic));
        assertEq(logs[0].emitter, address(this));

        bytes memory localData = abi.decode(logs[0].data, (bytes));
        assert(keccak256(localData) == keccak256(data));

        assertEq(logs[1].topics.length, 2);
        assertEq(logs[1].topics[0], keccak256("Helper(uint256,bytes)"));
        assertEq(logs[1].topics[1], bytes32(topic + 1));
        assertEq(logs[1].emitter, address(emitter));

        Vm.Log[] memory drained = vm.getRecordedLogs();
        assertEq(drained.length, 0);
    }

    /// forge-config: default.symbolic.array_lengths = [2]
    function checkRecordedLogsJson(uint256 topic, bytes memory data) public {
        vm.recordLogs();
        emit Local(topic, data);

        string memory json = vm.getRecordedLogsJson();
        assert(bytes(json).length > 0);

        Vm.Log[] memory drained = vm.getRecordedLogs();
        assertEq(drained.length, 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicRecordedLogs"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkRecordedLogs(uint256,bytes)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkRecordedLogsJson(uint256,bytes)
"#]],
    );
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.getRecordedLogsJson"), "{stdout}");
});

forgetest_init!(symbolic_vm_env_crypto_and_console_helpers, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_env_crypto_and_console_helpers because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicEnvCryptoConsole.t.sol",
        r#"
import "forge-std/Test.sol";
import "forge-std/console2.sol";

contract SymbolicEnvCryptoConsole is Test {
    function checkEnvCryptoConsole(uint256 x) public {
        assertTrue(vm.envExists("FOUNDRY_SYMBOLIC_ENV_PRESENT"));
        assertEq(vm.envUint("FOUNDRY_SYMBOLIC_ENV_UINT"), 42);
        assertEq(vm.envOr("FOUNDRY_SYMBOLIC_ENV_MISSING", uint256(7)), 7);
        assertEq(vm.envString("FOUNDRY_SYMBOLIC_ENV_STRING"), "hello");

        uint256[] memory values = vm.envUint("FOUNDRY_SYMBOLIC_ENV_UINTS", ",");
        assertEq(values.length, 3);
        assertEq(values[0], 1);
        assertEq(values[2], 3);

        string[] memory words = vm.envString("FOUNDRY_SYMBOLIC_ENV_STRINGS", ",");
        assertEq(words.length, 2);
        assertEq(words[1], "beta");

        bytes[] memory blobs = vm.envBytes("FOUNDRY_SYMBOLIC_ENV_BYTES_ARRAY", ",");
        assertEq(blobs.length, 2);
        assertEq(blobs[1], hex"cafe");

        uint256[] memory defaultValues = new uint256[](2);
        defaultValues[0] = 5;
        defaultValues[1] = 6;
        uint256[] memory missing = vm.envOr("FOUNDRY_SYMBOLIC_ENV_MISSING_ARRAY", ",", defaultValues);
        assertEq(missing.length, 2);
        assertEq(missing[1], 6);

        address keyAddress = vm.addr(1);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(1, keccak256("foundry-symbolic"));
        assertTrue(keyAddress != address(0));
        assertTrue(v == 27 || v == 28);
        assertTrue(r != bytes32(0));
        assertTrue(s != bytes32(0));

        console2.log("symbolic", x);
    }

    function checkKeyUtilities() public {
        address keyAddress = vm.addr(1);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(1, keccak256("foundry-symbolic"));
        assertTrue(keyAddress != address(0));
        assertTrue(v == 27 || v == 28);
        assertTrue(r != bytes32(0));
        assertTrue(s != bytes32(0));
        (bytes32 compactR, bytes32 vs) = vm.signCompact(1, keccak256("foundry-symbolic"));
        assertEq(compactR, r);
        assertTrue(vs != bytes32(0));
        address remembered = vm.rememberKey(2);
        assertEq(remembered, vm.addr(2));
        address[] memory wallets = vm.getWallets();
        assertEq(wallets.length, 1);
        assertEq(wallets[0], remembered);
        uint256 derived = vm.deriveKey("test test test test test test test test test test test junk", uint32(0));
        assertEq(vm.addr(derived), 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
        address[] memory derivedWallets = vm.rememberKeys(
            "test test test test test test test test test test test junk",
            "m/44'/60'/0'/0/",
            uint32(3)
        );
        assertEq(derivedWallets.length, 3);
        assertEq(derivedWallets[0], vm.addr(derived));
    }

    function checkBase64Utilities() public {
        assertEq(vm.toBase64(bytes("hello")), "aGVsbG8=");
        assertEq(vm.toBase64URL(hex"ffff"), "__8=");
    }

    function checkParseToStringUtilities() public {
        assertEq(vm.parseBytes("0x1234"), hex"1234");
        assertEq(vm.parseAddress(vm.toString(address(0xBEEF))), address(0xBEEF));
        assertEq(vm.parseUint(vm.toString(uint256(123))), 123);
        assertEq(vm.parseInt(vm.toString(int256(-5))), -5);
        assertEq(vm.parseBytes32(vm.toString(bytes32(uint256(0x12)))), bytes32(uint256(0x12)));
        assertTrue(vm.parseBool(vm.toString(true)));
    }

    function checkStringUtilities() public {
        assertEq(vm.toLowercase("AbC"), "abc");
        assertEq(vm.toUppercase("AbC"), "ABC");
        assertEq(vm.trim("  foundry  "), "foundry");
        assertEq(vm.replace("hello forge", "forge", "symbolic"), "hello symbolic");
        string[] memory parts = vm.split("a,b,c", ",");
        assertEq(parts.length, 3);
        assertEq(parts[1], "b");
        assertEq(vm.indexOf("foundry", "dry"), 4);
        assertTrue(vm.contains("foundry", "ound"));
    }
}
"#,
    );

    cmd.env("FOUNDRY_SYMBOLIC_ENV_PRESENT", "1");
    cmd.env("FOUNDRY_SYMBOLIC_ENV_UINT", "42");
    cmd.env("FOUNDRY_SYMBOLIC_ENV_STRING", "hello");
    cmd.env("FOUNDRY_SYMBOLIC_ENV_UINTS", "1,2,3");
    cmd.env("FOUNDRY_SYMBOLIC_ENV_STRINGS", "alpha,beta");
    cmd.env("FOUNDRY_SYMBOLIC_ENV_BYTES_ARRAY", "12,cafe");

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicEnvCryptoConsole"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkEnvCryptoConsole(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkKeyUtilities()
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkBase64Utilities()
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkParseToStringUtilities()
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkStringUtilities()
"#]],
    );
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_ffi_is_config_gated, |prj, cmd| {
    if !z3_available() {
        let _ =
            sh_eprintln!("skipping symbolic_vm_ffi_is_config_gated because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicFfiDisabled.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicFfiDisabled is Test {
    function checkFfiDisabled(uint256) public {
        string[] memory input = new string[](1);
        input[0] = "true";
        vm.ffi(input);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkFfiDisabled"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
symbolic ffi disabled
"#]],
    );
});

forgetest_init!(symbolic_vm_ffi_success_when_enabled, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_ffi_success_when_enabled because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicFfiEnabled.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicFfiEnabled is Test {
    function checkFfiEnabled(uint256) public {
        string[] memory input = new string[](3);
        input[0] = "sh";
        input[1] = "-c";
        input[2] = "printf 0x1234";

        bytes memory output = vm.ffi(input);
        assertEq(output.length, 2);
        assertEq(uint8(output[0]), 0x12);
        assertEq(uint8(output[1]), 0x34);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--ffi", "--match-test", "checkFfiEnabled"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkFfiEnabled(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic ffi disabled"), "{stdout}");
});

forgetest_init!(symbolic_vm_etch_and_get_deployed_code, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_etch_and_get_deployed_code because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicEtch.t.sol",
        r#"
import "forge-std/Test.sol";

interface IEtchedHelper {
    function value() external pure returns (uint256);
}

contract EtchedHelper {
    function value() external pure returns (uint256) {
        return 99;
    }
}

contract SymbolicEtch is Test {
    function checkEtch(uint256) public {
        address target = address(0xBEEF);
        bytes memory code = vm.getDeployedCode("SymbolicEtch.t.sol:EtchedHelper");
        vm.etch(target, code);

        assertGt(target.code.length, 0);
        assertEq(IEtchedHelper(target).value(), 99);
    }

    function checkEtchSymbolicBytes(uint8 value) public {
        address target = address(0xCAFE);
        bytes memory code = abi.encodePacked(bytes1(0x60), bytes1(value), bytes1(0x00));
        vm.etch(target, code);

        assertEq(target.code.length, 3);

        bytes memory copied = new bytes(3);
        assembly {
            extcodecopy(target, add(copied, 0x20), 0, 3)
        }

        assertEq(copied[0], bytes1(0x60));
        assertEq(copied[1], bytes1(value));
        assertEq(copied[2], bytes1(0x00));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicEtch"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkEtch(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkEtchSymbolicBytes(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.etch"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.getCode artifact"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_extcodehash_distinguishes_empty_existing_account, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodehash_distinguishes_empty_existing_account because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCodeHash.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCodeHash is Test {
    function checkCodeHash(uint256) public {
        address target = address(0xBEEF);
        vm.etch(target, hex"");

        bytes32 emptyHash = keccak256(new bytes(0));
        assert(target.code.length == 0);
        assert(target.codehash == emptyHash);
        assert(address(0xCAFE).codehash == bytes32(0));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCodeHash"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCodeHash(uint256)
"#]],
    );
});

forgetest_init!(symbolic_extcodecopy_pads_partial_code_ranges, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodecopy_pads_partial_code_ranges because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeCopy.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeCopy is Test {
    function checkExtcodeCopy(uint256) public {
        address target = address(0xBEEF);
        vm.etch(target, hex"010203");

        bytes memory copied = new bytes(5);
        assembly {
            extcodecopy(target, add(copied, 0x20), 1, 5)
        }

        assert(uint8(copied[0]) == 2);
        assert(uint8(copied[1]) == 3);
        assert(uint8(copied[2]) == 0);
        assert(uint8(copied[3]) == 0);
        assert(uint8(copied[4]) == 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExtcodeCopy"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkExtcodeCopy(uint256)
"#]],
    );
});

forgetest_init!(symbolic_codecopy_accepts_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_codecopy_accepts_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCodeCopy.t.sol",
        r#"
contract SymbolicCodeCopy {
    function checkCodeCopy(uint16 offset) public pure {
        uint256 copied;
        uint256 size;
        assembly {
            size := codesize()
            codecopy(0, offset, 1)
            copied := mload(0)
        }

        if (offset >= size) {
            assert(copied == 0);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCodeCopy"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCodeCopy(uint16)
"#]],
    );
    assert!(!stdout.contains("symbolic CODECOPY offset"), "{stdout}");
});

forgetest_init!(symbolic_extcodecopy_accepts_symbolic_offset, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodecopy_accepts_symbolic_offset because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeCopyOffset.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeCopyOffset is Test {
    function checkExtcodeCopyOffset(uint16 offset) public {
        address target = address(0xBEEF);
        vm.etch(target, hex"010203");

        bytes memory copied = new bytes(1);
        assembly {
            extcodecopy(target, add(copied, 0x20), offset, 1)
        }

        if (offset == 1) {
            assert(uint8(copied[0]) == 2);
        }
        if (offset >= 3) {
            assert(uint8(copied[0]) == 0);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExtcodeCopyOffset"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkExtcodeCopyOffset(uint16)
"#]],
    );
    assert!(!stdout.contains("symbolic EXTCODECOPY offset"), "{stdout}");
});

forgetest_init!(symbolic_codecopy_accepts_bounded_symbolic_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_codecopy_accepts_bounded_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCodeCopySize.t.sol",
        r#"
contract SymbolicCodeCopySize {
    function checkCodeCopySize(uint8 rawSize) public pure {
        uint256 size = uint256(rawSize & 1);
        uint256 first;
        uint256 copied;
        assembly {
            codecopy(0x80, 0, 1)
            first := byte(0, mload(0x80))
            codecopy(0xa0, 0, size)
            copied := byte(0, mload(0xa0))
        }

        if (size == 1) {
            assert(copied == first);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCodeCopySize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCodeCopySize(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic CODECOPY size"), "{stdout}");
});

forgetest_init!(symbolic_extcodecopy_accepts_bounded_symbolic_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_extcodecopy_accepts_bounded_symbolic_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExtcodeCopySize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExtcodeCopySize is Test {
    function checkExtcodeCopySize(uint8 rawSize) public {
        address target = address(0xBEEF);
        vm.etch(target, hex"010203");
        uint256 size = uint256(rawSize & 3);

        bytes memory copied = new bytes(3);
        assembly {
            extcodecopy(target, add(copied, 0x20), 0, size)
        }

        if (size == 3) {
            assertEq(uint8(copied[0]), 1);
            assertEq(uint8(copied[1]), 2);
            assertEq(uint8(copied[2]), 3);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExtcodeCopySize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkExtcodeCopySize(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic EXTCODECOPY size"), "{stdout}");
});

forgetest_init!(symbolic_selfdestruct_updates_account_overlay, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_selfdestruct_updates_account_overlay because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSelfdestruct.t.sol",
        r#"
import "forge-std/Test.sol";

/// forge-config: default.evm_version = "shanghai"

contract Killable {
    receive() external payable {}

    function die(address payable beneficiary) external {
        selfdestruct(beneficiary);
    }
}

contract SymbolicSelfdestruct is Test {
    Killable killable;
    address payable beneficiary = payable(address(0xB0B));

    function setUp() public {
        killable = new Killable();
    }

    function checkSelfdestruct(uint256) public {
        vm.deal(address(killable), 7);

        killable.die(beneficiary);

        assert(address(killable).balance == 0);
        assert(beneficiary.balance == 7);
        assert(address(killable).code.length == 0);
        assert(address(killable).codehash == bytes32(0));
        assert(vm.getNonce(address(killable)) == 1);
    }
}
"#,
    );

    let stdout = cmd
        .args([
            "test",
            "--symbolic",
            "--evm-version",
            "shanghai",
            "--match-test",
            "checkSelfdestruct",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSelfdestruct(uint256)
"#]],
    );
    assert!(!stdout.contains("SELFDESTRUCT/EIP-6780 not modeled"), "{stdout}");
});

forgetest_init!(
    symbolic_selfdestruct_cancun_symbolic_beneficiary_reports_incomplete,
    |prj, cmd| {
        if !z3_available() {
            let _ = sh_eprintln!(
                "skipping symbolic_selfdestruct_cancun_symbolic_beneficiary_reports_incomplete because z3 is not available"
            );
            return;
        }

        prj.add_test(
            "SymbolicSelfdestructBeneficiary.t.sol",
            r#"
import "forge-std/Test.sol";

/// forge-config: default.evm_version = "cancun"

contract Killable {
    receive() external payable {}

    function die(address payable beneficiary) external {
        selfdestruct(beneficiary);
    }
}

contract SymbolicSelfdestructBeneficiary is Test {
    Killable killable;

    function setUp() public {
        killable = new Killable();
    }

    function checkSelfdestructBeneficiary(address payable beneficiary) public {
        vm.assume(beneficiary != address(killable));
        vm.deal(address(killable), 7);

        killable.die(beneficiary);

        assert(address(killable).balance == 0);
        assert(beneficiary.balance == 7);
    }
}
"#,
        );

        let stdout = cmd
            .args(["test", "--symbolic", "--match-test", "checkSelfdestructBeneficiary"])
            .assert_failure()
            .get_output()
            .stdout_lossy();

        assert_relevant_lines(
            &stdout,
            foundry_test_utils::str![[r#"
[FAIL: incomplete symbolic execution (Stuck): unsupported symbolic execution feature: symbolic SELFDESTRUCT beneficiary] checkSelfdestructBeneficiary(address)
"#]],
        );
        assert!(!stdout.contains("SELFDESTRUCT/EIP-6780 not modeled"), "{stdout}");
        assert!(!stdout.contains("symbolic BALANCE target"), "{stdout}");
    }
);

forgetest_init!(symbolic_selfdestruct_cancun_existing_preserves_account, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_selfdestruct_cancun_existing_preserves_account because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSelfdestructCancunExisting.t.sol",
        r#"
import "forge-std/Test.sol";

/// forge-config: default.evm_version = "cancun"

contract KillableCancun {
    uint256 value = 42;

    receive() external payable {}

    function die(address payable beneficiary) external {
        selfdestruct(beneficiary);
    }

    function get() external view returns (uint256) {
        return value;
    }
}

contract SymbolicSelfdestructCancunExisting is Test {
    KillableCancun killable;
    address payable beneficiary = payable(address(0xB0B));

    function setUp() public {
        killable = new KillableCancun();
    }

    function checkCancunSelfdestructExisting(uint256) public {
        vm.deal(address(killable), 7);

        killable.die(beneficiary);

        assertEq(address(killable).balance, 0);
        assertEq(beneficiary.balance, 7);
        assertGt(address(killable).code.length, 0);
        assertEq(killable.get(), 42);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCancunSelfdestructExisting"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCancunSelfdestructExisting(uint256)
"#]],
    );
    assert!(!stdout.contains("SELFDESTRUCT/EIP-6780 not modeled"), "{stdout}");
});

forgetest_init!(symbolic_selfdestruct_cancun_same_transaction_deletes_account, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_selfdestruct_cancun_same_transaction_deletes_account because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSelfdestructCancunSameTx.t.sol",
        r#"
import "forge-std/Test.sol";

/// forge-config: default.evm_version = "cancun"

contract KillableCancunSameTx {
    receive() external payable {}

    function die(address payable beneficiary) external {
        selfdestruct(beneficiary);
    }
}

contract SymbolicSelfdestructCancunSameTx is Test {
    address payable beneficiary = payable(address(0xB0B));

    function checkCancunSelfdestructSameTransaction(uint256) public {
        KillableCancunSameTx killable = new KillableCancunSameTx();
        vm.deal(address(killable), 7);

        killable.die(beneficiary);

        assertEq(address(killable).balance, 0);
        assertEq(beneficiary.balance, 7);
        assertEq(address(killable).code.length, 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCancunSelfdestructSameTransaction"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkCancunSelfdestructSameTransaction(uint256)
"#]],
    );
    assert!(!stdout.contains("SELFDESTRUCT/EIP-6780 not modeled"), "{stdout}");
});

forgetest_init!(symbolic_selfdestruct_cancun_wrong_delete_assertion_fails, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_selfdestruct_cancun_wrong_delete_assertion_fails because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSelfdestructCancunWrongDelete.t.sol",
        r#"
import "forge-std/Test.sol";

/// forge-config: default.evm_version = "cancun"

contract KillableCancunWrongDelete {
    receive() external payable {}

    function die(address payable beneficiary) external {
        selfdestruct(beneficiary);
    }
}

contract SymbolicSelfdestructCancunWrongDelete is Test {
    KillableCancunWrongDelete killable;
    address payable beneficiary = payable(address(0xB0B));

    function setUp() public {
        killable = new KillableCancunWrongDelete();
    }

    function checkCancunSelfdestructDoesNotDeleteExisting(uint256) public {
        killable.die(beneficiary);

        assertEq(address(killable).code.length, 0);
    }
}
"#,
    );

    let stdout = cmd
        .args([
            "test",
            "--symbolic",
            "--match-test",
            "checkCancunSelfdestructDoesNotDeleteExisting",
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[FAIL"), "{stdout}");
    assert!(stdout.contains("counterexample"), "{stdout}");
    assert!(stdout.contains("checkCancunSelfdestructDoesNotDeleteExisting"), "{stdout}");
    assert!(!stdout.contains("[PASS] checkCancunSelfdestructDoesNotDeleteExisting"), "{stdout}");
    assert!(!stdout.contains("incomplete symbolic execution"), "{stdout}");
    assert!(!stdout.contains("SELFDESTRUCT/EIP-6780 not modeled"), "{stdout}");
});

forgetest_init!(symbolic_vm_set_blockhash, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_vm_set_blockhash because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicBlockhash.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBlockhash is Test {
    function checkSetBlockhash(uint256) public {
        bytes32 previousHash = bytes32(uint256(0x1234));
        vm.roll(300);
        vm.setBlockhash(299, previousHash);
        vm.setBlockhash(300, bytes32(uint256(0xdead)));
        vm.setBlockhash(43, bytes32(uint256(0xbeef)));

        assertEq(blockhash(299), previousHash);
        assertEq(blockhash(300), bytes32(0));
        assertEq(blockhash(43), bytes32(0));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSetBlockhash"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSetBlockhash(uint256)
"#]],
    );
});

forgetest_init!(symbolic_blockhash_accepts_symbolic_number, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_blockhash_accepts_symbolic_number because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBlockhashNumber.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBlockhashNumber is Test {
    function checkSymbolicBlockhashNumber(uint256 blockNumber) public {
        bytes32 previousHash = bytes32(uint256(0x1234));
        vm.roll(300);
        vm.setBlockhash(299, previousHash);

        bytes32 hash = blockhash(blockNumber);
        if (hash == previousHash) {
            assertEq(blockNumber, 299);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicBlockhashNumber"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicBlockhashNumber(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic BLOCKHASH number"), "{stdout}");
});

forgetest_init!(symbolic_vm_set_blockhash_accepts_symbolic_hash, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_set_blockhash_accepts_symbolic_hash because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBlockhashValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBlockhashValue is Test {
    function checkSymbolicBlockhashValue(bytes32 blockHash) public {
        vm.roll(300);
        vm.setBlockhash(299, blockHash);

        assertEq(blockhash(299), blockHash);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicBlockhashValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicBlockhashValue(bytes32)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.setBlockhash hash"), "{stdout}");
});

forgetest_init!(symbolic_vm_block_environment_breadth, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_block_environment_breadth because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBlockEnvironment.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBlockEnvironment is Test {
    function checkBlockEnvironment(uint256) public {
        bytes32 randomness = bytes32(uint256(0xabc));

        vm.warp(123);
        assertEq(vm.getBlockTimestamp(), 123);
        assertEq(block.timestamp, 123);

        vm.txGasPrice(7);
        assertEq(tx.gasprice, 7);

        vm.prevrandao(randomness);
        assertEq(block.prevrandao, uint256(randomness));

        vm.blobBaseFee(11);
        assertEq(block.blobbasefee, 11);
        assertEq(vm.getBlobBaseFee(), 11);

        bytes32[] memory hashes = new bytes32[](2);
        hashes[0] = bytes32(uint256(0x1111));
        hashes[1] = bytes32(uint256(0x2222));
        vm.blobhashes(hashes);

        assertEq(blobhash(0), hashes[0]);
        assertEq(blobhash(1), hashes[1]);
        assertEq(blobhash(2), bytes32(0));

        bytes32[] memory got = vm.getBlobhashes();
        assertEq(got.length, 2);
        assertEq(got[0], hashes[0]);
        assertEq(got[1], hashes[1]);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkBlockEnvironment"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkBlockEnvironment(uint256)
"#]],
    );
});

forgetest_init!(symbolic_uses_prepared_executor_environment, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_uses_prepared_executor_environment because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPreparedEnvironment.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicPreparedEnvironment is Test {
    function setUp() public {
        vm.chainId(424242);
        vm.roll(12345);
        vm.warp(67890);
        vm.fee(77);
        vm.prevrandao(bytes32(uint256(99)));
        vm.coinbase(address(0xBEEF));
        vm.txGasPrice(66);
    }

    function checkPreparedEnvironment(uint256 x) public {
        if (x > 1) return;

        assertEq(block.chainid, 424242);
        assertEq(block.number, 12345);
        assertEq(block.timestamp, 67890);
        assertEq(block.basefee, 77);
        assertEq(block.prevrandao, 99);
        assertEq(block.coinbase, address(0xBEEF));
        assertEq(tx.gasprice, 66);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkPreparedEnvironment"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkPreparedEnvironment(uint256)
"#]],
    );
});

forgetest_init!(symbolic_vm_state_snapshots, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_vm_state_snapshots because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicStateSnapshots.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicStateSnapshots is Test {
    uint256 value;

    function checkStateSnapshots(uint256) public {
        value = 1;
        vm.deal(address(this), 5 ether);

        uint256 snapshotId = vm.snapshotState();
        value = 2;
        vm.deal(address(this), 8 ether);

        assertTrue(vm.revertToState(snapshotId));
        assertEq(value, 1);
        assertEq(address(this).balance, 5 ether);

        assertTrue(vm.deleteStateSnapshot(snapshotId));
        assertFalse(vm.revertToState(snapshotId));

        uint256 legacySnapshot = vm.snapshot();
        value = 3;

        assertTrue(vm.revertToAndDelete(legacySnapshot));
        assertEq(value, 1);
        assertFalse(vm.revertTo(legacySnapshot));

        uint256 deletedSnapshot = vm.snapshotState();
        assertTrue(vm.deleteSnapshot(deletedSnapshot));
        assertFalse(vm.revertToState(deletedSnapshot));

        uint256 clearedSnapshot = vm.snapshotState();
        vm.deleteStateSnapshots();
        assertFalse(vm.revertToState(clearedSnapshot));
    }

    receive() external payable {}
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkStateSnapshots"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkStateSnapshots(uint256)
"#]],
    );
});

forgetest_init!(symbolic_vm_random_bytes, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_vm_random_bytes because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicRandomBytes.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicRandomBytes is Test {
    function checkRandomBytes(uint256) public {
        bytes memory data = vm.randomBytes(3);

        assertEq(data.length, 3);
        vm.assume(data[0] == bytes1(0x11));
        vm.assume(data[1] == bytes1(0x22));
        vm.assume(data[2] == bytes1(0x33));

        assertTrue(data[0] == bytes1(0x11));
        assertTrue(data[1] == bytes1(0x22));
        assertTrue(data[2] == bytes1(0x33));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkRandomBytes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkRandomBytes(uint256)
"#]],
    );
});

forgetest_init!(symbolic_vm_random_bytes_accepts_bounded_symbolic_length, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_random_bytes_accepts_bounded_symbolic_length because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRandomBytesLength.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicRandomBytesLength is Test {
    function checkRandomBytesSymbolicLength(uint8 n) public {
        uint256 len = uint256(n);
        vm.assume(len <= 3);

        bytes memory data = vm.randomBytes(len);

        assertEq(data.length, len);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkRandomBytesSymbolicLength"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkRandomBytesSymbolicLength(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic randomBytes len"), "{stdout}");
    assert!(!stdout.contains("symbolic randomBytes length"), "{stdout}");
});

forgetest_init!(symbolic_cheatcodes_accept_constrained_scalar_args, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_cheatcodes_accept_constrained_scalar_args because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicConstrainedCheatcodes.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicConstrainedCheatcodes is Test {
    function checkConstrainedDeal(address target, uint256 amount) public {
        vm.assume(target == address(0xbeef));
        vm.assume(amount == 7);

        vm.deal(target, amount);

        assertEq(address(0xbeef).balance, 7);
    }

    function checkSymbolicDealValueFundsCall(uint256 amount) public {
        address recipient = address(0xbeef);

        vm.deal(address(this), amount);
        assertEq(address(this).balance, amount);

        (bool ok,) = recipient.call{value: amount}("");

        assertTrue(ok);
        assertEq(recipient.balance, amount);
        assertEq(address(this).balance, 0);
    }

    function checkSymbolicDealInsufficientFunds(uint256 amount) public {
        vm.assume(amount < type(uint256).max);
        address recipient = address(0xbeef);

        vm.deal(address(this), amount);

        (bool ok,) = recipient.call{value: amount + 1}("");

        assertFalse(ok);
        assertEq(address(this).balance, amount);
        assertEq(recipient.balance, 0);
    }

    function checkConstrainedRandomBytes(uint16 len) public {
        vm.assume(len == 3);

        bytes memory data = vm.randomBytes(len);

        assertEq(data.length, 3);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicConstrainedCheatcodes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkConstrainedDeal(address,uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicDealValueFundsCall(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicDealInsufficientFunds(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkConstrainedRandomBytes(uint16)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.deal target"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.deal value"), "{stdout}");
    assert!(!stdout.contains("symbolic randomBytes len"), "{stdout}");
});

forgetest_init!(symbolic_cheatcodes_reject_gas_deal_value, |prj, cmd| {
    skip_unless_z3!("symbolic_cheatcodes_reject_gas_deal_value");

    prj.add_test(
        "SymbolicDealGasValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicDealGasValue is Test {
    function checkGasDealValue() public {
        vm.deal(address(this), gasleft());
    }

    function checkDerivedGasDealValue() public {
        vm.deal(address(this), gasleft() + 1);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkGasDealValue"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Stuck): unsupported symbolic execution feature: GAS/gasleft() not modeled
"#]],
    );
});

forgetest_init!(symbolic_cheatcodes_reject_derived_gas_deal_value, |prj, cmd| {
    skip_unless_z3!("symbolic_cheatcodes_reject_derived_gas_deal_value");

    prj.add_test(
        "SymbolicDerivedDealGasValue.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicDerivedDealGasValue is Test {
    function checkDerivedGasDealValue() public {
        vm.deal(address(this), gasleft() + 1);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkDerivedGasDealValue"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Stuck): unsupported symbolic execution feature: GAS/gasleft() not modeled
"#]],
    );
});

forgetest_init!(symbolic_cheatcodes_accept_bounded_symbolic_input_size, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_cheatcodes_accept_bounded_symbolic_input_size because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCheatcodeInputSize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCheatcodeInputSize is Test {
    function checkLowLevelAssumeSize(uint256 size, bool condition) public {
        vm.assume(size >= 36);
        vm.assume(size <= 68);

        bytes memory data = abi.encodeWithSelector(bytes4(keccak256("assume(bool)")), condition);
        address cheatcode = address(vm);
        bool ok;
        assembly {
            ok := call(gas(), cheatcode, 0, add(data, 32), size, 0, 0)
        }

        assert(ok);
        assertTrue(condition);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkLowLevelAssumeSize"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkLowLevelAssumeSize(uint256,bool)
"#]],
    );
    assert!(!stdout.contains("symbolic cheatcode CALL input size"), "{stdout}");
});

forgetest_init!(symbolic_svm_creator_breadth, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_svm_creator_breadth because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicSvmCreators.t.sol",
        r#"
interface Svm {
    function createUint8(string calldata name) external returns (uint8);
    function createInt16(string calldata name) external returns (int16);
    function createBytes2(string calldata name) external returns (bytes2);
    function createBytes(string calldata name) external returns (bytes memory);
    function createBytes(uint256 len, string calldata name) external returns (bytes memory);
    function createString(string calldata name) external returns (string memory);
    function createString(uint256 len, string calldata name) external returns (string memory);
}

contract SymbolicSvmCreators {
    address constant SVM_ADDRESS = address(0xF3993A62377BCd56AE39D773740A5390411E8BC9);

    function checkSvmCreators(uint256) public {
        uint8 small = Svm(SVM_ADDRESS).createUint8("small");
        int16 signed = Svm(SVM_ADDRESS).createInt16("signed");
        bytes2 fixedBytes = Svm(SVM_ADDRESS).createBytes2("fixedBytes");
        bytes memory data = Svm(SVM_ADDRESS).createBytes("data");
        bytes memory sizedData = Svm(SVM_ADDRESS).createBytes(5, "sizedData");
        string memory text = Svm(SVM_ADDRESS).createString("text");
        string memory sizedText = Svm(SVM_ADDRESS).createString(3, "sizedText");

        assert(uint256(small) < 256);
        assert(signed == signed);
        assert(data.length == 2);
        assert(sizedData.length == 5);
        assert(bytes(text).length == 2);
        assert(bytes(sizedText).length == 3);
        assert(fixedBytes == fixedBytes);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSvmCreators"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSvmCreators(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic Halmos compatibility cheatcode"), "{stdout}");
});
forgetest_init!(symbolic_vm_expect_revert_matches_external_reverts, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_revert_matches_external_reverts because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectRevert.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedReverter {
    error Custom(uint256 value);

    function failWithCustom(uint256 value) external pure {
        revert Custom(value);
    }

    function failPanic() external pure {
        assert(false);
    }
}

contract SymbolicExpectRevert is Test {
    SymbolicExpectedReverter helper;

    function setUp() public {
        helper = new SymbolicExpectedReverter();
    }

    function checkExpectRevert(uint256) public {
        vm.expectRevert(SymbolicExpectedReverter.Custom.selector);
        helper.failWithCustom(7);

        vm.expectRevert(abi.encodeWithSelector(SymbolicExpectedReverter.Custom.selector, uint256(9)));
        helper.failWithCustom(9);

        vm.expectRevert(bytes4(0x4e487b71));
        helper.failPanic();
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExpectRevert"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkExpectRevert(uint256)
"#]],
    );
});

forgetest_init!(symbolic_vm_expect_revert_missing_is_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_revert_missing_is_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectRevertMissing.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedNoop {
    function noFail() external pure {}
}

contract SymbolicExpectRevertMissing is Test {
    SymbolicExpectedNoop helper;

    function setUp() public {
        helper = new SymbolicExpectedNoop();
    }

    function checkMissingExpectedRevert(uint256) public {
        vm.expectRevert();
        helper.noFail();
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMissingExpectedRevert"])
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
checkMissingExpectedRevert(uint256)
"#]],
    );
});

forgetest_init!(symbolic_vm_expect_revert_mismatch_is_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_revert_mismatch_is_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectRevertMismatch.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedMismatchReverter {
    error Custom(uint256 value);

    function failWithCustom(uint256 value) external pure {
        revert Custom(value);
    }
}

contract SymbolicExpectRevertMismatch is Test {
    SymbolicExpectedMismatchReverter helper;

    function setUp() public {
        helper = new SymbolicExpectedMismatchReverter();
    }

    function checkMismatchedExpectedRevert(uint256) public {
        vm.expectRevert(abi.encodeWithSelector(SymbolicExpectedMismatchReverter.Custom.selector, uint256(1)));
        helper.failWithCustom(2);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMismatchedExpectedRevert"])
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
checkMismatchedExpectedRevert(uint256)
"#]],
    );
});

forgetest_init!(symbolic_vm_expect_revert_accepts_symbolic_data, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_revert_accepts_symbolic_data because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectRevertSymbolicData.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedSymbolicReverter {
    error Custom(uint256 value);

    function failWithCustom(uint256 value) external pure {
        revert Custom(value);
    }

    function failWithSelector(bytes4 selector) external pure {
        assembly {
            mstore(0, selector)
            revert(0, 4)
        }
    }
}

contract SymbolicExpectRevertSymbolicData is Test {
    SymbolicExpectedSymbolicReverter helper;

    function setUp() public {
        helper = new SymbolicExpectedSymbolicReverter();
    }

    function checkSymbolicExpectedRevertPayload(uint256 value) public {
        vm.expectRevert(abi.encodeWithSelector(SymbolicExpectedSymbolicReverter.Custom.selector, value));
        helper.failWithCustom(value);
    }

    function checkSymbolicExpectedRevertSelector(bytes4 selector) public {
        vm.expectRevert(selector);
        helper.failWithSelector(selector);
    }

    function checkSymbolicExpectedReverter(address reverter) public {
        vm.assume(reverter == address(helper));
        vm.expectRevert(reverter);
        helper.failWithCustom(9);
    }

    function checkSymbolicExpectedRevertMismatch(uint256 value) public {
        vm.expectRevert(abi.encodeWithSelector(SymbolicExpectedSymbolicReverter.Custom.selector, uint256(7)));
        helper.failWithCustom(value);
    }
}
"#,
    );

    let stdout = cmd
        .args([
            "test",
            "--symbolic",
            "--match-contract",
            "SymbolicExpectRevertSymbolicData",
            "--match-test",
            "checkSymbolicExpectedRevertPayload|checkSymbolicExpectedRevertSelector|checkSymbolicExpectedReverter",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicExpectedRevertPayload(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicExpectedRevertSelector(bytes4)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSymbolicExpectedReverter(address)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.expectRevert"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_revert_symbolic_data_mismatch_fails, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_revert_symbolic_data_mismatch_fails because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectRevertSymbolicMismatch.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedSymbolicMismatchReverter {
    error Custom(uint256 value);

    function failWithCustom(uint256 value) external pure {
        revert Custom(value);
    }
}

contract SymbolicExpectRevertSymbolicMismatch is Test {
    SymbolicExpectedSymbolicMismatchReverter helper;

    function setUp() public {
        helper = new SymbolicExpectedSymbolicMismatchReverter();
    }

    function checkSymbolicExpectedRevertMismatch(uint256 value) public {
        vm.expectRevert(abi.encodeWithSelector(SymbolicExpectedSymbolicMismatchReverter.Custom.selector, uint256(7)));
        helper.failWithCustom(value);
    }

    function checkSymbolicExpectedReverterMismatch(address reverter) public {
        vm.expectRevert(reverter);
        helper.failWithCustom(7);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSymbolicExpectedRevertMismatch"])
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
checkSymbolicExpectedRevertMismatch(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic expected revert data"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkSymbolicExpectedReverterMismatch"])
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
checkSymbolicExpectedReverterMismatch(address)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.expectRevert"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_emit_matches_external_logs, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_emit_matches_external_logs because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectEmit.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicEmitter {
    event Seen(address indexed who, uint256 indexed id, uint256 value);

    function fire(address who, uint256 id, uint256 value) external {
        emit Seen(who, id, value);
    }
}

contract SymbolicExpectEmit is Test {
    event Seen(address indexed who, uint256 indexed id, uint256 value);

    SymbolicEmitter emitter;

    function setUp() public {
        emitter = new SymbolicEmitter();
    }

    function checkExpectEmit(uint256) public {
        vm.expectEmit(true, true, false, true, address(emitter));
        emit Seen(address(0xB0B), 7, 9);
        emitter.fire(address(0xB0B), 7, 9);
    }

    function checkExpectEmitSymbolicEmitter(address expectedEmitter) public {
        vm.assume(expectedEmitter == address(emitter));
        vm.expectEmit(true, true, false, true, expectedEmitter);
        emit Seen(address(0xB0B), 7, 9);
        emitter.fire(address(0xB0B), 7, 9);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExpectEmit"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkExpectEmit(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkExpectEmitSymbolicEmitter(address)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.expectEmit"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_emit_mismatch_is_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_emit_mismatch_is_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectEmitMismatch.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicMismatchEmitter {
    event Seen(address indexed who, uint256 indexed id, uint256 value);

    function fire(address who, uint256 id, uint256 value) external {
        emit Seen(who, id, value);
    }
}

contract SymbolicExpectEmitMismatch is Test {
    event Seen(address indexed who, uint256 indexed id, uint256 value);

    SymbolicMismatchEmitter emitter;

    function setUp() public {
        emitter = new SymbolicMismatchEmitter();
    }

    function checkMismatchedExpectEmit(uint256) public {
        vm.expectEmit(true, true, false, true, address(emitter));
        emit Seen(address(0xB0B), 7, 9);
        emitter.fire(address(0xB0B), 8, 9);
    }

    function checkMismatchedExpectEmitSymbolicEmitter(address expectedEmitter) public {
        vm.expectEmit(true, true, false, true, expectedEmitter);
        emit Seen(address(0xB0B), 7, 9);
        emitter.fire(address(0xB0B), 7, 9);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMismatchedExpectEmit"])
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
checkMismatchedExpectEmit(uint256)
"#]],
    );

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMismatchedExpectEmitSymbolicEmitter"])
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
checkMismatchedExpectEmitSymbolicEmitter(address)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.expectEmit"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_call_matches_and_reports_missing, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_call_matches_and_reports_missing because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicExpectCall.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicExpectedCallTarget {
    function ping(uint256 value) external pure returns (uint256) {
        return value + 1;
    }
}

contract SymbolicExpectCall is Test {
    SymbolicExpectedCallTarget target;

    function setUp() public {
        target = new SymbolicExpectedCallTarget();
    }

    function checkExpectCallMatches(uint256) public {
        vm.expectCall(
            address(target),
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(7))
        );
        assertEq(target.ping(7), 8);

        vm.expectCall(address(target), 0, abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(9)), 1);
        assertEq(target.ping(9), 10);
    }

    function checkExpectCallGasUnsupported(uint256) public {
        vm.expectCall(
            address(target),
            0,
            uint64(50000),
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(13))
        );
        assertEq(target.ping{gas: 50000}(13), 14);

        vm.expectCallMinGas(
            address(target),
            0,
            uint64(25000),
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(14))
        );
        assertEq(target.ping{gas: 50000}(14), 15);
    }

    function checkExpectCallSymbolicCallee(address expectedCallee) public {
        vm.assume(expectedCallee == address(target));
        vm.expectCall(
            expectedCallee,
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(7))
        );
        assertEq(target.ping(7), 8);
    }

    function checkExpectCallMissing(uint256) public {
        vm.expectCall(
            address(target),
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(11))
        );
    }

    function checkSymbolicCalleeExpectedCallMismatch(address expectedCallee) public {
        vm.expectCall(
            expectedCallee,
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(7))
        );
        assertEq(target.ping(7), 8);
    }

    function checkExpectCallMinGasMissing(uint256) public {
        vm.expectCallMinGas(
            address(target),
            0,
            uint64(60000),
            abi.encodeWithSelector(SymbolicExpectedCallTarget.ping.selector, uint256(15))
        );
        assertEq(target.ping{gas: 50000}(15), 16);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExpectCallMatches"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkExpectCallMatches(uint256)
"#]],
    );

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkExpectCallGasUnsupported"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL: incomplete symbolic execution (Stuck): unsupported symbolic execution feature: symbolic expected call gas] checkExpectCallGasUnsupported(uint256)
"#]],
    );

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkExpectCallSymbolicCallee"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkExpectCallSymbolicCallee(address)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.expectCall"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkExpectCallMissing"])
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
checkExpectCallMissing(uint256)
"#]],
    );

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCalleeExpectedCallMismatch"])
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
checkSymbolicCalleeExpectedCallMismatch(address)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.expectCall"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkExpectCallMinGasMissing"])
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
checkExpectCallMinGasMissing(uint256)
"#]],
    );
});

forgetest_init!(symbolic_vm_mock_call_returns_and_reverts, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_mock_call_returns_and_reverts because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMockCall.t.sol",
        r#"
import "forge-std/Test.sol";

interface IMockedTarget {
    function value(uint256 input) external returns (uint256);
}

contract SymbolicMockCall is Test {
    function checkMockCall(uint256) public {
        address target = address(0x1234);

        vm.mockCall(
            target,
            abi.encodeWithSelector(IMockedTarget.value.selector, uint256(1)),
            abi.encode(uint256(99))
        );
        assertEq(IMockedTarget(target).value(1), 99);

        vm.mockCallRevert(
            target,
            abi.encodeWithSelector(IMockedTarget.value.selector, uint256(2)),
            abi.encodeWithSignature("Error(string)", "mocked")
        );
        (bool ok, bytes memory data) =
            target.call(abi.encodeWithSelector(IMockedTarget.value.selector, uint256(2)));
        assertFalse(ok);
        assertGt(data.length, 0);
    }

    function checkMockCallSymbolicCallee(address mocked) public {
        address target = address(0x1234);
        vm.assume(mocked == target);

        vm.mockCall(
            mocked,
            abi.encodeWithSelector(IMockedTarget.value.selector, uint256(1)),
            abi.encode(uint256(99))
        );
        assertEq(IMockedTarget(target).value(1), 99);
    }

    function checkSymbolicCalleeMockMismatch(address mocked) public {
        address target = address(0x1234);

        vm.mockCall(
            mocked,
            abi.encodeWithSelector(IMockedTarget.value.selector, uint256(1)),
            abi.encode(uint256(99))
        );
        (bool ok, bytes memory data) =
            target.call(abi.encodeWithSelector(IMockedTarget.value.selector, uint256(1)));
        uint256 value = data.length == 32 ? abi.decode(data, (uint256)) : 0;
        assertTrue(ok);
        assertEq(value, 99);
    }

    function checkSelectorMockCallsAndClear(uint256 input) public {
        address target = address(0x4567);
        bytes[] memory returnValues = new bytes[](2);
        returnValues[0] = abi.encode(uint256(100));
        returnValues[1] = abi.encode(uint256(200));

        vm.mockCalls(target, abi.encodePacked(IMockedTarget.value.selector), returnValues);
        assertEq(IMockedTarget(target).value(input), 100);
        assertEq(IMockedTarget(target).value(1), 200);
        assertEq(IMockedTarget(target).value(2), 200);

        vm.clearMockedCalls();
        (bool ok, bytes memory data) =
            target.call(abi.encodeWithSelector(IMockedTarget.value.selector, input));
        assertTrue(ok);
        assertEq(data.length, 0);
    }

    function checkMockCallsAcceptsSymbolicData(address mocked, uint256 input) public {
        address target = address(0x4567);
        vm.assume(mocked == target);
        vm.assume(input < type(uint256).max - 2);

        bytes[] memory returnValues = new bytes[](2);
        returnValues[0] = abi.encode(input + 1);
        returnValues[1] = abi.encode(input + 2);

        vm.mockCalls(
            mocked,
            abi.encodeWithSelector(IMockedTarget.value.selector, input),
            returnValues
        );
        assertEq(IMockedTarget(target).value(input), input + 1);
        assertEq(IMockedTarget(target).value(input), input + 2);
        assertEq(IMockedTarget(target).value(input), input + 2);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMockCall"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkMockCall(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkMockCallSymbolicCallee(address)
"#]],
    );
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.mockCall"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkSelectorMockCallsAndClear"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSelectorMockCallsAndClear(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMockCallsAcceptsSymbolicData"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkMockCallsAcceptsSymbolicData(address,uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.mockCalls"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkSymbolicCalleeMockMismatch"])
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
checkSymbolicCalleeMockMismatch(address)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.mockCall"), "{stdout}");
});

forgetest_init!(symbolic_vm_call_expectations_allow_symbolic_value_when_unpinned, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_call_expectations_allow_symbolic_value_when_unpinned because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicUnpinnedCallValue.t.sol",
        r#"
import "forge-std/Test.sol";

interface IValueTarget {
    function ping(uint256 input) external payable returns (uint256);
}

contract ValueTarget {
    function ping(uint256 input) external payable returns (uint256) {
        return input + msg.value;
    }
}

contract SymbolicUnpinnedCallValue is Test {
    ValueTarget target;

    function setUp() public {
        target = new ValueTarget();
    }

    function checkExpectCallAllowsSymbolicValue(uint8 amount) public {
        vm.assume(amount <= 1);
        vm.deal(address(this), 1);

        vm.expectCall(
            address(target),
            abi.encodeWithSelector(ValueTarget.ping.selector, uint256(7))
        );

        assertEq(target.ping{value: amount}(7), 7 + amount);
    }

    function checkMockCallAllowsSymbolicValue(uint8 amount) public {
        vm.assume(amount <= 1);
        address mocked = address(0xBEEF);

        vm.mockCall(
            mocked,
            abi.encodeWithSelector(IValueTarget.ping.selector, uint256(3)),
            abi.encode(uint256(44))
        );

        assertEq(IValueTarget(mocked).ping{value: amount}(3), 44);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicUnpinnedCallValue"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkExpectCallAllowsSymbolicValue(uint8)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkMockCallAllowsSymbolicValue(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic expected call value"), "{stdout}");
    assert!(!stdout.contains("symbolic mocked call value"), "{stdout}");
});

forgetest_init!(symbolic_vm_call_expectations_branch_symbolic_pinned_value, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_call_expectations_branch_symbolic_pinned_value because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPinnedCallValue.t.sol",
        r#"
import "forge-std/Test.sol";

interface IPinnedValueTarget {
    function ping(uint256 input) external payable returns (uint256);
}

contract PinnedValueTarget {
    function ping(uint256 input) external payable returns (uint256) {
        return input + msg.value;
    }
}

contract SymbolicPinnedCallValue is Test {
    PinnedValueTarget target;

    function setUp() public {
        target = new PinnedValueTarget();
    }

    function checkExpectCallPinnedValueFindsMismatch(uint8 amount) public {
        vm.assume(amount <= 1);
        vm.deal(address(this), 1);

        vm.expectCall(
            address(target),
            uint256(1),
            abi.encodeWithSelector(PinnedValueTarget.ping.selector, uint256(7)),
            1
        );

        assertEq(target.ping{value: amount}(7), 7 + amount);
    }

    function checkMockCallPinnedValueMatches(uint8 amount) public {
        vm.assume(amount == 1);
        vm.deal(address(this), 1);
        address mocked = address(0xCAFE);

        vm.mockCall(
            mocked,
            uint256(1),
            abi.encodeWithSelector(IPinnedValueTarget.ping.selector, uint256(3)),
            abi.encode(uint256(44))
        );

        assertEq(IPinnedValueTarget(mocked).ping{value: amount}(3), 44);
    }

    function checkMockCallPinnedValueFindsMismatch(uint8 amount) public {
        vm.assume(amount <= 1);
        vm.deal(address(this), 1);
        address mocked = address(0xBEEF);

        vm.mockCall(
            mocked,
            uint256(1),
            abi.encodeWithSelector(IPinnedValueTarget.ping.selector, uint256(3)),
            abi.encode(uint256(44))
        );

        (bool ok, bytes memory data) = mocked.call{value: amount}(
            abi.encodeWithSelector(IPinnedValueTarget.ping.selector, uint256(3))
        );
        assertTrue(ok);
        assertEq(data.length, 32);
        assertEq(abi.decode(data, (uint256)), 44);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkExpectCallPinnedValueFindsMismatch"])
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
checkExpectCallPinnedValueFindsMismatch(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic expected call value"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMockCallPinnedValueMatches"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkMockCallPinnedValueMatches(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic mocked call value"), "{stdout}");

    let stdout = prj
        .forge_command()
        .args(["test", "--symbolic", "--match-test", "checkMockCallPinnedValueFindsMismatch"])
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
checkMockCallPinnedValueFindsMismatch(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic mocked call value"), "{stdout}");
});

forgetest_init!(symbolic_vm_expect_and_mock_call_accept_symbolic_data, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_expect_and_mock_call_accept_symbolic_data because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCallDataCheatcodes.t.sol",
        r#"
import "forge-std/Test.sol";

interface ISymbolicDataTarget {
    function value(uint256 input) external returns (uint256);
}

contract SymbolicDataTarget {
    function value(uint256 input) external pure returns (uint256) {
        return input;
    }
}

contract SymbolicFunctionMockTarget {
    function value(uint256 input) external pure returns (uint256) {
        return input + 10;
    }
}

contract SymbolicCallDataCheatcodes is Test {
    SymbolicDataTarget target;
    SymbolicFunctionMockTarget functionTarget;

    function setUp() public {
        target = new SymbolicDataTarget();
        functionTarget = new SymbolicFunctionMockTarget();
    }

    function checkExpectCallAcceptsSymbolicData(uint256 input) public {
        vm.expectCall(
            address(target),
            abi.encodeWithSelector(SymbolicDataTarget.value.selector, input)
        );

        assertEq(target.value(input), input);
    }

    function checkMockCallAcceptsSymbolicDataAndReturn(uint256 input) public {
        address mocked = address(0xDADA);

        vm.mockCall(
            mocked,
            abi.encodeWithSelector(ISymbolicDataTarget.value.selector, input),
            abi.encode(input + 1)
        );

        assertEq(ISymbolicDataTarget(mocked).value(input), input + 1);
    }

    function checkMockCallAcceptsSymbolicBytes4Selector(bytes4 selector) public {
        vm.assume(selector == ISymbolicDataTarget.value.selector);
        address mocked = address(0xFACE);

        vm.mockCall(mocked, selector, abi.encode(uint256(99)));

        assertEq(ISymbolicDataTarget(mocked).value(1), 99);
    }

    function checkMockFunctionAcceptsSymbolicData(uint256 input) public {
        address mocked = address(0xF00D);

        vm.mockFunction(
            mocked,
            address(functionTarget),
            abi.encodeWithSelector(ISymbolicDataTarget.value.selector, input)
        );

        assertEq(ISymbolicDataTarget(mocked).value(input), input + 10);
    }

    function checkMockFunctionAcceptsSymbolicCallee(address mocked, uint256 input) public {
        address actual = address(0xF00D);
        vm.assume(mocked == actual);

        vm.mockFunction(
            mocked,
            address(functionTarget),
            abi.encodeWithSelector(ISymbolicDataTarget.value.selector, input)
        );

        assertEq(ISymbolicDataTarget(actual).value(input), input + 10);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicCallDataCheatcodes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkExpectCallAcceptsSymbolicData(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkMockCallAcceptsSymbolicDataAndReturn(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkMockCallAcceptsSymbolicBytes4Selector(bytes4)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkMockFunctionAcceptsSymbolicData(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkMockFunctionAcceptsSymbolicCallee(address,uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.expectCall"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.mockCall"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.mockFunction"), "{stdout}");
});

forgetest_init!(symbolic_vm_call_data_match_branches_find_mismatch, |prj, _cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_call_data_match_branches_find_mismatch because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicCallDataMismatch.t.sol",
        r#"
import "forge-std/Test.sol";

interface ISymbolicCallDataMismatchTarget {
    function value(uint256 input) external returns (uint256);
}

contract SymbolicCallDataMismatchTarget {
    function value(uint256 input) external pure returns (uint256) {
        return input;
    }
}

contract SymbolicFunctionMockMismatchTarget {
    function value(uint256 input) external pure returns (uint256) {
        return input + 10;
    }
}

contract SymbolicCallDataMismatch is Test {
    SymbolicCallDataMismatchTarget target;
    SymbolicFunctionMockMismatchTarget functionTarget;

    function setUp() public {
        target = new SymbolicCallDataMismatchTarget();
        functionTarget = new SymbolicFunctionMockMismatchTarget();
    }

    function checkExpectCallSymbolicDataFindsMismatch(uint256 expected, uint256 actual) public {
        vm.expectCall(
            address(target),
            abi.encodeWithSelector(SymbolicCallDataMismatchTarget.value.selector, expected)
        );

        target.value(actual);
    }

    function checkMockCallSymbolicDataFindsMismatch(uint256 expected, uint256 actual) public {
        address mocked = address(0xDADA);

        vm.mockCall(
            mocked,
            abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, expected),
            abi.encode(uint256(99))
        );

        (bool ok, bytes memory data) =
            mocked.call(abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, actual));
        assertTrue(ok);
        assertEq(data.length, 32);
        assertEq(abi.decode(data, (uint256)), 99);
    }

    function checkMockFunctionSymbolicDataFindsMismatch(uint256 expected, uint256 actual) public {
        address mocked = address(0xF00D);

        vm.mockFunction(
            mocked,
            address(functionTarget),
            abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, expected)
        );

        (bool ok, bytes memory data) =
            mocked.call(abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, actual));
        assertTrue(ok);
        assertEq(data.length, 32);
        assertEq(abi.decode(data, (uint256)), actual + 10);
    }

    function checkMockFunctionSymbolicCalleeFindsMismatch(address mocked) public {
        address actual = address(0xF00D);

        vm.mockFunction(
            mocked,
            address(functionTarget),
            abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, uint256(1))
        );

        (bool ok, bytes memory data) =
            actual.call(abi.encodeWithSelector(ISymbolicCallDataMismatchTarget.value.selector, uint256(1)));
        assertTrue(ok);
        assertEq(data.length, 32);
        assertEq(abi.decode(data, (uint256)), uint256(11));
    }
}
"#,
    );

    for test in [
        "checkExpectCallSymbolicDataFindsMismatch",
        "checkMockCallSymbolicDataFindsMismatch",
        "checkMockFunctionSymbolicDataFindsMismatch",
        "checkMockFunctionSymbolicCalleeFindsMismatch",
    ] {
        let stdout = prj
            .forge_command()
            .args(["test", "--symbolic", "--match-test", test])
            .assert_failure()
            .get_output()
            .stdout_lossy();

        assert_relevant_lines(
            &stdout,
            foundry_test_utils::str![[r#"
[FAIL:
"#]],
        );
        assert_relevant_lines(&stdout, test);
        assert!(!stdout.contains("symbolic vm.expectCall"), "{stdout}");
        assert!(!stdout.contains("symbolic vm.mockCall"), "{stdout}");
        assert!(!stdout.contains("symbolic vm.mockFunction"), "{stdout}");
    }
});

forgetest_init!(symbolic_vm_mock_function_routes_to_target, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_mock_function_routes_to_target because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMockFunction.t.sol",
        r#"
import "forge-std/Test.sol";

interface IFunctionMock {
    function value(uint256 input) external returns (uint256);
    function who(uint256 input) external returns (address);
}

contract FunctionCallee {
    function value(uint256 input) external pure returns (uint256) {
        return input + 1;
    }

    function who(uint256) external view returns (address) {
        return address(this);
    }
}

contract FunctionTarget {
    function value(uint256 input) external pure returns (uint256) {
        return input ^ 0x55;
    }

    function who(uint256) external view returns (address) {
        return address(this);
    }
}

contract SymbolicMockFunction is Test {
    FunctionCallee callee;
    FunctionTarget target;

    function setUp() public {
        callee = new FunctionCallee();
        target = new FunctionTarget();
    }

    function checkMockFunction(uint256 input) public {
        vm.mockFunction(
            address(callee),
            address(target),
            abi.encodePacked(IFunctionMock.value.selector)
        );
        assertEq(IFunctionMock(address(callee)).value(input), input ^ 0x55);

        vm.mockFunction(
            address(callee),
            address(target),
            abi.encodePacked(IFunctionMock.who.selector)
        );
        assertEq(IFunctionMock(address(callee)).who(input), address(callee));
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkMockFunction"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkMockFunction(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_record_accesses_tracks_symbolic_slots, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_record_accesses_tracks_symbolic_slots because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRecordAccesses.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicRecordAccesses is Test {
    function checkRecordAccesses(bytes32 slot, bytes32 stored) public {
        vm.record();

        bytes32 loadedSlot;
        assembly {
            sstore(slot, stored)
            loadedSlot := sload(slot)
        }

        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(address(this));
        assertEq(loadedSlot, stored);
        assertEq(reads.length, 1);
        assertEq(writes.length, 1);
        assertEq(reads[0], slot);
        assertEq(writes[0], slot);

        vm.stopRecord();
    }

    function checkRecordAccessesSymbolicTarget(address target, bytes32 slot, bytes32 stored) public {
        vm.assume(target == address(this));
        vm.record();

        bytes32 loadedSlot;
        assembly {
            sstore(slot, stored)
            loadedSlot := sload(slot)
        }

        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(target);
        assertEq(loadedSlot, stored);
        assertEq(reads.length, 1);
        assertEq(writes.length, 1);
        assertEq(reads[0], slot);
        assertEq(writes[0], slot);

        vm.stopRecord();
    }

    function checkRecordAccessesSymbolicTargetBranches(address target, bytes32 slot, bytes32 stored) public {
        address other = address(0xBEEF);
        vm.assume(target == address(this) || target == other);
        vm.record();

        bytes32 loadedSlot;
        assembly {
            sstore(slot, stored)
            loadedSlot := sload(slot)
        }

        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(target);
        assertEq(loadedSlot, stored);
        if (target == address(this)) {
            assertEq(reads.length, 1);
            assertEq(writes.length, 1);
            assertEq(reads[0], slot);
            assertEq(writes[0], slot);
        } else {
            assertEq(reads.length, 0);
            assertEq(writes.length, 0);
        }

        vm.stopRecord();
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkRecordAccesses"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkRecordAccesses(bytes32,bytes32)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkRecordAccessesSymbolicTarget(address,bytes32,bytes32)
"#]],
    );
    assert!(
        stdout
            .contains("[PASS] checkRecordAccessesSymbolicTargetBranches(address,bytes32,bytes32)"),
        "{stdout}"
    );
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
    assert!(!stdout.contains("symbolic vm.accesses address"), "{stdout}");
});

forgetest_init!(symbolic_vm_bound_skip_and_gas_noops, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_bound_skip_and_gas_noops because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBoundSkip.t.sol",
        r#"
import "forge-std/Test.sol";

interface SymbolicVmCompat {
    enum CallerMode {
        None,
        Broadcast,
        RecurrentBroadcast,
        Prank,
        RecurrentPrank
    }

    enum ForgeContext {
        TestGroup,
        Test,
        Coverage,
        Snapshot,
        ScriptGroup,
        ScriptDryRun,
        ScriptBroadcast,
        ScriptResume,
        Unknown
    }

    function readCallers() external view returns (CallerMode callerMode, address msgSender, address txOrigin);
    function isContext(ForgeContext context) external view returns (bool result);
}

contract SymbolicBoundSkip is Test {
    function externalNoop() external {}

    function checkBoundSkipAndGasNoops(uint256 x, int256 y) public {
        vm.pauseGasMetering();
        vm.resumeGasMetering();
        vm.resetGasMetering();
        vm.assume(x >= 10 && x <= 12);
        uint256 bounded = vm.bound(x, 10, 12);
        assertGe(bounded, 10);
        assertLe(bounded, 12);

        vm.assume(y >= -3 && y <= 3);
        int256 signedBounded = vm.bound(y, -3, 3);
        assertGe(signedBounded, -3);
        assertLe(signedBounded, 3);

        vm.skip(x == 42);
        assertTrue(x != 42);
    }

    function checkVmCompatibilityTail() public {
        SymbolicVmCompat compat = SymbolicVmCompat(address(vm));
        SymbolicVmCompat.CallerMode mode;
        address sender;
        (mode,,) = compat.readCallers();
        assertEq(uint256(mode), uint256(SymbolicVmCompat.CallerMode.None));

        vm.prank(address(0xB0B));
        (mode, sender,) = compat.readCallers();
        assertEq(uint256(mode), uint256(SymbolicVmCompat.CallerMode.Prank));
        assertEq(sender, address(0xB0B));
        vm.stopPrank();

        vm.startPrank(address(0xCAFE));
        (mode, sender,) = compat.readCallers();
        assertEq(uint256(mode), uint256(SymbolicVmCompat.CallerMode.RecurrentPrank));
        assertEq(sender, address(0xCAFE));
        vm.stopPrank();

        vm.allowCheatcodes(address(this));
        vm.makePersistent(address(this));
        assertTrue(vm.isPersistent(address(this)));
        address[] memory accounts = new address[](1);
        accounts[0] = address(0xBEEF);
        vm.makePersistent(accounts);
        assertTrue(vm.isPersistent(address(0xBEEF)));
        vm.revokePersistent(address(this));
        vm.revokePersistent(accounts);
        assertFalse(vm.isPersistent(address(this)));
        assertFalse(vm.isPersistent(address(0xBEEF)));

        vm.label(address(this), "self");
        assertEq(vm.getLabel(address(this)), "self");
        vm.snapshotValue("value", 1);
        vm.snapshotValue("group", "value", 1);
        vm.cool(address(this));
        vm.warmSlot(address(this), bytes32(uint256(1)));
        vm.coolSlot(address(this), bytes32(uint256(1)));
        vm.noAccessList();
        assertEq(vm.getChainId(), block.chainid);
        assertTrue(bytes(vm.projectRoot()).length != 0);
        assertTrue(vm.unixTime() != 0);
        assertTrue(compat.isContext(SymbolicVmCompat.ForgeContext.TestGroup));
        assertTrue(compat.isContext(SymbolicVmCompat.ForgeContext.Test));
        assertFalse(compat.isContext(SymbolicVmCompat.ForgeContext.ScriptGroup));
    }

    function checkRuntimeNoopsAndArrayAssertions() public {
        vm.breakpoint("symbolic");
        vm.breakpoint("symbolic", true);
        assertTrue(bytes(vm.getFoundryVersion()).length != 0);
        vm.sleep(0);
        Vm.AccessListItem[] memory access = new Vm.AccessListItem[](0);
        vm.accessList(access);

        uint256[] memory left = new uint256[](1);
        uint256[] memory right = new uint256[](1);
        left[0] = 1;
        right[0] = 1;
        assertEq(left, right);
        right[0] = 2;
        assertNotEq(left, right);

        string[] memory words = new string[](1);
        string[] memory sameWords = new string[](1);
        words[0] = "foundry";
        sameWords[0] = "foundry";
        assertEq(words, sameWords);

        assertEqDecimal(uint256(1e18), uint256(1e18), 18);
        assertEqDecimal(int256(-1e18), int256(-1e18), 18);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicBoundSkip"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkBoundSkipAndGasNoops(uint256,int256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkVmCompatibilityTail()
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkRuntimeNoopsAndArrayAssertions()
"#]],
    );
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_bound_invalid_range_fails_without_stuck, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_bound_invalid_range_fails_without_stuck because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBoundInvalid.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicBoundInvalid is Test {
    function checkInvalidUnsignedBound(uint256 x) public {
        vm.bound(x, 12, 10);
    }

    function checkInvalidSignedBound(int256 x) public {
        vm.bound(x, 3, -3);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicBoundInvalid"])
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
checkInvalidUnsignedBound(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkInvalidSignedBound(int256)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.bound range"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_assume_no_revert_prunes_reverting_call, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_assume_no_revert_prunes_reverting_call because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicAssumeNoRevert.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicAssumeNoRevertTarget {
    function maybeRevert(uint256 x) external pure {
        require(x != 7, "seven");
    }
}

contract SymbolicAssumeNoRevert is Test {
    SymbolicAssumeNoRevertTarget target;

    function setUp() public {
        target = new SymbolicAssumeNoRevertTarget();
    }

    function checkAssumeNoRevertPrunes(uint256 x) public {
        vm.assumeNoRevert();
        (bool ok,) = address(target).call(abi.encodeWithSelector(target.maybeRevert.selector, x));
        assertTrue(ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkAssumeNoRevertPrunes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkAssumeNoRevertPrunes(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.assumeNoRevert"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});

forgetest_init!(symbolic_vm_assume_no_revert_filters_revert_matches, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_assume_no_revert_filters_revert_matches because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicAssumeNoRevertFilters.t.sol",
        r#"
import "forge-std/Test.sol";

interface SymbolicVm {
    struct PotentialRevert {
        address reverter;
        bool partialMatch;
        bytes revertData;
    }

    function assume(bool condition) external pure;
    function assumeNoRevert(PotentialRevert calldata potentialRevert) external pure;
    function assumeNoRevert(PotentialRevert[] calldata potentialReverts) external pure;
}

error Expected(uint256 value);
error Other(uint256 value);

contract SymbolicAssumeNoRevertFilterTarget {
    function onlyExpected(uint256 x) external pure {
        if (x == 7) revert Expected(7);
    }

    function twoReverts(uint256 x) external pure {
        if (x == 7) revert Expected(x);
        if (x == 9) revert Other(x);
    }
}

contract SymbolicAssumeNoRevertOtherTarget {
    function onlyExpected(uint256 x) external pure {
        if (x == 7) revert Expected(7);
    }
}

contract SymbolicAssumeNoRevertFilters is Test {
    SymbolicAssumeNoRevertFilterTarget target;
    SymbolicAssumeNoRevertOtherTarget other;
    SymbolicVm symbolicVm = SymbolicVm(VM_ADDRESS);

    function setUp() public {
        target = new SymbolicAssumeNoRevertFilterTarget();
        other = new SymbolicAssumeNoRevertOtherTarget();
    }

    function checkAssumeNoRevertExactFilterPrunes(uint256 x) public {
        symbolicVm.assumeNoRevert(SymbolicVm.PotentialRevert({
            reverter: address(target),
            partialMatch: false,
            revertData: abi.encodeWithSelector(Expected.selector, uint256(7))
        }));

        (bool ok,) = address(target).call(abi.encodeWithSelector(target.onlyExpected.selector, x));
        assertTrue(ok);
    }

    function checkAssumeNoRevertArrayFilterPrunes(uint256 x) public {
        SymbolicVm.PotentialRevert[] memory filters = new SymbolicVm.PotentialRevert[](2);
        filters[0] = SymbolicVm.PotentialRevert({
            reverter: address(target),
            partialMatch: true,
            revertData: abi.encodeWithSelector(Expected.selector)
        });
        filters[1] = SymbolicVm.PotentialRevert({
            reverter: address(target),
            partialMatch: false,
            revertData: abi.encodeWithSelector(Other.selector, uint256(9))
        });
        symbolicVm.assumeNoRevert(filters);

        (bool ok,) = address(target).call(abi.encodeWithSelector(target.twoReverts.selector, x));
        assertTrue(ok);
    }

    function checkAssumeNoRevertWrongDataFails(uint256 x) public {
        symbolicVm.assume(x == 9);
        symbolicVm.assumeNoRevert(SymbolicVm.PotentialRevert({
            reverter: address(target),
            partialMatch: false,
            revertData: abi.encodeWithSelector(Other.selector, uint256(8))
        }));

        (bool ok,) = address(target).call(abi.encodeWithSelector(target.twoReverts.selector, x));
        assertTrue(ok);
    }

    function checkAssumeNoRevertWrongReverterFails(uint256 x) public {
        symbolicVm.assume(x == 7);
        symbolicVm.assumeNoRevert(SymbolicVm.PotentialRevert({
            reverter: address(other),
            partialMatch: true,
            revertData: abi.encodeWithSelector(Expected.selector)
        }));

        (bool ok,) = address(target).call(abi.encodeWithSelector(target.onlyExpected.selector, x));
        assertTrue(ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkAssumeNoRevert.*Prunes"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkAssumeNoRevertExactFilterPrunes(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkAssumeNoRevertArrayFilterPrunes(uint256)
"#]],
    );
    assert!(!stdout.contains("symbolic vm.assumeNoRevert"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");

    for test in ["checkAssumeNoRevertWrongDataFails", "checkAssumeNoRevertWrongReverterFails"] {
        let stdout = prj
            .forge_command()
            .args(["test", "--symbolic", "--match-test", test])
            .assert_failure()
            .get_output()
            .stdout_lossy();

        assert_relevant_lines(
            &stdout,
            foundry_test_utils::str![[r#"
[FAIL:
"#]],
        );
        assert_relevant_lines(&stdout, test);
        assert!(!stdout.contains("symbolic vm.assumeNoRevert"), "{stdout}");
        assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
    }
});

// The `vm.prank(address, bool delegateCall)` overload diverges from concrete
// Forge semantics when `delegateCall == true`: the engine does not model
// pranking through a delegatecall frame, so this branch must fail closed as
// Unsupported rather than silently behaving like the address-only overload.
forgetest_init!(symbolic_vm_prank_delegatecall_overload_reports_unsupported, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_prank_delegatecall_overload_reports_unsupported because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicPrankDelegateCall.t.sol",
        r#"
import "forge-std/Test.sol";

contract Probe {
    function sender() external view returns (address) {
        return msg.sender;
    }
}

contract SymbolicPrankDelegateCall is Test {
    Probe probe;

    function setUp() public {
        probe = new Probe();
    }

    function checkPrankDelegateCall(address who) public {
        vm.prank(who, true);
        probe.sender();
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkPrankDelegateCall"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
unsupported symbolic execution feature: symbolic vm.prank delegatecall
"#]],
    );
});

forgetest_init!(symbolic_vm_deploy_code_models_constructor_outcomes, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_vm_deploy_code_models_constructor_outcomes because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicDeployCodeCheatcode.t.sol",
        r#"
import "forge-std/Test.sol";

/// forge-config: default.evm_version = "cancun"

interface SymbolicDeployCodeVm {
    function randomBool() external view returns (bool);
}

contract OkDeployCodeCtor {
    uint256 public value = 1;
}

contract RevertingDeployCodeCtor {
    constructor() {
        require(false, "ctor");
    }
}

contract EnvBranchingDeployCodeCtor {
    SymbolicDeployCodeVm constant VM =
        SymbolicDeployCodeVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    constructor() {
        if (VM.randomBool()) {
            revert("branch");
        }
    }
}

contract SelfDestructDeployCodeCtor {
    constructor() payable {
        selfdestruct(payable(msg.sender));
    }
}

contract SymbolicDeployCodeCheatcode is Test {
    string constant TARGET = "test/SymbolicDeployCodeCheatcode.t.sol";

    function checkDeployCodeExpectedRevert() public {
        vm.expectRevert();
        vm.deployCode(string.concat(TARGET, ":RevertingDeployCodeCtor"));
    }

    function checkDeployCodeBranchingConstructor() public {
        vm.deployCode(string.concat(TARGET, ":EnvBranchingDeployCodeCtor"));
    }

    function checkDeployCodeStaticContext() public {
        (bool ok,) = address(this).staticcall(abi.encodeCall(this.helperDeployCode, ()));
        assertFalse(ok);
    }

    function helperDeployCode() external {
        vm.deployCode(string.concat(TARGET, ":OkDeployCodeCtor"));
    }

    function checkDeployCodeSelfDestructConstructor() public {
        address deployed = vm.deployCode(string.concat(TARGET, ":SelfDestructDeployCodeCtor"));
        assertEq(deployed.code.length, 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicDeployCodeCheatcode"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkDeployCodeExpectedRevert()
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkDeployCodeBranchingConstructor()
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkDeployCodeStaticContext()
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkDeployCodeSelfDestructConstructor()
"#]],
    );
    assert!(!stdout.contains("symbolic vm.deployCode"), "{stdout}");
    assert!(!stdout.contains("symbolic Foundry cheatcode"), "{stdout}");
});
