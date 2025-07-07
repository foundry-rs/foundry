// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import {Test} from "forge-std/Test.sol";

library InternalLib {
    // This function was previously incorrectly identified as an invariant test
    function invariantProtocol() public pure returns (uint256 code) {
        return 1;
    }
    
    // This internal function would have worked because it's not public
    function _invariantProtocol() internal pure returns (uint256) {
        return 2;
    }
}

contract InternalLibTest is Test {
    function testInternalLibInvariantProtocol() public {
        assertEq(InternalLib.invariantProtocol(), 1);
    }
} 
