// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract ToStringTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testAddressToString() public {
        address testAddress = 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D;
        string memory stringAddress = vm.toString(testAddress);
        assertEq("0x7109709ECfa91a80626fF3989D68f67F5b1DD12D", stringAddress);
    }

    function testBytes32ToString() public {
        bytes32 testBytes = "test";
        string memory stringBytes = vm.toString(testBytes);
        assertEq("0x7465737400000000000000000000000000000000000000000000000000000000", stringBytes);
    }

    function testBoolToString() public {
        bool testBool = true;
        string memory stringBool = vm.toString(testBool);
        assertEq("true", stringBool);
    }

    function testBytesToString() public {
        bytes memory testBytes = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        string memory stringBytes = vm.toString(testBytes);
        assertEq("0x7109709ecfa91a80626ff3989d68f67f5b1dd12d", stringBytes);
    }

    function testUintToString() public {
        uint256 testUint = 420;
        string memory stringUint = vm.toString(testUint);
        assertEq("420", stringUint);
    }

    function testIntToString() public {
        int256 testInt = 420;
        string memory stringInt = vm.toString(testInt);
        assertEq("420", stringInt);
    }
}
