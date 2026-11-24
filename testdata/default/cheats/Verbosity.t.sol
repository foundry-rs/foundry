// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract VerbosityTest is Test {
    function testVerbosityReturnsCurrentLevel() public view {
        (bool success, bytes memory data) = address(vm).staticcall(abi.encodeCall(vm.verbosity, ()));
        assertTrue(success);
        assertEq(data.length, 32);
        assertEq(abi.decode(data, (uint8)), vm.verbosity());
    }
}
