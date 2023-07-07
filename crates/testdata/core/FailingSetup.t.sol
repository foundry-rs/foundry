// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract FailingSetupTest is DSTest {
    event Test(uint256 n);

    function setUp() public {
        emit Test(42);
        require(false, "setup failed predictably");
    }

    function testFailShouldBeMarkedAsFailedBecauseOfSetup() public {
        emit log("setup did not fail");
    }
}
