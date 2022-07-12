// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "forge-std/Test.sol";

contract FailingSetupTest is Test {
    event Test(uint256 n);

    function setUp() public {
        emit Test(42);
        require(false, "setup failed predictably");
    }

    function testFailShouldBeMarkedAsFailedBecauseOfSetup() public {
        emit log("setup did not fail");
    }
}
