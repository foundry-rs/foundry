// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

contract RevertingTest {
    function testFailRevert() public pure {
        require(false, "should revert here");
    }
}
