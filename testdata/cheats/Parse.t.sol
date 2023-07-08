// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract ParseTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testParseBytes() public {
        bytes memory testBytes = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";

        string memory stringBytes = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        assertEq(testBytes, vm.parseBytes(stringBytes));

        stringBytes = "7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        assertEq(testBytes, vm.parseBytes(stringBytes));
    }

    function testParseBytesFuzzed(bytes memory testBytes) public {
        string memory stringBytes = vm.toString(testBytes);
        assertEq(testBytes, vm.parseBytes(stringBytes));
    }

    function testParseAddress() public {
        address testAddress = 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D;

        string memory stringAddress = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        assertEq(testAddress, vm.parseAddress(stringAddress));

        stringAddress = "7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        assertEq(testAddress, vm.parseAddress(stringAddress));
    }

    function testParseAddressFuzzed(address testAddress) public {
        string memory stringAddress = vm.toString(testAddress);
        assertEq(testAddress, vm.parseAddress(stringAddress));
    }

    function testParseUint() public {
        uint256 testUint256 = 420;

        string memory stringUint256 = "420";
        assertEq(testUint256, vm.parseUint(stringUint256));
    }

    function testParseUintStringBytes() public {
        uint256 testUint256 = 420;

        string memory stringUint256 = "0x1A4";
        assertEq(testUint256, vm.parseUint(stringUint256));
    }

    function testParseUintBytes() public {
        uint256 testUint256 = 420;
        bytes memory testBytes = hex"01A4";
        string memory stringUint256 = vm.toString(testBytes);
        assertEq(testUint256, vm.parseUint(stringUint256));
    }

    function testParseUintFuzzed(uint256 testUint256) public {
        string memory stringUint256 = vm.toString(testUint256);
        assertEq(testUint256, vm.parseUint(stringUint256));
    }

    function testParseInt() public {
        int256 testInt256 = 420;

        string memory stringInt256 = "420";
        assertEq(testInt256, vm.parseInt(stringInt256));
    }

    function testParseIntFuzzed(int256 testInt256) public {
        string memory stringInt256 = vm.toString(testInt256);
        assertEq(testInt256, vm.parseInt(stringInt256));
    }

    function testParseBytes32() public {
        bytes32 testBytes = "test";

        string memory stringBytes = "7465737400000000000000000000000000000000000000000000000000000000";
        assertEq(testBytes, vm.parseBytes32(stringBytes));

        stringBytes = "0x7465737400000000000000000000000000000000000000000000000000000000";
        assertEq(testBytes, vm.parseBytes32(stringBytes));
    }

    function testParseBytes32Fuzzed(bytes32 testBytes) public {
        string memory stringBytes = vm.toString(testBytes);
        assertEq(testBytes, vm.parseBytes32(stringBytes));
    }

    function testParseBool() public {
        bool testBool = true;

        string memory stringBool = "true";
        assertEq(testBool, vm.parseBool(stringBool));
    }

    function testParseBoolFuzzed(bool testBool) public {
        string memory stringBool = vm.toString(testBool);
        assertEq(testBool, vm.parseBool(stringBool));
    }
}
