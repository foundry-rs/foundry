// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

library InternalLib {
    // This function name starts with "invariant" which causes the issue
    function invariantProtocol() public pure returns (uint256 code) {
        return 1;
    }
}

contract InternalLibTest {
    function testInternalLibInvariantProtocol() public {
        assertEq(InternalLib.invariantProtocol(), 1);
    }

    function assertEq(uint256 a, uint256 b) internal pure {
        require(a == b, "Not equal");
    }
} 
