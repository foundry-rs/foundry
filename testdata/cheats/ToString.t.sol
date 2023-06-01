// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract ToStringTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testAddressToString() public {
        address testAddress = 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D;
        string memory stringAddress = cheats.toString(testAddress);
        assertEq("0x7109709ECfa91a80626fF3989D68f67F5b1DD12D", stringAddress);
    }

    function testBytes32ToString() public {
        bytes32 testBytes = "test";
        string memory stringBytes = cheats.toString(testBytes);
        assertEq("0x7465737400000000000000000000000000000000000000000000000000000000", stringBytes);
    }

    function testBoolToString() public {
        bool testBool = true;
        string memory stringBool = cheats.toString(testBool);
        assertEq("true", stringBool);
    }

    function testBytesToString() public {
        bytes memory testBytes = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        string memory stringBytes = cheats.toString(testBytes);
        assertEq("0x7109709ecfa91a80626ff3989d68f67f5b1dd12d", stringBytes);
    }

    function testUintToString() public {
        uint256 testUint = 420;
        string memory stringUint = cheats.toString(testUint);
        assertEq("420", stringUint);
    }

    function testIntToString() public {
        int256 testInt = 420;
        string memory stringInt = cheats.toString(testInt);
        assertEq("420", stringInt);
    }
}
