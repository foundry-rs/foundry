// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../logs/console.sol";
import "./Cheats.sol";

contract ParseTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testParseBytes() public {
        string memory stringBytes = "7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        bytes memory testBytes = hex"7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        assertEq0(testBytes, cheats.parseBytes(stringBytes));
    }

    function testParseAddress() public {
        string memory stringAddress = "0x7109709ECfa91a80626fF3989D68f67F5b1DD12D";
        address testAddress = 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D;
        assertEq(testAddress, cheats.parseAddress(stringAddress));
    }

    function testParseUint256() public {
        string memory stringUint256 = "420";
        uint256 testUint256 = 420;
        assertEq(testUint256, cheats.parseUint256(stringUint256));
    }

    function testParseInt256() public {
        string memory stringInt256 = "420";
        int256 testInt256 = 420;
        assertEq(testInt256, cheats.parseInt256(stringInt256));
    }

    function testParseBytes32() public {
        string memory stringBytes = "7465737400000000000000000000000000000000000000000000000000000000";
        bytes32 testBytes = "test";
        assertEq(testBytes, cheats.parseBytes32(stringBytes));
    }

    function testParseBool() public {
        string memory stringBool = "true";
        bool testBool = true;
        assertEq(testBool ? 1 : 0, cheats.parseBool(stringBool) ? 1 : 0);
    }
}
