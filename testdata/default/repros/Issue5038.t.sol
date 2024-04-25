// SPDX-License-Identifier: MIT
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

struct Value {
    uint256 value;
}

// https://github.com/foundry-rs/foundry/issues/5038
contract Issue5038Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testParseMaxUint64() public {
        string memory json = '{"value": 18446744073709551615}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 18446744073709551615);
    }

    function testParse18446744073709551616000() public {
        string memory json = '{"value": 18446744073709551616000}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 18446744073709551616000);
    }

    function testParse1e19() public {
        string memory json = '{"value": 10000000000000000000}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 10000000000000000000);
    }

    function test_parse_1e20() public {
        string memory json = '{"value": 100000000000000000000}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 100000000000000000000);
    }

    function testParse1e21() public {
        string memory json = '{"value": 1000000000000000000000}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 1000000000000000000000);
    }

    function testParse1e42() public {
        string memory json = '{"value": 1000000000000000000000000000000000000000000}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 1000000000000000000000000000000000000000000);
    }

    function testParse1e42plusOne() public {
        string memory json = '{"value": 1000000000000000000000000000000000000000001}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 1000000000000000000000000000000000000000001);
    }

    function testParse1e43plus5() public {
        string memory json = '{"value": 10000000000000000000000000000000000000000005}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 10000000000000000000000000000000000000000005);
    }

    function testParse1e76() public {
        string memory json = '{"value": 10000000000000000000000000000000000000000000000000000000000000000000000000000}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 10000000000000000000000000000000000000000000000000000000000000000000000000000);
    }

    function testParse1e76Plus10() public {
        string memory json = '{"value": 10000000000000000000000000000000000000000000000000000000000000000000000000010}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 10000000000000000000000000000000000000000000000000000000000000000000000000010);
    }

    function testParseMinUint256() public {
        string memory json = '{"value": -57896044618658097711785492504343953926634992332820282019728792003956564819968}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));

        assertEq(int256(data.value), -57896044618658097711785492504343953926634992332820282019728792003956564819968);
    }

    function testParseMaxUint256() public {
        string memory json = '{"value": 115792089237316195423570985008687907853269984665640564039457584007913129639935}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 115792089237316195423570985008687907853269984665640564039457584007913129639935);
    }
}
