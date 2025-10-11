// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract NoAssertionsRevertTest is Test {
    function testMultipleAssertFailures() public {
        vm.assertEq(uint256(1), uint256(2));
        vm.assertLt(uint256(5), uint256(4));
    }
}

/// forge-config: default.legacy_assertions = true
contract LegacyAssertionsTest {
    bool public failed;

    function testFlagNotSetSuccess() public {}

    function testFlagSetFailure() public {
        failed = true;
    }
}
