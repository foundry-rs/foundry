// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

contract RevertingTest {
    function testFailRevert() public pure {
        require(false, "should revert here");
    }
}
