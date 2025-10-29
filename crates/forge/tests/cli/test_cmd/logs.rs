//! Tests for various logging functionality

use foundry_test_utils::str;

forgetest_init!(debug_logs, |prj, cmd| {
    prj.add_test(
        "DebugLogs.t.sol",
        r#"
import "forge-std/Test.sol";

contract DebugLogsTest is Test {
    constructor() {
        emit log_uint(0);
    }

    function setUp() public {
        emit log_uint(1);
    }

    function test1() public {
        emit log_uint(2);
    }

    function test2() public {
        emit log_uint(3);
    }

    function testRevertIfWithRevert() public {
        Fails fails = new Fails();
        emit log_uint(4);
        vm.expectRevert();
        fails.failure();
    }

    /// forge-config: default.allow_internal_expect_revert = true
    function testRevertIfWithRequire() public {
        emit log_uint(5);
        vm.expectRevert();
        require(false);
    }

    function testLog() public {
        emit log("Error: Assertion Failed");
    }

    function testLogs() public {
        emit logs(bytes("abcd"));
    }

    function testLogAddress() public {
        emit log_address(address(1));
    }

    function testLogBytes32() public {
        emit log_bytes32(bytes32("abcd"));
    }

    function testLogInt() public {
        emit log_int(int256(-31337));
    }

    function testLogBytes() public {
        emit log_bytes(bytes("abcd"));
    }

    function testLogString() public {
        emit log_string("here");
    }

    function testLogNamedAddress() public {
        emit log_named_address("address", address(1));
    }

    function testLogNamedBytes32() public {
        emit log_named_bytes32("abcd", bytes32("abcd"));
    }

    function testLogNamedDecimalInt() public {
        emit log_named_decimal_int("amount", int256(-31337), uint256(18));
    }

    function testLogNamedDecimalUint() public {
        emit log_named_decimal_uint("amount", uint256(1 ether), uint256(18));
    }

    function testLogNamedInt() public {
        emit log_named_int("amount", int256(-31337));
    }

    function testLogNamedUint() public {
        emit log_named_uint("amount", uint256(1 ether));
    }

    function testLogNamedBytes() public {
        emit log_named_bytes("abcd", bytes("abcd"));
    }

    function testLogNamedString() public {
        emit log_named_string("key", "val");
    }
}

contract Fails is Test {
    function failure() public {
        emit log_uint(100);
        revert();
    }
}
"#,
    );

    cmd.args(["test", "-vv"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 19 tests for test/DebugLogs.t.sol:DebugLogsTest
[PASS] test1() ([GAS])
Logs:
  0
  1
  2

[PASS] test2() ([GAS])
Logs:
  0
  1
  3

[PASS] testLog() ([GAS])
Logs:
  0
  1
  Error: Assertion Failed

[PASS] testLogAddress() ([GAS])
Logs:
  0
  1
  0x0000000000000000000000000000000000000001

[PASS] testLogBytes() ([GAS])
Logs:
  0
  1
  0x61626364

[PASS] testLogBytes32() ([GAS])
Logs:
  0
  1
  0x6162636400000000000000000000000000000000000000000000000000000000

[PASS] testLogInt() ([GAS])
Logs:
  0
  1
  -31337

[PASS] testLogNamedAddress() ([GAS])
Logs:
  0
  1
  address: 0x0000000000000000000000000000000000000001

[PASS] testLogNamedBytes() ([GAS])
Logs:
  0
  1
  abcd: 0x61626364

[PASS] testLogNamedBytes32() ([GAS])
Logs:
  0
  1
  abcd: 0x6162636400000000000000000000000000000000000000000000000000000000

[PASS] testLogNamedDecimalInt() ([GAS])
Logs:
  0
  1
  amount: -0.000000000000031337

[PASS] testLogNamedDecimalUint() ([GAS])
Logs:
  0
  1
  amount: 1.000000000000000000

[PASS] testLogNamedInt() ([GAS])
Logs:
  0
  1
  amount: -31337

[PASS] testLogNamedString() ([GAS])
Logs:
  0
  1
  key: val

[PASS] testLogNamedUint() ([GAS])
Logs:
  0
  1
  amount: 1000000000000000000

[PASS] testLogString() ([GAS])
Logs:
  0
  1
  here

[PASS] testLogs() ([GAS])
Logs:
  0
  1
  0x61626364

[PASS] testRevertIfWithRequire() ([GAS])
Logs:
  0
  1
  5

[PASS] testRevertIfWithRevert() ([GAS])
Logs:
  0
  1
  4
  100

Suite result: ok. 19 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 19 tests passed, 0 failed, 0 skipped (19 total tests)

"#]]);
});

forgetest_init!(hardhat_logs, |prj, cmd| {
    prj.add_test(
        "HardhatLogs.t.sol",
        r#"
import "forge-std/console.sol";

contract HardhatLogsTest {
    constructor() {
        console.log("constructor");
    }

    string testStr;
    int256 testInt;
    uint256 testUint;
    bool testBool;
    address testAddr;
    bytes testBytes;

    function setUp() public {
        testStr = "test";
        testInt = -31337;
        testUint = 1;
        testBool = false;
        testAddr = 0x0000000000000000000000000000000000000001;
        testBytes = "a";
    }

    function testInts() public view {
        console.log(uint256(0));
        console.log(uint256(1));
        console.log(uint256(2));
        console.log(uint256(3));
    }

    function testStrings() public view {
        console.log("testStrings");
    }

    function testMisc() public view {
        console.log("testMisc", address(1));
        console.log("testMisc", uint256(42));
    }

    function testConsoleLog() public view {
        console.log(testStr);
    }

    function testLogInt() public view {
        console.logInt(testInt);
    }

    function testLogUint() public view {
        console.logUint(testUint);
    }

    function testLogString() public view {
        console.logString(testStr);
    }

    function testLogBool() public view {
        console.logBool(testBool);
    }

    function testLogAddress() public view {
        console.logAddress(testAddr);
    }

    function testLogBytes() public view {
        console.logBytes(testBytes);
    }

    function testLogBytes1() public view {
        console.logBytes1(bytes1(testBytes));
    }

    function testLogBytes2() public view {
        console.logBytes2(bytes2(testBytes));
    }

    function testLogBytes3() public view {
        console.logBytes3(bytes3(testBytes));
    }

    function testLogBytes4() public view {
        console.logBytes4(bytes4(testBytes));
    }

    function testLogBytes5() public view {
        console.logBytes5(bytes5(testBytes));
    }

    function testLogBytes6() public view {
        console.logBytes6(bytes6(testBytes));
    }

    function testLogBytes7() public view {
        console.logBytes7(bytes7(testBytes));
    }

    function testLogBytes8() public view {
        console.logBytes8(bytes8(testBytes));
    }

    function testLogBytes9() public view {
        console.logBytes9(bytes9(testBytes));
    }

    function testLogBytes10() public view {
        console.logBytes10(bytes10(testBytes));
    }

    function testLogBytes11() public view {
        console.logBytes11(bytes11(testBytes));
    }

    function testLogBytes12() public view {
        console.logBytes12(bytes12(testBytes));
    }

    function testLogBytes13() public view {
        console.logBytes13(bytes13(testBytes));
    }

    function testLogBytes14() public view {
        console.logBytes14(bytes14(testBytes));
    }

    function testLogBytes15() public view {
        console.logBytes15(bytes15(testBytes));
    }

    function testLogBytes16() public view {
        console.logBytes16(bytes16(testBytes));
    }

    function testLogBytes17() public view {
        console.logBytes17(bytes17(testBytes));
    }

    function testLogBytes18() public view {
        console.logBytes18(bytes18(testBytes));
    }

    function testLogBytes19() public view {
        console.logBytes19(bytes19(testBytes));
    }

    function testLogBytes20() public view {
        console.logBytes20(bytes20(testBytes));
    }

    function testLogBytes21() public view {
        console.logBytes21(bytes21(testBytes));
    }

    function testLogBytes22() public view {
        console.logBytes22(bytes22(testBytes));
    }

    function testLogBytes23() public view {
        console.logBytes23(bytes23(testBytes));
    }

    function testLogBytes24() public view {
        console.logBytes24(bytes24(testBytes));
    }

    function testLogBytes25() public view {
        console.logBytes25(bytes25(testBytes));
    }

    function testLogBytes26() public view {
        console.logBytes26(bytes26(testBytes));
    }

    function testLogBytes27() public view {
        console.logBytes27(bytes27(testBytes));
    }

    function testLogBytes28() public view {
        console.logBytes28(bytes28(testBytes));
    }

    function testLogBytes29() public view {
        console.logBytes29(bytes29(testBytes));
    }

    function testLogBytes30() public view {
        console.logBytes30(bytes30(testBytes));
    }

    function testLogBytes31() public view {
        console.logBytes31(bytes31(testBytes));
    }

    function testLogBytes32() public view {
        console.logBytes32(bytes32(testBytes));
    }

    function testConsoleLogUint() public view {
        console.log(testUint);
    }

    function testConsoleLogString() public view {
        console.log(testStr);
    }

    function testConsoleLogBool() public view {
        console.log(testBool);
    }

    function testConsoleLogAddress() public view {
        console.log(testAddr);
    }

    function testConsoleLogFormatString() public view {
        console.log("formatted log str=%s", testStr);
    }

    function testConsoleLogFormatUint() public view {
        console.log("formatted log uint=%s", testUint);
    }

    function testConsoleLogFormatAddress() public view {
        console.log("formatted log addr=%s", testAddr);
    }

    function testConsoleLogFormatMulti() public view {
        console.log("formatted log str=%s uint=%d", testStr, testUint);
    }

    function testConsoleLogFormatEscape() public view {
        console.log("formatted log %% %s", testStr);
    }

    function testConsoleLogFormatSpill() public view {
        console.log("formatted log %s", testStr, testUint);
    }
}
"#,
    );

    cmd.args(["test", "-vv"]).assert_success().stdout_eq(str![[r#"
...
Ran 52 tests for test/HardhatLogs.t.sol:HardhatLogsTest
[PASS] testConsoleLog() ([GAS])
Logs:
  constructor
  test

[PASS] testConsoleLogAddress() ([GAS])
Logs:
  constructor
  0x0000000000000000000000000000000000000001

[PASS] testConsoleLogBool() ([GAS])
Logs:
  constructor
  false

[PASS] testConsoleLogFormatAddress() ([GAS])
Logs:
  constructor
  formatted log addr=0x0000000000000000000000000000000000000001

[PASS] testConsoleLogFormatEscape() ([GAS])
Logs:
  constructor
  formatted log % test

[PASS] testConsoleLogFormatMulti() ([GAS])
Logs:
  constructor
  formatted log str=test uint=1

[PASS] testConsoleLogFormatSpill() ([GAS])
Logs:
  constructor
  formatted log test 1

[PASS] testConsoleLogFormatString() ([GAS])
Logs:
  constructor
  formatted log str=test

[PASS] testConsoleLogFormatUint() ([GAS])
Logs:
  constructor
  formatted log uint=1

[PASS] testConsoleLogString() ([GAS])
Logs:
  constructor
  test

[PASS] testConsoleLogUint() ([GAS])
Logs:
  constructor
  1

[PASS] testInts() ([GAS])
Logs:
  constructor
  0
  1
  2
  3

[PASS] testLogAddress() ([GAS])
Logs:
  constructor
  0x0000000000000000000000000000000000000001

[PASS] testLogBool() ([GAS])
Logs:
  constructor
  false

[PASS] testLogBytes() ([GAS])
Logs:
  constructor
  0x61

[PASS] testLogBytes1() ([GAS])
Logs:
  constructor
  0x61

[PASS] testLogBytes10() ([GAS])
Logs:
  constructor
  0x61000000000000000000

[PASS] testLogBytes11() ([GAS])
Logs:
  constructor
  0x6100000000000000000000

[PASS] testLogBytes12() ([GAS])
Logs:
  constructor
  0x610000000000000000000000

[PASS] testLogBytes13() ([GAS])
Logs:
  constructor
  0x61000000000000000000000000

[PASS] testLogBytes14() ([GAS])
Logs:
  constructor
  0x6100000000000000000000000000

[PASS] testLogBytes15() ([GAS])
Logs:
  constructor
  0x610000000000000000000000000000

[PASS] testLogBytes16() ([GAS])
Logs:
  constructor
  0x61000000000000000000000000000000

[PASS] testLogBytes17() ([GAS])
Logs:
  constructor
  0x6100000000000000000000000000000000

[PASS] testLogBytes18() ([GAS])
Logs:
  constructor
  0x610000000000000000000000000000000000

[PASS] testLogBytes19() ([GAS])
Logs:
  constructor
  0x61000000000000000000000000000000000000

[PASS] testLogBytes2() ([GAS])
Logs:
  constructor
  0x6100

[PASS] testLogBytes20() ([GAS])
Logs:
  constructor
  0x6100000000000000000000000000000000000000

[PASS] testLogBytes21() ([GAS])
Logs:
  constructor
  0x610000000000000000000000000000000000000000

[PASS] testLogBytes22() ([GAS])
Logs:
  constructor
  0x61000000000000000000000000000000000000000000

[PASS] testLogBytes23() ([GAS])
Logs:
  constructor
  0x6100000000000000000000000000000000000000000000

[PASS] testLogBytes24() ([GAS])
Logs:
  constructor
  0x610000000000000000000000000000000000000000000000

[PASS] testLogBytes25() ([GAS])
Logs:
  constructor
  0x61000000000000000000000000000000000000000000000000

[PASS] testLogBytes26() ([GAS])
Logs:
  constructor
  0x6100000000000000000000000000000000000000000000000000

[PASS] testLogBytes27() ([GAS])
Logs:
  constructor
  0x610000000000000000000000000000000000000000000000000000

[PASS] testLogBytes28() ([GAS])
Logs:
  constructor
  0x61000000000000000000000000000000000000000000000000000000

[PASS] testLogBytes29() ([GAS])
Logs:
  constructor
  0x6100000000000000000000000000000000000000000000000000000000

[PASS] testLogBytes3() ([GAS])
Logs:
  constructor
  0x610000

[PASS] testLogBytes30() ([GAS])
Logs:
  constructor
  0x610000000000000000000000000000000000000000000000000000000000

[PASS] testLogBytes31() ([GAS])
Logs:
  constructor
  0x61000000000000000000000000000000000000000000000000000000000000

[PASS] testLogBytes32() ([GAS])
Logs:
  constructor
  0x6100000000000000000000000000000000000000000000000000000000000000

[PASS] testLogBytes4() ([GAS])
Logs:
  constructor
  0x61000000

[PASS] testLogBytes5() ([GAS])
Logs:
  constructor
  0x6100000000

[PASS] testLogBytes6() ([GAS])
Logs:
  constructor
  0x610000000000

[PASS] testLogBytes7() ([GAS])
Logs:
  constructor
  0x61000000000000

[PASS] testLogBytes8() ([GAS])
Logs:
  constructor
  0x6100000000000000

[PASS] testLogBytes9() ([GAS])
Logs:
  constructor
  0x610000000000000000

[PASS] testLogInt() ([GAS])
Logs:
  constructor
  -31337

[PASS] testLogString() ([GAS])
Logs:
  constructor
  test

[PASS] testLogUint() ([GAS])
Logs:
  constructor
  1

[PASS] testMisc() ([GAS])
Logs:
  constructor
  testMisc 0x0000000000000000000000000000000000000001
  testMisc 42

[PASS] testStrings() ([GAS])
Logs:
  constructor
  testStrings

Suite result: ok. 52 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 52 tests passed, 0 failed, 0 skipped (52 total tests)

"#]]);
});
