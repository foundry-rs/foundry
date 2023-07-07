pragma solidity 0.8.18;

import "ds-test/test.sol";

contract DebugLogsTest is DSTest {
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

    function testFailWithRevert() public {
        Fails fails = new Fails();
        emit log_uint(4);
        fails.failure();
    }

    function testFailWithRequire() public {
        emit log_uint(5);
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

contract Fails is DSTest {
    function failure() public {
        emit log_uint(100);
        revert();
    }
}
