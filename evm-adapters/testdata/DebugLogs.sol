pragma solidity ^0.8.0;

import "./DsTest.sol";

contract DebugLogs is DSTest {
    function test_log() public {
        emit log("Hi");
        emit logs(hex"1234");
        emit log_address(0x1111111111111111111111111111111111111111);
        emit log_bytes32(keccak256(abi.encodePacked("foo")));
        emit log_int(123);
        emit log_uint(1234);
        emit log_bytes(hex"4567");
        emit log_string("lol");
        emit log_named_address("addr", 0x2222222222222222222222222222222222222222);
        emit log_named_bytes32("key", keccak256(abi.encodePacked("foo")));
        emit log_named_decimal_int("key", 123, 18);
        emit log_named_decimal_uint("key", 1234, 18);
        emit log_named_int("key", 123);
        emit log_named_uint("key", 1234);
        emit log_named_bytes("key", hex"4567");
        emit log_named_string("key", "lol");
    }
}
