// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "forge-std/Test.sol";

contract MultipleSetup is Test {
    function setUp() public { }

    function setup() public { }

    function testFailShouldBeMarkedAsFailedBecauseOfSetup() public {
      assert(true);
    }
}
